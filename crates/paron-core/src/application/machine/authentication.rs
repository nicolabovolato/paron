use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use paron_tunnel_protocol::{AgentId, BrokerAddress, Fingerprint};
use secrecy::SecretString;

use super::commands::issue_enrollment_token::IssueEnrollmentTokenCommand;
use super::errors::{EnrollError, IssueEnrollmentTokenError, UnenrollError};
use super::service::MachineService;
use super::use_cases::MachineEnrollment;
use crate::application::clock::Clock;
use crate::application::identity::authorization::actor::Actor;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::application::tokens::Tokens;
use crate::domain::machine::tunnel::token::{ServerEndpoint, Token};
use crate::domain::machine::{Connectivity, Machine, tunnel};
use crate::domain::user::Permission;

pub struct EnrollmentToken {
    pub token: Token,
    pub expires_at: jiff::Timestamp,
}

#[derive(Debug, Clone, Copy)]
pub struct EnrollmentPolicy {
    pub invitation_lifetime: Duration,
}

fn tunnel_enrollment(machine: &Machine) -> Option<&tunnel::Enrollment> {
    match &machine.connectivity {
        Connectivity::Tunneled(enrollment) => Some(enrollment),
        Connectivity::Direct { .. } => None,
    }
}

pub struct MachineAuthenticationService<Tx> {
    service: Arc<MachineService<Tx>>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    tokens: Arc<dyn Tokens>,
    clock: Arc<dyn Clock>,
    policy: EnrollmentPolicy,
    broker_fingerprint: Fingerprint,
    broker_address: BrokerAddress,
}

impl<Tx: Send + Sync + 'static> MachineAuthenticationService<Tx> {
    pub fn new(
        service: Arc<MachineService<Tx>>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        tokens: Arc<dyn Tokens>,
        clock: Arc<dyn Clock>,
        policy: EnrollmentPolicy,
        broker_fingerprint: Fingerprint,
        broker_address: BrokerAddress,
    ) -> Self {
        Self {
            service,
            unit_of_work,
            tokens,
            clock,
            policy,
            broker_fingerprint,
            broker_address,
        }
    }

    pub async fn issue_enrollment_token(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        cmd: IssueEnrollmentTokenCommand,
    ) -> Result<EnrollmentToken, IssueEnrollmentTokenError> {
        actor.authorize(Permission::MachineEnroll)?;

        let expires_at = self.clock.now() + self.policy.invitation_lifetime;

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let machine = self.service.get(ctx, actor, cmd.machine_id).await?;

                match tunnel_enrollment(&machine).ok_or(IssueEnrollmentTokenError::NotEnrollable)? {
                    tunnel::Enrollment::NotEnrolled | tunnel::Enrollment::Invited { .. } => {}
                    tunnel::Enrollment::Enrolled { .. } => {
                        return Err(IssueEnrollmentTokenError::AlreadyEnrolled);
                    }
                }

                let (secret, mac) = self.tokens.issue();

                self.service
                    .set_enrollment(
                        ctx,
                        actor,
                        machine.id,
                        tunnel::Enrollment::Invited { mac, expires_at },
                    )
                    .await?;

                let token = Token {
                    server_endpoint: ServerEndpoint {
                        address: self.broker_address.clone(),
                        machine: AgentId::new(machine.id),
                        fingerprint: self.broker_fingerprint.clone(),
                    },
                    secret,
                };

                Ok(EnrollmentToken { token, expires_at })
            })
            .await
    }

    pub async fn enroll(
        &self,
        ctx: &mut TxContext<Tx>,
        machine_id: uuid::Uuid,
        token: &SecretString,
        client_public_key: tunnel::PublicKey,
    ) -> Result<(), EnrollError> {
        let now = self.clock.now();

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let machine = self.service.get(ctx, &Actor::System, machine_id).await?;

                let (outstanding, expires_at) =
                    match tunnel_enrollment(&machine).ok_or(EnrollError::NotEnrollable)? {
                        tunnel::Enrollment::NotEnrolled => return Err(EnrollError::NotInvited),
                        tunnel::Enrollment::Invited { mac, expires_at } => (mac, expires_at),
                        tunnel::Enrollment::Enrolled { .. } => {
                            return Err(EnrollError::AlreadyEnrolled);
                        }
                    };

                if *expires_at <= now {
                    return Err(EnrollError::Expired);
                }

                if self.tokens.mac(token) != *outstanding {
                    return Err(EnrollError::InvalidToken);
                }

                self.service
                    .set_enrollment(
                        ctx,
                        &Actor::System,
                        machine.id,
                        tunnel::Enrollment::Enrolled { client_public_key },
                    )
                    .await?;

                Ok(())
            })
            .await
    }

    pub async fn unenroll(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        machine_id: uuid::Uuid,
    ) -> Result<(), UnenrollError> {
        actor.authorize(Permission::MachineEnroll)?;

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let machine = self.service.get(ctx, actor, machine_id).await?;

                tunnel_enrollment(&machine).ok_or(UnenrollError::NotEnrollable)?;

                self.service
                    .set_enrollment(ctx, actor, machine_id, tunnel::Enrollment::NotEnrolled)
                    .await?;

                Ok(())
            })
            .await
    }

    pub async fn authorize(
        &self,
        machine_id: uuid::Uuid,
        client_public_key: &tunnel::PublicKey,
    ) -> bool {
        let Ok(machine) = self
            .service
            .get(&mut TxContext::none(), &Actor::System, machine_id)
            .await
        else {
            return false;
        };

        matches!(
            &machine.connectivity,
            Connectivity::Tunneled(tunnel::Enrollment::Enrolled {
                client_public_key: stored,
            }) if stored == client_public_key
        )
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> MachineEnrollment for MachineAuthenticationService<Tx> {
    async fn issue_enrollment_token(
        &self,
        actor: &Actor,
        cmd: IssueEnrollmentTokenCommand,
    ) -> Result<EnrollmentToken, IssueEnrollmentTokenError> {
        Self::issue_enrollment_token(self, &mut TxContext::none(), actor, cmd).await
    }

    async fn unenroll(&self, actor: &Actor, machine_id: uuid::Uuid) -> Result<(), UnenrollError> {
        Self::unenroll(self, &mut TxContext::none(), actor, machine_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::collections::BTreeSet;

    use futures_util::FutureExt as _;
    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::application::audit::AuditService;
    use crate::application::events::{Event, EventStream as _, SubscriptionScope};
    use crate::application::machine::event::EventKind;
    use crate::application::machine::event::{AuditKind, MachineSnapshot};
    use crate::application::machine::ports::{MockRepository, MockSshKeypairs};
    use crate::application::repository::{MockUnitOfWork, TxGuarantee};
    use crate::application::tokens::MockTokens;
    use crate::domain::entity::EntityKind;
    use crate::domain::token_mac::TokenMac;
    use crate::infrastructure::in_memory::events::InMemoryEventStream;
    use crate::testing::clock::clock_at;
    use crate::testing::events::RecordedAudit;
    use crate::testing::mocks::{
        machine_repository_returning, machine_repository_without_machines,
    };
    use crate::testing::{enrollment, events, identity, keys, machine};

    const ISSUED_TOKEN: &str = "a-fresh-token";
    const ISSUED_MAC: [u8; 32] = [0x11; 32];
    const WRONG_MAC: [u8; 32] = [0x22; 32];

    struct ServiceBuilder {
        repository: MockRepository<()>,
        events: Arc<InMemoryEventStream>,
        transactions: MockUnitOfWork,
        tokens: MockTokens,
        now: jiff::Timestamp,
        audit: Arc<RecordedAudit<()>>,
    }

    impl ServiceBuilder {
        fn new() -> Self {
            Self {
                audit: events::recorded_audit(),
                repository: MockRepository::new(),
                events: Arc::new(InMemoryEventStream::default()),
                transactions: MockUnitOfWork::new(),
                tokens: MockTokens::new(),
                now: enrollment::issued_at(),
            }
        }

        fn repository(mut self, repository: MockRepository<()>) -> Self {
            self.repository = repository;
            self
        }

        fn events(mut self, events: Arc<InMemoryEventStream>) -> Self {
            self.events = events;
            self
        }

        fn transactions(mut self, transactions: MockUnitOfWork) -> Self {
            self.transactions = transactions;
            self
        }

        fn tokens(mut self, tokens: MockTokens) -> Self {
            self.tokens = tokens;
            self
        }

        fn now(mut self, now: jiff::Timestamp) -> Self {
            self.now = now;
            self
        }

        fn audit(mut self, audit: Arc<RecordedAudit<()>>) -> Self {
            self.audit = audit;
            self
        }

        fn build(self) -> MachineAuthenticationService<()> {
            let transactions = Arc::new(self.transactions);
            let events = events::event_service_on(self.events, clock_at(self.now));

            MachineAuthenticationService::new(
                Arc::new(MachineService::new(
                    Arc::new(self.repository),
                    Arc::clone(&transactions) as Arc<_>,
                    Arc::new(MockSshKeypairs::new()),
                    Arc::new(AuditService::new(self.audit, clock_at(self.now))),
                    events,
                )),
                transactions,
                Arc::new(self.tokens),
                clock_at(self.now),
                enrollment::enrollment_policy(),
                enrollment::broker_fingerprint(),
                enrollment::broker_address(),
            )
        }
    }

    fn transactions_expecting_one_begin() -> MockUnitOfWork {
        let mut transactions = MockUnitOfWork::new();
        transactions
            .expect_begin()
            .once()
            .withf(|guarantee| *guarantee == TxGuarantee::ReadModifyWrite)
            .returning(|guarantee| Ok(TxContext::new((), guarantee)));
        transactions.expect_savepoint().returning(|_| Ok(()));
        transactions.expect_release().returning(|_| Ok(()));
        transactions.expect_rollback_to().returning(|_| Ok(()));

        transactions
    }

    fn transactions_expecting_one_commit() -> MockUnitOfWork {
        let mut transactions = transactions_expecting_one_begin();
        transactions.expect_commit().once().returning(|_| Ok(()));

        transactions
    }

    fn transactions_expecting_no_commit() -> MockUnitOfWork {
        let mut transactions = transactions_expecting_one_begin();
        transactions.expect_commit().never();

        transactions
    }

    fn tokens_that_issue() -> MockTokens {
        let mut tokens = MockTokens::new();
        tokens.expect_issue().once().returning(|| {
            (
                SecretString::from(ISSUED_TOKEN),
                TokenMac::from(ISSUED_MAC.to_vec()),
            )
        });

        tokens
    }

    fn tokens_that_mac_to(mac: [u8; 32]) -> MockTokens {
        let mut tokens = MockTokens::new();
        tokens
            .expect_mac()
            .once()
            .returning(move |_| TokenMac::from(mac.to_vec()));

        tokens
    }

    fn is_invited_with_issued_mac(machine: &Machine) -> bool {
        matches!(
            &machine.connectivity,
            Connectivity::Tunneled(tunnel::Enrollment::Invited { mac, expires_at })
                if **mac == ISSUED_MAC && *expires_at == enrollment::expires_at()
        )
    }

    #[tokio::test]
    async fn issue_enrollment_token_without_machine_enroll_returns_denied() {
        let service = ServiceBuilder::new().build();

        let issued = service
            .issue_enrollment_token(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                IssueEnrollmentTokenCommand {
                    machine_id: uuid::Uuid::now_v7(),
                },
            )
            .await;

        assert_matches!(issued.err(), Some(IssueEnrollmentTokenError::Denied(_)));
    }

    #[tokio::test]
    async fn issue_enrollment_token_stores_invitation() {
        let mut repository = machine_repository_returning(machine::unenrolled_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| is_invited_with_issued_mac(machine))
            .returning(|_, _| Ok(machine::invited_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .tokens(tokens_that_issue())
            .transactions(transactions_expecting_one_commit())
            .build();

        service
            .issue_enrollment_token(
                &mut TxContext::none(),
                &Actor::System,
                IssueEnrollmentTokenCommand {
                    machine_id: uuid::Uuid::now_v7(),
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn issue_enrollment_token_returns_issued_secret_broker_address_and_expiry() {
        let mut repository = machine_repository_returning(machine::unenrolled_machine);
        repository
            .expect_update()
            .returning(|_, _| Ok(machine::invited_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .tokens(tokens_that_issue())
            .transactions(transactions_expecting_one_commit())
            .build();

        let issued = service
            .issue_enrollment_token(
                &mut TxContext::none(),
                &Actor::System,
                IssueEnrollmentTokenCommand {
                    machine_id: uuid::Uuid::now_v7(),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            (
                issued.token.secret.expose_secret(),
                issued.token.server_endpoint.address.to_string(),
                issued.expires_at
            ),
            (
                ISSUED_TOKEN,
                enrollment::broker_address().to_string(),
                enrollment::expires_at()
            )
        );
    }

    #[tokio::test]
    async fn issue_enrollment_token_for_invited_machine_replaces_the_outstanding_one() {
        let mut repository = machine_repository_returning(machine::invited_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| is_invited_with_issued_mac(machine))
            .returning(|_, _| Ok(machine::invited_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .tokens(tokens_that_issue())
            .transactions(transactions_expecting_one_commit())
            .build();

        service
            .issue_enrollment_token(
                &mut TxContext::none(),
                &Actor::System,
                IssueEnrollmentTokenCommand {
                    machine_id: uuid::Uuid::now_v7(),
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn issue_enrollment_token_for_enrolled_machine_returns_already_enrolled() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::enrolled_machine))
            .transactions(transactions_expecting_no_commit())
            .build();

        let issued = service
            .issue_enrollment_token(
                &mut TxContext::none(),
                &Actor::System,
                IssueEnrollmentTokenCommand {
                    machine_id: uuid::Uuid::now_v7(),
                },
            )
            .await;

        assert_matches!(
            issued.err(),
            Some(IssueEnrollmentTokenError::AlreadyEnrolled)
        );
    }

    #[tokio::test]
    async fn issue_enrollment_token_for_direct_machine_returns_not_enrollable() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::direct_machine))
            .transactions(transactions_expecting_no_commit())
            .build();

        let issued = service
            .issue_enrollment_token(
                &mut TxContext::none(),
                &Actor::System,
                IssueEnrollmentTokenCommand {
                    machine_id: uuid::Uuid::now_v7(),
                },
            )
            .await;

        assert_matches!(issued.err(), Some(IssueEnrollmentTokenError::NotEnrollable));
    }

    #[tokio::test]
    async fn enroll_with_valid_token_stores_client_public_key() {
        let mut repository = machine_repository_returning(machine::invited_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                matches!(
                    &machine.connectivity,
                    Connectivity::Tunneled(tunnel::Enrollment::Enrolled { client_public_key })
                        if *client_public_key == keys::tunnel_public_key()
                )
            })
            .returning(|_, _| Ok(machine::enrolled_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .tokens(tokens_that_mac_to(enrollment::TOKEN_MAC))
            .transactions(transactions_expecting_one_commit())
            .build();

        service
            .enroll(
                &mut TxContext::none(),
                uuid::Uuid::now_v7(),
                &enrollment::enrollment_token(),
                keys::tunnel_public_key(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn enroll_with_wrong_token_returns_invalid_token() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::invited_machine))
            .tokens(tokens_that_mac_to(WRONG_MAC))
            .transactions(transactions_expecting_no_commit())
            .build();

        let enrolled = service
            .enroll(
                &mut TxContext::none(),
                uuid::Uuid::now_v7(),
                &enrollment::enrollment_token(),
                keys::tunnel_public_key(),
            )
            .await;

        assert_matches!(enrolled.err(), Some(EnrollError::InvalidToken));
    }

    #[tokio::test]
    async fn enroll_at_or_after_expiry_returns_expired() {
        for now in [enrollment::expires_at(), enrollment::after_expiry()] {
            let service = ServiceBuilder::new()
                .repository(machine_repository_returning(machine::invited_machine))
                .transactions(transactions_expecting_no_commit())
                .now(now)
                .build();

            let enrolled = service
                .enroll(
                    &mut TxContext::none(),
                    uuid::Uuid::now_v7(),
                    &enrollment::enrollment_token(),
                    keys::tunnel_public_key(),
                )
                .await;

            assert_matches!(enrolled.err(), Some(EnrollError::Expired), "{now}");
        }
    }

    #[tokio::test]
    async fn enroll_uninvited_machine_returns_not_invited() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::unenrolled_machine))
            .transactions(transactions_expecting_no_commit())
            .build();

        let enrolled = service
            .enroll(
                &mut TxContext::none(),
                uuid::Uuid::now_v7(),
                &enrollment::enrollment_token(),
                keys::tunnel_public_key(),
            )
            .await;

        assert_matches!(enrolled.err(), Some(EnrollError::NotInvited));
    }

    #[tokio::test]
    async fn enroll_enrolled_machine_returns_already_enrolled() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::enrolled_machine))
            .transactions(transactions_expecting_no_commit())
            .build();

        let enrolled = service
            .enroll(
                &mut TxContext::none(),
                uuid::Uuid::now_v7(),
                &enrollment::enrollment_token(),
                keys::tunnel_public_key(),
            )
            .await;

        assert_matches!(enrolled.err(), Some(EnrollError::AlreadyEnrolled));
    }

    #[tokio::test]
    async fn enroll_direct_machine_returns_not_enrollable() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::direct_machine))
            .transactions(transactions_expecting_no_commit())
            .build();

        let enrolled = service
            .enroll(
                &mut TxContext::none(),
                uuid::Uuid::now_v7(),
                &enrollment::enrollment_token(),
                keys::tunnel_public_key(),
            )
            .await;

        assert_matches!(enrolled.err(), Some(EnrollError::NotEnrollable));
    }

    #[tokio::test]
    async fn unenroll_without_machine_enroll_returns_denied() {
        let service = ServiceBuilder::new().build();

        let unenrolled = service
            .unenroll(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                uuid::Uuid::now_v7(),
            )
            .await;

        assert_matches!(unenrolled.err(), Some(UnenrollError::Denied(_)));
    }

    #[tokio::test]
    async fn unenroll_enrolled_machine_stores_it_not_enrolled() {
        let mut repository = machine_repository_returning(machine::enrolled_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                matches!(
                    machine.connectivity,
                    Connectivity::Tunneled(tunnel::Enrollment::NotEnrolled)
                )
            })
            .returning(|_, _| Ok(machine::unenrolled_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .transactions(transactions_expecting_one_commit())
            .build();

        service
            .unenroll(&mut TxContext::none(), &Actor::System, uuid::Uuid::now_v7())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn unenroll_publishes_the_end_of_the_enrollment() {
        let mut repository = machine_repository_returning(machine::enrolled_machine);
        repository
            .expect_update()
            .returning(|_, _| Ok(machine::unenrolled_machine()));
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .transactions(transactions_expecting_one_commit())
            .build();

        service
            .unenroll(&mut TxContext::none(), &Actor::System, uuid::Uuid::now_v7())
            .await
            .unwrap();

        let published: Event<EventKind> = machine_events
            .next()
            .now_or_never()
            .unwrap()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            published.kind,
            EventKind::Updated {
                connectivity_changed: true,
                protocols_changed: BTreeSet::new(),
                enrollment_ended: true,
            }
        );
    }

    #[tokio::test]
    async fn unenroll_unenrolled_machine_succeeds() {
        let mut repository = machine_repository_returning(machine::unenrolled_machine);
        repository
            .expect_update()
            .once()
            .returning(|_, _| Ok(machine::unenrolled_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .transactions(transactions_expecting_one_commit())
            .build();

        service
            .unenroll(&mut TxContext::none(), &Actor::System, uuid::Uuid::now_v7())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn unenroll_direct_machine_returns_not_enrollable() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::direct_machine))
            .transactions(transactions_expecting_no_commit())
            .build();

        let unenrolled = service
            .unenroll(&mut TxContext::none(), &Actor::System, uuid::Uuid::now_v7())
            .await;

        assert_matches!(unenrolled.err(), Some(UnenrollError::NotEnrollable));
    }

    #[tokio::test]
    async fn authorize_unknown_machine_returns_false() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_without_machines())
            .build();

        let authorized = service
            .authorize(uuid::Uuid::now_v7(), &keys::tunnel_public_key())
            .await;

        assert!(!authorized);
    }

    #[tokio::test]
    async fn authorize_direct_machine_returns_false() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::direct_machine))
            .build();

        let authorized = service
            .authorize(uuid::Uuid::now_v7(), &keys::tunnel_public_key())
            .await;

        assert!(!authorized);
    }

    #[tokio::test]
    async fn authorize_invited_machine_returns_false() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::invited_machine))
            .build();

        let authorized = service
            .authorize(uuid::Uuid::now_v7(), &keys::tunnel_public_key())
            .await;

        assert!(!authorized);
    }

    #[tokio::test]
    async fn authorize_enrolled_machine_with_its_own_key_returns_true() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::enrolled_machine))
            .build();

        let authorized = service
            .authorize(uuid::Uuid::now_v7(), &keys::tunnel_public_key())
            .await;

        assert!(authorized);
    }

    #[tokio::test]
    async fn authorize_enrolled_machine_with_another_key_returns_false() {
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine::enrolled_machine))
            .build();

        let authorized = service
            .authorize(uuid::Uuid::now_v7(), &keys::other_tunnel_public_key())
            .await;

        assert!(!authorized);
    }

    #[tokio::test]
    async fn issue_enrollment_token_records_the_invitation() {
        let mut repository = machine_repository_returning(machine::unenrolled_machine);
        repository
            .expect_update()
            .returning(|_, machine| Ok(machine.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .repository(repository)
            .tokens(tokens_that_issue())
            .transactions(transactions_expecting_one_commit())
            .audit(Arc::clone(&audited))
            .build();

        service
            .issue_enrollment_token(
                &mut TxContext::none(),
                &Actor::System,
                IssueEnrollmentTokenCommand {
                    machine_id: uuid::Uuid::now_v7(),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            audited.recorded_kinds(),
            [AuditKind::Updated {
                before: MachineSnapshot::from(&machine::unenrolled_machine()),
                after: MachineSnapshot::from(&machine::invited_machine()),
                credentials_replaced: BTreeSet::new(),
            }
            .into()]
        );
    }

    #[tokio::test]
    async fn enroll_records_the_enrollment() {
        let mut repository = machine_repository_returning(machine::invited_machine);
        repository
            .expect_update()
            .returning(|_, machine| Ok(machine.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .repository(repository)
            .tokens(tokens_that_mac_to(enrollment::TOKEN_MAC))
            .transactions(transactions_expecting_one_commit())
            .audit(Arc::clone(&audited))
            .build();

        service
            .enroll(
                &mut TxContext::none(),
                uuid::Uuid::now_v7(),
                &enrollment::enrollment_token(),
                keys::tunnel_public_key(),
            )
            .await
            .unwrap();

        assert_eq!(
            audited.recorded_kinds(),
            [AuditKind::Updated {
                before: MachineSnapshot::from(&machine::invited_machine()),
                after: MachineSnapshot::from(&machine::enrolled_machine()),
                credentials_replaced: BTreeSet::new(),
            }
            .into()]
        );
    }

    #[tokio::test]
    async fn unenroll_records_the_end_of_the_enrollment() {
        let mut repository = machine_repository_returning(machine::enrolled_machine);
        repository
            .expect_update()
            .returning(|_, machine| Ok(machine.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .repository(repository)
            .transactions(transactions_expecting_one_commit())
            .audit(Arc::clone(&audited))
            .build();

        service
            .unenroll(&mut TxContext::none(), &Actor::System, uuid::Uuid::now_v7())
            .await
            .unwrap();

        assert_eq!(
            audited.recorded_kinds(),
            [AuditKind::Updated {
                before: MachineSnapshot::from(&machine::enrolled_machine()),
                after: MachineSnapshot::from(&machine::unenrolled_machine()),
                credentials_replaced: BTreeSet::new(),
            }
            .into()]
        );
    }
}

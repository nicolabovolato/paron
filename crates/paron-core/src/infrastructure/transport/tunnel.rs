use std::sync::Arc;

use async_trait::async_trait;
use paron_tunnel_protocol::{
    Access, AgentId, BrokerPolicy, PublicKey, Rejected, SequenceOutcome, StillBound,
};
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};

use crate::application::identity::authorization::actor::Actor;
use crate::application::machine::MachineAuthenticationService;
use crate::application::machine::{EnrollError, UnenrollError};
use crate::application::repository::{RepositoryError, TxContext};
use crate::domain::machine::tunnel;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SequenceStore: Send + Sync {
    async fn advance(&self, machine: uuid::Uuid, sequence: u64) -> Result<bool, RepositoryError>;
    async fn clear(&self, machine: uuid::Uuid) -> Result<(), RepositoryError>;
}

pub struct MachinePolicy<Tx> {
    authentication: Arc<MachineAuthenticationService<Tx>>,
    sequences: Arc<dyn SequenceStore>,
}

impl<Tx> MachinePolicy<Tx> {
    pub fn new(
        authentication: Arc<MachineAuthenticationService<Tx>>,
        sequences: Arc<dyn SequenceStore>,
    ) -> Self {
        Self {
            authentication,
            sequences,
        }
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> BrokerPolicy for MachinePolicy<Tx> {
    async fn verify_access(&self, agent: AgentId, authenticated_key: &PublicKey) -> Access {
        let Ok(authenticated_key) = tunnel::PublicKey::parse(authenticated_key.as_bytes()) else {
            return Access::Denied;
        };

        if self
            .authentication
            .authorize(agent.uuid(), &authenticated_key)
            .await
        {
            Access::Granted
        } else {
            Access::Denied
        }
    }

    async fn assign_identity(
        &self,
        agent: AgentId,
        authenticated_key: &PublicKey,
        token: &SecretSlice<u8>,
    ) -> Result<(), Rejected> {
        let Ok(authenticated_key) = tunnel::PublicKey::parse(authenticated_key.as_bytes()) else {
            return Err(Rejected);
        };

        let Ok(token) = std::str::from_utf8(token.expose_secret()) else {
            return Err(Rejected);
        };
        let token = SecretString::from(token.to_owned());

        if let Err(error) = self.sequences.clear(agent.uuid()).await {
            tracing::error!(
                machine = %agent.uuid(),
                ?error,
                "could not clear a machine's claim sequence, so it is refused rather than \
                 enrolled into a sequence it cannot exceed"
            );

            return Err(Rejected);
        }

        self.authentication
            .enroll(
                &mut TxContext::none(),
                agent.uuid(),
                &token,
                authenticated_key,
            )
            .await
            .map_err(|error| {
                if let EnrollError::Unexpected(cause) = &error {
                    tracing::error!(machine = %agent.uuid(), ?cause, "could not enroll a machine");
                }

                Rejected
            })
    }

    async fn advance_sequence(&self, agent: AgentId, sequence: u64) -> SequenceOutcome {
        match self.sequences.advance(agent.uuid(), sequence).await {
            Ok(true) => SequenceOutcome::Current,
            Ok(false) => SequenceOutcome::Stale,
            Err(error) => {
                tracing::error!(
                    machine = %agent.uuid(),
                    ?error,
                    "could not read a machine's claim sequence, so its claim is refused"
                );

                SequenceOutcome::Undetermined
            }
        }
    }

    async fn unassign_identity(&self, agent: AgentId) -> Result<(), StillBound> {
        self.authentication
            .unenroll(&mut TxContext::none(), &Actor::System, agent.uuid())
            .await
            .map_err(|error| {
                if let UnenrollError::Unexpected(cause) = &error {
                    tracing::error!(machine = %agent.uuid(), ?cause, "could not unenroll a machine");
                }

                StillBound
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::machine::{MachineService, MockRepository, MockSshKeypairs};
    use crate::application::tokens::MockTokens;
    use crate::domain::machine::Connectivity;
    use crate::testing::logs::CapturedLogs;
    use crate::testing::mocks::{machine_repository_returning, transactions_that_commit};
    use crate::testing::{clock, enrollment, events, keys, machine};

    fn machine_policy(
        repository: MockRepository<()>,
        sequences: MockSequenceStore,
    ) -> MachinePolicy<()> {
        let transactions = Arc::new(transactions_that_commit());

        MachinePolicy::new(
            Arc::new(MachineAuthenticationService::new(
                Arc::new(MachineService::new(
                    Arc::new(repository),
                    Arc::clone(&transactions) as Arc<_>,
                    Arc::new(MockSshKeypairs::new()),
                    events::audit_service(),
                    events::event_service(),
                )),
                transactions,
                Arc::new(MockTokens::new()),
                clock::clock_at(enrollment::issued_at()),
                enrollment::enrollment_policy(),
                enrollment::broker_fingerprint(),
                enrollment::broker_address(),
            )),
            Arc::new(sequences),
        )
    }

    fn agent_id() -> AgentId {
        AgentId::new(uuid::Uuid::now_v7())
    }

    fn tunnel_public_key() -> PublicKey {
        PublicKey::parse(keys::TUNNEL_PUBLIC_KEY).unwrap()
    }

    #[tokio::test]
    async fn verify_access_for_enrolled_machine_with_its_own_key_returns_granted() {
        let policy = machine_policy(
            machine_repository_returning(machine::enrolled_machine),
            MockSequenceStore::new(),
        );

        let access = policy.verify_access(agent_id(), &tunnel_public_key()).await;

        assert_eq!(access, Access::Granted);
    }

    #[tokio::test]
    async fn verify_access_for_enrolled_machine_with_another_key_returns_denied() {
        let policy = machine_policy(
            machine_repository_returning(machine::enrolled_machine),
            MockSequenceStore::new(),
        );

        let access = policy
            .verify_access(
                agent_id(),
                &PublicKey::parse(keys::OTHER_TUNNEL_PUBLIC_KEY).unwrap(),
            )
            .await;

        assert_eq!(access, Access::Denied);
    }

    #[tokio::test]
    async fn advance_sequence_higher_than_the_mark_returns_current() {
        let mut sequences = MockSequenceStore::new();
        sequences.expect_advance().returning(|_, _| Ok(true));
        let policy = machine_policy(MockRepository::new(), sequences);

        let outcome = policy.advance_sequence(agent_id(), 4).await;

        assert_eq!(outcome, SequenceOutcome::Current);
    }

    #[tokio::test]
    async fn advance_sequence_not_higher_than_the_mark_returns_stale() {
        let mut sequences = MockSequenceStore::new();
        sequences.expect_advance().returning(|_, _| Ok(false));
        let policy = machine_policy(MockRepository::new(), sequences);

        let outcome = policy.advance_sequence(agent_id(), 4).await;

        assert_eq!(outcome, SequenceOutcome::Stale);
    }

    #[tokio::test]
    async fn advance_sequence_with_unreachable_store_returns_undetermined_and_logs_the_error() {
        let logs = CapturedLogs::capture();
        let agent = agent_id();
        let mut sequences = MockSequenceStore::new();
        sequences
            .expect_advance()
            .returning(|_, _| Err(RepositoryError::Unexpected(anyhow::anyhow!("no database"))));
        let policy = machine_policy(MockRepository::new(), sequences);

        let outcome = policy.advance_sequence(agent, 4).await;

        assert_eq!(outcome, SequenceOutcome::Undetermined);
        let lines = logs.lines();
        assert!(
            lines.iter().any(|line| {
                line.contains("no database") && line.contains(&agent.uuid().to_string())
            }),
            "{lines:?}"
        );
    }

    #[tokio::test]
    async fn unassign_identity_unbinds_the_key() {
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
        let policy = machine_policy(repository, MockSequenceStore::new());

        let unassigned = policy.unassign_identity(agent_id()).await;

        assert_eq!(unassigned, Ok(()));
    }

    #[tokio::test]
    async fn unassign_identity_that_cannot_be_written_returns_still_bound() {
        let mut repository = machine_repository_returning(machine::enrolled_machine);
        repository
            .expect_update()
            .returning(|_, _| Err(RepositoryError::Unexpected(anyhow::anyhow!("no database"))));
        let policy = machine_policy(repository, MockSequenceStore::new());

        let unassigned = policy.unassign_identity(agent_id()).await;

        assert_eq!(unassigned, Err(StillBound));
    }

    #[tokio::test]
    async fn assign_identity_without_invitation_clears_the_sequence_and_returns_rejected() {
        let mut sequences = MockSequenceStore::new();
        sequences.expect_clear().once().returning(|_| Ok(()));
        let policy = machine_policy(
            machine_repository_returning(machine::unenrolled_machine),
            sequences,
        );

        let assigned = policy
            .assign_identity(
                agent_id(),
                &tunnel_public_key(),
                &SecretSlice::from(b"a-token".to_vec()),
            )
            .await;

        assert_eq!(assigned, Err(Rejected));
    }

    #[tokio::test]
    async fn assign_identity_with_unreachable_sequence_store_returns_rejected() {
        let mut sequences = MockSequenceStore::new();
        sequences
            .expect_clear()
            .once()
            .returning(|_| Err(RepositoryError::Unexpected(anyhow::anyhow!("no database"))));
        let policy = machine_policy(MockRepository::new(), sequences);

        let assigned = policy
            .assign_identity(
                agent_id(),
                &tunnel_public_key(),
                &SecretSlice::from(b"a-token".to_vec()),
            )
            .await;

        assert_eq!(assigned, Err(Rejected));
    }
}

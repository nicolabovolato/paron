use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;

use super::commands::create::CreateMachineCommand;
use super::commands::update::UpdateMachineCommand;
use super::errors::{
    CreateMachineError, DeleteMachineError, GetMachineError, ListMachinesError, UpdateMachineError,
};
use super::ports::{Repository, SshKeypairs};
use super::use_cases::MachineManagement;
use crate::application::audit::AuditService;
use crate::application::events::EventService;
use crate::application::identity::authorization::actor::Actor;
use crate::application::machine::event::{AuditKind, EventKind, MachineSnapshot};
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{
    ListQuery, Paginated, RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _,
};
use crate::domain::machine::tunnel::Enrollment;
use crate::domain::machine::{Connectivity, Machine, ProtocolKind, tunnel};
use crate::domain::user::Permission;

pub struct MachineService<Tx> {
    repository: Arc<dyn Repository<Tx>>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    keypairs: Arc<dyn SshKeypairs>,
    audit: Arc<AuditService<Tx>>,
    events: Arc<EventService>,
}

impl<Tx: Send + Sync + 'static> MachineService<Tx> {
    pub fn new(
        repository: Arc<dyn Repository<Tx>>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        keypairs: Arc<dyn SshKeypairs>,
        audit: Arc<AuditService<Tx>>,
        events: Arc<EventService>,
    ) -> Self {
        Self {
            repository,
            unit_of_work,
            keypairs,
            audit,
            events,
        }
    }

    pub async fn list(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        query: &ListQuery,
    ) -> Result<Paginated<Machine>, ListMachinesError> {
        actor.authorize(Permission::MachineRead)?;

        Ok(self
            .repository
            .list(ctx, query)
            .await
            .context("could not list machines")?)
    }

    pub async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        cmd: CreateMachineCommand,
    ) -> Result<Machine, CreateMachineError> {
        actor.authorize(Permission::MachineWrite)?;

        let machine = cmd.into_domain(&*self.keypairs);

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let machine = self
                    .repository
                    .create(ctx, &machine)
                    .await
                    .context("could not create machine")?;

                self.audit
                    .append(
                        ctx,
                        actor,
                        &machine,
                        AuditKind::Created(MachineSnapshot::from(&machine)),
                    )
                    .await?;

                Ok(machine)
            })
            .await
    }

    pub async fn get(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        id: uuid::Uuid,
    ) -> Result<Machine, GetMachineError> {
        actor.authorize(Permission::MachineRead)?;

        match self.repository.get_by_id(ctx, id).await {
            Ok(machine) => Ok(machine),
            Err(RepositoryError::NotFound) => Err(GetMachineError::NotFound),
            Err(error) => Err(anyhow::Error::new(error)
                .context("could not get machine by id")
                .into()),
        }
    }

    pub async fn update(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        cmd: UpdateMachineCommand,
    ) -> Result<Machine, UpdateMachineError> {
        actor.authorize(Permission::MachineWrite)?;

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let old = match self.repository.get_by_id(ctx, cmd.id).await {
                    Ok(old) => old,
                    Err(RepositoryError::NotFound) => return Err(UpdateMachineError::NotFound),
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get machine by id")
                            .into());
                    }
                };
                let new = cmd.apply_to(&old, &*self.keypairs)?;

                Ok(self.replace(ctx, actor, &old, &new).await?)
            })
            .await
    }

    pub async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        id: uuid::Uuid,
    ) -> Result<Machine, DeleteMachineError> {
        actor.authorize(Permission::MachineDelete)?;

        let machine = self
            .unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let machine = match self.repository.delete(ctx, id).await {
                    Ok(machine) => machine,
                    Err(RepositoryError::NotFound) => return Err(DeleteMachineError::NotFound),
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not delete machine")
                            .into());
                    }
                };

                self.audit
                    .append(ctx, actor, &machine, AuditKind::Deleted)
                    .await?;

                Ok::<_, DeleteMachineError>(machine)
            })
            .await?;

        self.events.publish(machine.id, EventKind::Deleted);

        Ok(machine)
    }

    /// Mutates the enrollment of a tunneled machine
    pub(super) async fn set_enrollment(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        id: uuid::Uuid,
        enrollment: tunnel::Enrollment,
    ) -> Result<Machine, anyhow::Error> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let old = self.repository.get_by_id(ctx, id).await?;
                let new = Machine {
                    connectivity: Connectivity::Tunneled(enrollment),
                    ..old.clone()
                };

                self.replace(ctx, actor, &old, &new).await
            })
            .await
    }

    async fn replace(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        old: &Machine,
        new: &Machine,
    ) -> Result<Machine, anyhow::Error> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let machine = self.repository.update(ctx, new).await?;

                let ssh_credentials_replaced = match (&old.protocols.ssh, &machine.protocols.ssh) {
                    (Some(old_access), Some(stored_access)) => !old_access
                        .authentication
                        .method()
                        .has_same_secret(stored_access.authentication.method()),
                    _ => false,
                };
                let credentials_replaced =
                    BTreeSet::from_iter(ssh_credentials_replaced.then_some(ProtocolKind::Ssh));

                let before = MachineSnapshot::from(old);
                let after = MachineSnapshot::from(&machine);

                self.audit
                    .append(
                        ctx,
                        actor,
                        &machine,
                        AuditKind::Updated {
                            before: before.clone(),
                            after: after.clone(),
                            credentials_replaced: credentials_replaced.clone(),
                        },
                    )
                    .await?;

                let enrollment_ended = match (&old.connectivity, &machine.connectivity) {
                    (
                        Connectivity::Tunneled(Enrollment::Enrolled {
                            client_public_key: enrolled_before,
                        }),
                        Connectivity::Tunneled(Enrollment::Enrolled {
                            client_public_key: enrolled_after,
                        }),
                    ) => enrolled_before != enrolled_after,
                    (Connectivity::Tunneled(Enrollment::Enrolled { .. }), _) => true,
                    _ => false,
                };

                self.events.publish(
                    machine.id,
                    EventKind::Updated {
                        connectivity_changed: before.connectivity != after.connectivity,
                        protocols_changed: ProtocolKind::ALL
                            .iter()
                            .copied()
                            .filter(|kind| {
                                let snapshot_changed = match kind {
                                    ProtocolKind::Ssh => {
                                        before.protocols.ssh != after.protocols.ssh
                                    }
                                };

                                snapshot_changed || credentials_replaced.contains(kind)
                            })
                            .collect(),
                        enrollment_ended,
                    },
                );

                Ok(machine)
            })
            .await
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> MachineManagement for MachineService<Tx> {
    async fn list(
        &self,
        actor: &Actor,
        query: &ListQuery,
    ) -> Result<Paginated<Machine>, ListMachinesError> {
        Self::list(self, &mut TxContext::none(), actor, query).await
    }

    async fn get(&self, actor: &Actor, id: uuid::Uuid) -> Result<Machine, GetMachineError> {
        Self::get(self, &mut TxContext::none(), actor, id).await
    }

    async fn create(
        &self,
        actor: &Actor,
        cmd: CreateMachineCommand,
    ) -> Result<Machine, CreateMachineError> {
        Self::create(self, &mut TxContext::none(), actor, cmd).await
    }

    async fn update(
        &self,
        actor: &Actor,
        cmd: UpdateMachineCommand,
    ) -> Result<Machine, UpdateMachineError> {
        Self::update(self, &mut TxContext::none(), actor, cmd).await
    }

    async fn delete(&self, actor: &Actor, id: uuid::Uuid) -> Result<Machine, DeleteMachineError> {
        Self::delete(self, &mut TxContext::none(), actor, id).await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use futures_util::FutureExt as _;
    use secrecy::{ExposeSecret as _, SecretSlice, SecretString};

    use super::*;
    use crate::application::audit::Entity;
    use crate::application::events::{Event, EventStream as _, SubscriptionScope};
    use crate::application::machine::commands::create::{
        AuthenticationCommand, ConnectivityCommand, ProtocolsCommand, SshCommand,
    };
    use crate::application::machine::commands::update::{
        InvalidUpdate, ProtocolUpdate, UpdateProtocolsCommand, UpdateSshCommand,
    };
    use crate::application::machine::ports::{MockRepository, MockSshKeypairs, SshKeyAlgorithm};
    use crate::domain::entity::EntityKind;
    use crate::domain::machine::{MachineName, ssh};
    use crate::infrastructure::in_memory::events::InMemoryEventStream;
    use crate::testing::clock::clock_at;
    use crate::testing::events::RecordedAudit;
    use crate::testing::mocks::{machine_repository_returning, transactions_that_commit};
    use crate::testing::{events, identity, keys, machine, pagination};

    struct ServiceBuilder {
        repository: MockRepository<()>,
        keypairs: MockSshKeypairs,
        events: Arc<InMemoryEventStream>,
        audit: Arc<RecordedAudit<()>>,
    }

    impl ServiceBuilder {
        fn new() -> Self {
            Self {
                repository: MockRepository::new(),
                keypairs: MockSshKeypairs::new(),
                events: Arc::new(InMemoryEventStream::default()),
                audit: events::recorded_audit(),
            }
        }

        fn audit(mut self, audit: Arc<RecordedAudit<()>>) -> Self {
            self.audit = audit;
            self
        }

        fn repository(mut self, repository: MockRepository<()>) -> Self {
            self.repository = repository;
            self
        }

        fn events(mut self, events: Arc<InMemoryEventStream>) -> Self {
            self.events = events;
            self
        }

        fn keypairs(mut self, keypairs: MockSshKeypairs) -> Self {
            self.keypairs = keypairs;
            self
        }

        fn build(self) -> MachineService<()> {
            MachineService::new(
                Arc::new(self.repository),
                Arc::new(transactions_that_commit()),
                Arc::new(self.keypairs),
                Arc::new(AuditService::new(
                    self.audit,
                    clock_at(jiff::Timestamp::UNIX_EPOCH),
                )),
                events::event_service_on(self.events, clock_at(jiff::Timestamp::UNIX_EPOCH)),
            )
        }
    }

    fn create_command_for(machine: Machine) -> CreateMachineCommand {
        CreateMachineCommand {
            name: machine.name,
            connectivity: connectivity_command_for(machine.connectivity),
            protocols: ProtocolsCommand {
                ssh: machine.protocols.ssh.map(ssh_command_for),
            },
        }
    }

    fn update_command_for(machine: Machine) -> UpdateMachineCommand {
        let ssh = match machine.protocols.ssh.map(ssh_command_for) {
            None => ProtocolUpdate::Remove,
            Some(cmd) => ProtocolUpdate::Set(UpdateSshCommand {
                port: Some(cmd.port),
                user: Some(cmd.user),
                host_key: Some(cmd.host_key),
                authentication: Some(cmd.authentication),
                connect_timeout: Some(cmd.connect_timeout),
            }),
        };

        UpdateMachineCommand {
            id: machine.id,
            name: Some(machine.name),
            connectivity: Some(connectivity_command_for(machine.connectivity)),
            protocols: UpdateProtocolsCommand { ssh: Some(ssh) },
        }
    }

    fn update_command_without_changes() -> UpdateMachineCommand {
        UpdateMachineCommand {
            id: uuid::Uuid::now_v7(),
            name: None,
            connectivity: None,
            protocols: UpdateProtocolsCommand::default(),
        }
    }

    fn update_ssh_command_without_changes() -> UpdateSshCommand {
        UpdateSshCommand {
            port: None,
            user: None,
            host_key: None,
            authentication: None,
            connect_timeout: None,
        }
    }

    fn update_command_with_new_host_key(
        authentication: Option<AuthenticationCommand>,
    ) -> UpdateMachineCommand {
        UpdateMachineCommand {
            protocols: UpdateProtocolsCommand {
                ssh: Some(ProtocolUpdate::Set(UpdateSshCommand {
                    host_key: Some(keys::unauthorized_ssh_private_key().public_key()),
                    authentication,
                    ..update_ssh_command_without_changes()
                })),
            },
            ..update_command_without_changes()
        }
    }

    fn update_command_with_new_password() -> UpdateMachineCommand {
        UpdateMachineCommand {
            protocols: UpdateProtocolsCommand {
                ssh: Some(ProtocolUpdate::Set(UpdateSshCommand {
                    authentication: Some(AuthenticationCommand::Password(
                        ssh::Password::parse(SecretString::from("a new password")).unwrap(),
                    )),
                    ..update_ssh_command_without_changes()
                })),
            },
            ..update_command_without_changes()
        }
    }

    fn ssh_command_for(access: ssh::Access) -> SshCommand {
        SshCommand {
            port: access.port,
            user: access.user,
            host_key: access.authentication.host_key().clone(),
            authentication: match access.authentication.method() {
                ssh::AuthMethod::Password(password) => {
                    AuthenticationCommand::Password(password.clone())
                }
                ssh::AuthMethod::Keypair {
                    source: ssh::KeySource::Managed,
                    ..
                } => AuthenticationCommand::ManagedKeypair(SshKeyAlgorithm::Ed25519),
                ssh::AuthMethod::Keypair {
                    private_key,
                    source: ssh::KeySource::Imported,
                } => AuthenticationCommand::ImportedKeypair(
                    ssh::PrivateKey::parse(SecretSlice::from(private_key.expose_secret().to_vec()))
                        .unwrap(),
                ),
            },
            connect_timeout: access.connect_timeout,
        }
    }

    fn connectivity_command_for(connectivity: Connectivity) -> ConnectivityCommand {
        match connectivity {
            Connectivity::Direct { host } => ConnectivityCommand::Direct { host },
            Connectivity::Tunneled(_) => ConnectivityCommand::Tunneled,
        }
    }

    fn machine_without_ssh() -> Machine {
        machine::machine_builder().without_ssh().build()
    }

    fn managed_machine() -> Machine {
        machine::machine_builder()
            .authentication(machine::managed_keypair_method(
                keys::unauthorized_ssh_private_key(),
            ))
            .build()
    }

    fn managed_public_key_of(machine: &Machine) -> Option<ssh::PublicKey> {
        match machine::ssh_access(machine).authentication.method() {
            ssh::AuthMethod::Keypair {
                private_key,
                source: ssh::KeySource::Managed,
            } => Some(private_key.public_key()),
            ssh::AuthMethod::Keypair { .. } | ssh::AuthMethod::Password(_) => None,
        }
    }

    fn keypairs_that_generate_once() -> MockSshKeypairs {
        let mut keypairs = MockSshKeypairs::new();
        keypairs
            .expect_generate()
            .once()
            .returning(|_| keys::ssh_private_key());

        keypairs
    }

    #[tokio::test]
    async fn list_without_machine_read_returns_denied() {
        let service = ServiceBuilder::new().build();

        let listed = service
            .list(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                &pagination::everything(),
            )
            .await;

        assert_matches!(listed.err(), Some(ListMachinesError::Denied(_)));
    }

    #[tokio::test]
    async fn get_without_machine_read_returns_denied() {
        let service = ServiceBuilder::new().build();

        let found = service
            .get(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                uuid::Uuid::now_v7(),
            )
            .await;

        assert_matches!(found.err(), Some(GetMachineError::Denied(_)));
    }

    #[tokio::test]
    async fn create_without_machine_write_returns_denied() {
        let service = ServiceBuilder::new().build();

        let created = service
            .create(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                create_command_for(machine::direct_machine()),
            )
            .await;

        assert_matches!(created.err(), Some(CreateMachineError::Denied(_)));
    }

    #[tokio::test]
    async fn create_tunneled_machine_stores_it_not_enrolled() {
        let mut repository = MockRepository::new();
        repository
            .expect_create()
            .once()
            .withf(|_, machine| {
                matches!(
                    machine.connectivity,
                    Connectivity::Tunneled(tunnel::Enrollment::NotEnrolled)
                )
            })
            .returning(|_, _| Ok(machine::unenrolled_machine()));
        let service = ServiceBuilder::new().repository(repository).build();

        service
            .create(
                &mut TxContext::none(),
                &Actor::System,
                create_command_for(machine::enrolled_machine()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_with_managed_keypair_stores_generated_key() {
        let mut repository = MockRepository::new();
        repository
            .expect_create()
            .once()
            .withf(|_, machine| {
                managed_public_key_of(machine) == Some(keys::ssh_private_key().public_key())
            })
            .returning(|_, _| Ok(machine::direct_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .keypairs(keypairs_that_generate_once())
            .build();

        service
            .create(
                &mut TxContext::none(),
                &Actor::System,
                create_command_for(managed_machine()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_without_machine_write_returns_denied() {
        let service = ServiceBuilder::new().build();

        let updated = service
            .update(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                update_command_for(machine::direct_machine()),
            )
            .await;

        assert_matches!(updated.err(), Some(UpdateMachineError::Denied(_)));
    }

    #[tokio::test]
    async fn update_tunneled_machine_keeps_its_enrollment() {
        let mut repository = machine_repository_returning(machine::enrolled_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                matches!(
                    machine.connectivity,
                    Connectivity::Tunneled(tunnel::Enrollment::Enrolled { .. })
                )
            })
            .returning(|_, _| Ok(machine::enrolled_machine()));
        let service = ServiceBuilder::new().repository(repository).build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::unenrolled_machine()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_direct_machine_to_tunneled_stores_it_not_enrolled() {
        let mut repository = machine_repository_returning(machine::direct_machine);
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
        let service = ServiceBuilder::new().repository(repository).build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::unenrolled_machine()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_enrolled_machine_to_direct_publishes_the_end_of_the_enrollment() {
        let mut repository = machine_repository_returning(machine::enrolled_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| matches!(machine.connectivity, Connectivity::Direct { .. }))
            .returning(|_, _| Ok(machine::direct_machine()));
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::direct_machine()),
            )
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
    async fn update_invited_machine_to_direct_publishes_no_end_of_an_enrollment() {
        let mut repository = machine_repository_returning(machine::invited_machine);
        repository
            .expect_update()
            .once()
            .returning(|_, _| Ok(machine::direct_machine()));
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::direct_machine()),
            )
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
                enrollment_ended: false,
            }
        );
    }

    #[tokio::test]
    async fn update_with_a_new_user_publishes_a_change_of_protocol() {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository.expect_update().once().returning(|_, _| {
            let mut changed = machine::direct_machine();
            changed.protocols.ssh.as_mut().unwrap().user = ssh::User::parse("other").unwrap();

            Ok(changed)
        });
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::direct_machine()),
            )
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
                connectivity_changed: false,
                protocols_changed: BTreeSet::from([ProtocolKind::Ssh]),
                enrollment_ended: false,
            }
        );
    }

    #[tokio::test]
    async fn update_with_only_a_new_name_publishes_no_change() {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository.expect_update().once().returning(|_, _| {
            Ok(Machine {
                name: MachineName::parse("renamed").unwrap(),
                ..machine::direct_machine()
            })
        });
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::direct_machine()),
            )
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
                connectivity_changed: false,
                protocols_changed: BTreeSet::new(),
                enrollment_ended: false,
            }
        );
    }

    #[tokio::test]
    async fn update_enrolled_machine_staying_tunneled_publishes_no_change() {
        let mut repository = machine_repository_returning(machine::enrolled_machine);
        repository
            .expect_update()
            .once()
            .returning(|_, _| Ok(machine::enrolled_machine()));
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::unenrolled_machine()),
            )
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
                connectivity_changed: false,
                protocols_changed: BTreeSet::new(),
                enrollment_ended: false,
            }
        );
    }

    #[tokio::test]
    async fn update_without_authentication_keeps_stored_password() {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                &*machine.name == "renamed"
                    && matches!(
                        machine::ssh_access(machine).authentication.method(),
                        ssh::AuthMethod::Password(password)
                            if password.expose_secret() == machine::SSH_PASSWORD
                    )
            })
            .returning(|_, _| Ok(machine::direct_machine()));
        let service = ServiceBuilder::new().repository(repository).build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateMachineCommand {
                    name: Some(MachineName::parse("renamed").unwrap()),
                    ..update_command_without_changes()
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_managed_machine_without_authentication_keeps_its_key() {
        let mut repository = machine_repository_returning(managed_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                managed_public_key_of(machine)
                    == Some(keys::unauthorized_ssh_private_key().public_key())
            })
            .returning(|_, _| Ok(machine::direct_machine()));
        let mut keypairs = MockSshKeypairs::new();
        keypairs.expect_generate().never();
        let service = ServiceBuilder::new()
            .repository(repository)
            .keypairs(keypairs)
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateMachineCommand {
                    name: Some(MachineName::parse("renamed").unwrap()),
                    ..update_command_without_changes()
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_managed_machine_with_managed_keypair_stores_new_key() {
        let mut repository = machine_repository_returning(managed_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                managed_public_key_of(machine) == Some(keys::ssh_private_key().public_key())
            })
            .returning(|_, _| Ok(machine::direct_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .keypairs(keypairs_that_generate_once())
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(managed_machine()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_imported_machine_to_managed_stores_generated_key() {
        let mut repository = machine_repository_returning(|| {
            machine::machine_builder()
                .authentication(machine::imported_keypair_method(
                    keys::unauthorized_ssh_private_key(),
                ))
                .build()
        });
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                managed_public_key_of(machine) == Some(keys::ssh_private_key().public_key())
            })
            .returning(|_, _| Ok(machine::direct_machine()));
        let service = ServiceBuilder::new()
            .repository(repository)
            .keypairs(keypairs_that_generate_once())
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(managed_machine()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_machine_without_ssh_with_every_ssh_field_stores_ssh() {
        let mut repository = machine_repository_returning(machine_without_ssh);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| machine.protocols.ssh.is_some())
            .returning(|_, machine| Ok(machine.clone()));
        let service = ServiceBuilder::new().repository(repository).build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::direct_machine()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_machine_without_ssh_with_some_ssh_fields_returns_incomplete_protocol() {
        let mut repository = machine_repository_returning(machine_without_ssh);
        repository.expect_update().never();
        let service = ServiceBuilder::new().repository(repository).build();

        let updated = service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_with_new_password(),
            )
            .await;

        assert_matches!(
            updated.err(),
            Some(UpdateMachineError::InvalidUpdate(
                InvalidUpdate::IncompleteProtocol(ProtocolKind::Ssh)
            ))
        );
    }

    #[tokio::test]
    async fn update_machine_without_ssh_with_some_ssh_fields_generates_no_keypair() {
        let mut keypairs = MockSshKeypairs::new();
        keypairs.expect_generate().never();
        let service = ServiceBuilder::new()
            .repository(machine_repository_returning(machine_without_ssh))
            .keypairs(keypairs)
            .build();

        let updated = service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_with_new_host_key(Some(AuthenticationCommand::ManagedKeypair(
                    SshKeyAlgorithm::Ed25519,
                ))),
            )
            .await;

        assert_matches!(
            updated.err(),
            Some(UpdateMachineError::InvalidUpdate(
                InvalidUpdate::IncompleteProtocol(ProtocolKind::Ssh)
            ))
        );
    }

    #[tokio::test]
    async fn update_removing_ssh_publishes_a_change_of_protocol() {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| machine.protocols.ssh.is_none())
            .returning(|_, machine| Ok(machine.clone()));
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateMachineCommand {
                    protocols: UpdateProtocolsCommand {
                        ssh: Some(ProtocolUpdate::Remove),
                    },
                    ..update_command_without_changes()
                },
            )
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
                connectivity_changed: false,
                protocols_changed: BTreeSet::from([ProtocolKind::Ssh]),
                enrollment_ended: false,
            }
        );
    }

    #[tokio::test]
    async fn update_password_machine_to_new_host_key_without_password_returns_password_bound_to_host_key()
     {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository.expect_update().never();
        let service = ServiceBuilder::new().repository(repository).build();

        let updated = service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_with_new_host_key(None),
            )
            .await;

        assert_matches!(
            updated.err(),
            Some(UpdateMachineError::InvalidUpdate(
                InvalidUpdate::PasswordBoundToHostKey(_)
            ))
        );
    }

    #[tokio::test]
    async fn update_password_machine_to_new_host_key_with_password_stores_both() {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                let ssh_access = machine::ssh_access(machine);

                *ssh_access.authentication.host_key()
                    == keys::unauthorized_ssh_private_key().public_key()
                    && matches!(
                        ssh_access.authentication.method(),
                        ssh::AuthMethod::Password(password)
                            if password.expose_secret() == "entered again"
                    )
            })
            .returning(|_, _| Ok(machine::direct_machine()));
        let service = ServiceBuilder::new().repository(repository).build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_with_new_host_key(Some(AuthenticationCommand::Password(
                    ssh::Password::parse(SecretString::from("entered again")).unwrap(),
                ))),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_keypair_machine_to_new_host_key_keeps_its_key() {
        let mut repository = machine_repository_returning(|| {
            machine::machine_builder()
                .authentication(machine::imported_keypair_method(keys::ssh_private_key()))
                .build()
        });
        repository
            .expect_update()
            .once()
            .withf(|_, machine| {
                let ssh_access = machine::ssh_access(machine);

                *ssh_access.authentication.host_key()
                    == keys::unauthorized_ssh_private_key().public_key()
                    && matches!(
                        ssh_access.authentication.method(),
                        ssh::AuthMethod::Keypair { private_key, .. }
                            if private_key.public_key() == keys::ssh_private_key().public_key()
                    )
            })
            .returning(|_, _| Ok(machine::direct_machine()));
        let service = ServiceBuilder::new().repository(repository).build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_with_new_host_key(None),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_without_machine_delete_returns_denied() {
        let service = ServiceBuilder::new().build();

        let deleted = service
            .delete(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                uuid::Uuid::now_v7(),
            )
            .await;

        assert_matches!(deleted.err(), Some(DeleteMachineError::Denied(_)));
    }

    #[tokio::test]
    async fn delete_unknown_machine_returns_not_found() {
        let mut repository = MockRepository::new();
        repository
            .expect_delete()
            .once()
            .returning(|_, _| Err(RepositoryError::NotFound));
        let service = ServiceBuilder::new().repository(repository).build();

        let deleted = service
            .delete(&mut TxContext::none(), &Actor::System, uuid::Uuid::now_v7())
            .await;

        assert_matches!(deleted.err(), Some(DeleteMachineError::NotFound));
    }

    #[tokio::test]
    async fn delete_enrolled_machine_publishes_deleted() {
        let mut repository = MockRepository::new();
        repository
            .expect_delete()
            .once()
            .returning(|_, _| Ok(machine::enrolled_machine()));
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .build();

        service
            .delete(&mut TxContext::none(), &Actor::System, uuid::Uuid::now_v7())
            .await
            .unwrap();

        let published: Event<EventKind> = machine_events
            .next()
            .now_or_never()
            .unwrap()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(published.kind, EventKind::Deleted);
    }

    #[tokio::test]
    async fn create_records_the_created_machine() {
        let mut repository = MockRepository::new();
        repository
            .expect_create()
            .returning(|_, machine| Ok(machine.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .repository(repository)
            .audit(Arc::clone(&audited))
            .build();

        let created = service
            .create(
                &mut TxContext::none(),
                &Actor::System,
                create_command_for(machine::direct_machine()),
            )
            .await
            .unwrap();

        assert_eq!(
            audited
                .recorded()
                .into_iter()
                .map(|entry| (entry.target, entry.kind))
                .collect::<Vec<_>>(),
            [(
                Entity::from(&created),
                AuditKind::Created(MachineSnapshot::from(&created)).into()
            )]
        );
    }

    #[tokio::test]
    async fn delete_enrolled_machine_records_only_the_deletion() {
        let mut repository = MockRepository::new();
        repository
            .expect_delete()
            .returning(|_, _| Ok(machine::enrolled_machine()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .repository(repository)
            .audit(Arc::clone(&audited))
            .build();

        service
            .delete(&mut TxContext::none(), &Actor::System, uuid::Uuid::now_v7())
            .await
            .unwrap();

        assert_eq!(audited.recorded_kinds(), [AuditKind::Deleted.into()]);
    }

    #[tokio::test]
    async fn update_enrolled_machine_to_direct_records_one_update() {
        let mut repository = machine_repository_returning(machine::enrolled_machine);
        repository
            .expect_update()
            .returning(|_, machine| Ok(machine.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .repository(repository)
            .audit(Arc::clone(&audited))
            .build();
        let renamed_direct = Machine {
            name: MachineName::parse("renamed").unwrap(),
            ..machine::direct_machine()
        };

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(renamed_direct.clone()),
            )
            .await
            .unwrap();

        assert_eq!(
            audited.recorded_kinds(),
            [AuditKind::Updated {
                before: MachineSnapshot::from(&machine::enrolled_machine()),
                after: MachineSnapshot::from(&renamed_direct),
                credentials_replaced: BTreeSet::new(),
            }
            .into()]
        );
    }

    #[tokio::test]
    async fn update_with_the_stored_values_records_the_update() {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository
            .expect_update()
            .returning(|_, machine| Ok(machine.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .repository(repository)
            .audit(Arc::clone(&audited))
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_for(machine::direct_machine()),
            )
            .await
            .unwrap();

        let snapshot = MachineSnapshot::from(&machine::direct_machine());
        assert_eq!(
            audited.recorded_kinds(),
            [AuditKind::Updated {
                before: snapshot.clone(),
                after: snapshot,
                credentials_replaced: BTreeSet::new(),
            }
            .into()]
        );
    }

    #[tokio::test]
    async fn update_with_a_new_password_records_credentials_replaced() {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository
            .expect_update()
            .returning(|_, machine| Ok(machine.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .repository(repository)
            .audit(Arc::clone(&audited))
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_with_new_password(),
            )
            .await
            .unwrap();

        let snapshot = MachineSnapshot::from(&machine::direct_machine());
        assert_eq!(
            audited.recorded_kinds(),
            [AuditKind::Updated {
                before: snapshot.clone(),
                after: snapshot,
                credentials_replaced: BTreeSet::from([ProtocolKind::Ssh]),
            }
            .into()]
        );
    }

    #[tokio::test]
    async fn update_with_a_new_password_publishes_a_change_of_protocol() {
        let mut repository = machine_repository_returning(machine::direct_machine);
        repository
            .expect_update()
            .returning(|_, machine| Ok(machine.clone()));
        let events = Arc::new(InMemoryEventStream::default());
        let mut machine_events = events
            .subscribe(EntityKind::Machine, SubscriptionScope::All)
            .await
            .unwrap();
        let service = ServiceBuilder::new()
            .repository(repository)
            .events(events)
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                update_command_with_new_password(),
            )
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
                connectivity_changed: false,
                protocols_changed: BTreeSet::from([ProtocolKind::Ssh]),
                enrollment_ended: false,
            }
        );
    }
}

use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;

use super::authentication::sessions::SessionService;
use super::authorization::actor::Actor;
use super::authorization::authorize;
use super::event::{AuditKind, EventKind, UserSnapshot};
use super::ports::{UserFilter, UserRepository};
use super::use_cases::UserManagement;
use crate::application::audit::AuditService;
use crate::application::clock::Clock;
use crate::application::events::EventService;
use crate::application::identity::errors::{
    CreateUserError, DeleteUserError, GetUserError, ListUsersError, UpdateUserError,
};
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{
    ListQuery, Paginated, RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _,
};
use crate::domain::user::{DisplayName, Permission, Role, User, UserStatus, Username};

pub struct CreateUserCommand {
    pub username: Username,
    pub display_name: DisplayName,
    pub roles: BTreeSet<Role>,
}

pub struct UpdateUserCommand {
    pub id: uuid::Uuid,
    pub username: Option<Username>,
    pub display_name: Option<DisplayName>,
    pub roles: Option<BTreeSet<Role>>,
    pub status: Option<UserStatus>,
}

enum ChangedUser<'a> {
    Stored(&'a User),
    Removed(uuid::Uuid),
}

pub struct UserService<Tx> {
    users: Arc<dyn UserRepository<Tx>>,
    sessions: Arc<SessionService<Tx>>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    audit: Arc<AuditService<Tx>>,
    events: Arc<EventService>,
    clock: Arc<dyn Clock>,
}

impl<Tx: Send + Sync + 'static> UserService<Tx> {
    pub fn new(
        users: Arc<dyn UserRepository<Tx>>,
        sessions: Arc<SessionService<Tx>>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        audit: Arc<AuditService<Tx>>,
        events: Arc<EventService>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            users,
            sessions,
            unit_of_work,
            audit,
            events,
            clock,
        }
    }

    pub async fn list(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        query: &ListQuery,
    ) -> Result<Paginated<User>, ListUsersError> {
        actor.authorize(Permission::UserRead)?;

        Ok(self
            .users
            .list(ctx, query)
            .await
            .context("could not list users")?)
    }

    pub async fn get_by_id(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        id: uuid::Uuid,
    ) -> Result<User, GetUserError> {
        if !actor.is_user(id) {
            actor.authorize(Permission::UserRead)?;
        }

        match self.users.get_by_id(ctx, id).await {
            Ok(user) => Ok(user),
            Err(RepositoryError::NotFound) => Err(GetUserError::NotFound),
            Err(error) => Err(anyhow::Error::new(error)
                .context("could not get user by id")
                .into()),
        }
    }

    pub async fn get_by_username(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        username: &Username,
    ) -> Result<User, GetUserError> {
        if !actor
            .user()
            .is_some_and(|authenticated| authenticated.user.username == *username)
        {
            actor.authorize(Permission::UserRead)?;
        }

        match self.users.get_by_username(ctx, username).await {
            Ok(user) => Ok(user),
            Err(RepositoryError::NotFound) => Err(GetUserError::NotFound),
            Err(error) => Err(anyhow::Error::new(error)
                .context("could not get user by username")
                .into()),
        }
    }

    pub async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        cmd: CreateUserCommand,
    ) -> Result<User, CreateUserError> {
        actor.authorize(Permission::UserWrite)?;

        let user = User {
            id: uuid::Uuid::now_v7(),
            username: cmd.username,
            display_name: cmd.display_name,
            roles: cmd.roles,
            status: UserStatus::Active,
            created_at: self.clock.now(),
        };

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let user = match self.users.create(ctx, &user).await {
                    Ok(created) => created,
                    Err(RepositoryError::AlreadyExists) => {
                        return Err(CreateUserError::UsernameTaken);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not create user")
                            .into());
                    }
                };

                self.audit
                    .append(
                        ctx,
                        actor,
                        &user,
                        AuditKind::Created(UserSnapshot::from(&user)),
                    )
                    .await?;

                Ok(user)
            })
            .await
    }

    pub async fn update(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        cmd: UpdateUserCommand,
    ) -> Result<User, UpdateUserError> {
        let needs_privileges =
            cmd.username.is_some() || cmd.roles.is_some() || cmd.status.is_some();

        if needs_privileges || !actor.is_user(cmd.id) {
            actor.authorize(Permission::UserWrite)?;
        }

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let old = match self.users.get_by_id(ctx, cmd.id).await {
                    Ok(old) => old,
                    Err(RepositoryError::NotFound) => {
                        return Err(UpdateUserError::NotFound);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not get user by id")
                            .into());
                    }
                };

                let user = User {
                    id: old.id,
                    created_at: old.created_at,
                    username: cmd.username.unwrap_or(old.username.clone()),
                    display_name: cmd.display_name.unwrap_or(old.display_name.clone()),
                    roles: cmd.roles.unwrap_or(old.roles.clone()),
                    status: cmd.status.unwrap_or(old.status),
                };

                if !matches!(actor, Actor::System)
                    && self
                        .is_last_administrator(ctx, ChangedUser::Stored(&user))
                        .await?
                {
                    return Err(UpdateUserError::LastAdministrator);
                }

                let user = match self.users.update(ctx, &user).await {
                    Ok(updated) => updated,
                    Err(RepositoryError::AlreadyExists) => {
                        return Err(UpdateUserError::UsernameTaken);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not update user")
                            .into());
                    }
                };

                self.audit
                    .append(
                        ctx,
                        actor,
                        &user,
                        AuditKind::Updated {
                            before: UserSnapshot::from(&old),
                            after: UserSnapshot::from(&user),
                        },
                    )
                    .await?;

                if old.roles != user.roles {
                    self.events.publish(
                        user.id,
                        EventKind::RolesChanged {
                            roles: user.roles.clone(),
                        },
                    );
                }

                if old.status != user.status && user.status == UserStatus::Suspended {
                    self.sessions.delete_for_user(ctx, user.id, None).await?;
                }

                Ok(user)
            })
            .await
    }

    pub async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        actor: &Actor,
        id: uuid::Uuid,
    ) -> Result<User, DeleteUserError> {
        actor.authorize(Permission::UserWrite)?;

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                if self
                    .is_last_administrator(ctx, ChangedUser::Removed(id))
                    .await?
                {
                    return Err(DeleteUserError::LastAdministrator);
                }

                let user = match self.users.delete(ctx, id).await {
                    Ok(user) => user,
                    Err(RepositoryError::NotFound) => {
                        return Err(DeleteUserError::NotFound);
                    }
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("could not delete user")
                            .into());
                    }
                };

                self.audit
                    .append(ctx, actor, &user, AuditKind::Deleted)
                    .await?;
                self.sessions.delete_for_user(ctx, user.id, None).await?;

                Ok(user)
            })
            .await
    }

    async fn is_last_administrator(
        &self,
        ctx: &mut TxContext<Tx>,
        changed_user: ChangedUser<'_>,
    ) -> Result<bool, anyhow::Error> {
        let changed_id = match changed_user {
            ChangedUser::Stored(user) => {
                if authorize(user, Permission::UserWrite) {
                    return Ok(false);
                }

                user.id
            }
            ChangedUser::Removed(id) => id,
        };

        let other_administrators = UserFilter {
            status: Some(UserStatus::Active),
            roles: Some(Role::all_granting(Permission::UserWrite)),
            excluding: Some(changed_id),
        };

        Ok(self.users.count(ctx, &other_administrators).await? == 0)
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> UserManagement for UserService<Tx> {
    async fn list(
        &self,
        actor: &Actor,
        query: &ListQuery,
    ) -> Result<Paginated<User>, ListUsersError> {
        Self::list(self, &mut TxContext::none(), actor, query).await
    }

    async fn get_by_id(&self, actor: &Actor, id: uuid::Uuid) -> Result<User, GetUserError> {
        Self::get_by_id(self, &mut TxContext::none(), actor, id).await
    }

    async fn create(&self, actor: &Actor, cmd: CreateUserCommand) -> Result<User, CreateUserError> {
        Self::create(self, &mut TxContext::none(), actor, cmd).await
    }

    async fn update(&self, actor: &Actor, cmd: UpdateUserCommand) -> Result<User, UpdateUserError> {
        Self::update(self, &mut TxContext::none(), actor, cmd).await
    }

    async fn delete(&self, actor: &Actor, id: uuid::Uuid) -> Result<User, DeleteUserError> {
        Self::delete(self, &mut TxContext::none(), actor, id).await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::application::audit::{self, Entity};
    use crate::application::identity::authentication::ports::MockSessionRepository;
    use crate::application::identity::ports::MockUserRepository;
    use crate::application::repository::MockUnitOfWork;
    use crate::application::tokens::MockTokens;
    use crate::testing::events::RecordedAudit;
    use crate::testing::mocks::{transactions_that_commit, user_repository_returning};
    use crate::testing::{clock, events, identity, pagination};

    struct ServiceBuilder {
        users: MockUserRepository<()>,
        sessions: MockSessionRepository<()>,
        transactions: MockUnitOfWork,
        audit: Arc<RecordedAudit<()>>,
    }

    impl ServiceBuilder {
        fn new() -> Self {
            Self {
                users: MockUserRepository::new(),
                sessions: MockSessionRepository::new(),
                transactions: transactions_that_commit(),
                audit: events::recorded_audit(),
            }
        }

        fn audit(mut self, audit: Arc<RecordedAudit<()>>) -> Self {
            self.audit = audit;
            self
        }

        fn users(mut self, users: MockUserRepository<()>) -> Self {
            self.users = users;
            self
        }

        fn sessions(mut self, sessions: MockSessionRepository<()>) -> Self {
            self.sessions = sessions;
            self
        }

        fn build(self) -> UserService<()> {
            let audit = Arc::new(AuditService::new(
                self.audit,
                clock::clock_at(jiff::Timestamp::UNIX_EPOCH),
            ));
            let events = events::event_service();
            let transactions = Arc::new(self.transactions);

            UserService::new(
                Arc::new(self.users),
                Arc::new(SessionService::new(
                    Arc::new(self.sessions),
                    Arc::new(MockTokens::new()),
                    clock::clock_at(jiff::Timestamp::UNIX_EPOCH),
                    Arc::clone(&transactions) as Arc<_>,
                    Arc::clone(&events),
                    identity::session_policy(),
                )),
                transactions,
                audit,
                events,
                clock::clock_at(jiff::Timestamp::UNIX_EPOCH),
            )
        }
    }

    fn create_user_command() -> CreateUserCommand {
        CreateUserCommand {
            username: identity::username(),
            display_name: DisplayName::parse("Nicola").unwrap(),
            roles: BTreeSet::from([Role::Operator]),
        }
    }

    fn update_command_without_changes(id: uuid::Uuid) -> UpdateUserCommand {
        UpdateUserCommand {
            id,
            username: None,
            display_name: None,
            roles: None,
            status: None,
        }
    }

    /// The repository holds the user `id` and one other active administrator.
    fn user_repository_with_other_administrator(id: uuid::Uuid) -> MockUserRepository<()> {
        let mut users = user_repository_returning(move || identity::user_builder().id(id).build());
        users.expect_count().returning(|_, _| Ok(1));

        users
    }

    #[tokio::test]
    async fn list_without_permission_returns_denied() {
        let service = ServiceBuilder::new().build();

        let listed = service
            .list(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                &pagination::everything(),
            )
            .await;

        assert_matches!(listed.err(), Some(ListUsersError::Denied(_)));
    }

    #[tokio::test]
    async fn get_by_id_own_account_without_permission_succeeds() {
        let user = identity::user_builder().roles(&[]).build();
        let id = user.id;
        let actor = identity::actor_for(user);
        let service = ServiceBuilder::new()
            .users(user_repository_returning(move || {
                identity::user_builder().id(id).build()
            }))
            .build();

        let found = service
            .get_by_id(&mut TxContext::none(), &actor, id)
            .await
            .unwrap();

        assert_eq!(found.id, id);
    }

    #[tokio::test]
    async fn get_by_id_other_user_without_permission_returns_denied() {
        let service = ServiceBuilder::new().build();

        let found = service
            .get_by_id(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[Role::Operator]),
                uuid::Uuid::now_v7(),
            )
            .await;

        assert_matches!(found.err(), Some(GetUserError::Denied(_)));
    }

    #[tokio::test]
    async fn create_without_permission_returns_denied() {
        let service = ServiceBuilder::new().build();

        let created = service
            .create(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[Role::Operator]),
                create_user_command(),
            )
            .await;

        assert_matches!(created.err(), Some(CreateUserError::Denied(_)));
    }

    #[tokio::test]
    async fn create_with_taken_username_returns_username_taken() {
        let mut users = MockUserRepository::new();
        users
            .expect_create()
            .returning(|_, _| Err(RepositoryError::AlreadyExists));
        let service = ServiceBuilder::new().users(users).build();

        let created = service
            .create(
                &mut TxContext::none(),
                &Actor::System,
                create_user_command(),
            )
            .await;

        assert_matches!(created.err(), Some(CreateUserError::UsernameTaken));
    }

    #[tokio::test]
    async fn update_with_only_a_display_name_keeps_the_other_fields() {
        let id = uuid::Uuid::now_v7();
        let mut users = user_repository_with_other_administrator(id);
        users
            .expect_update()
            .once()
            .withf(|_, user| {
                &*user.display_name == "Renamed"
                    && user.username == identity::username()
                    && user.roles == identity::user_builder().build().roles
                    && user.status == UserStatus::Active
            })
            .returning(|_, user| Ok(user.clone()));
        let service = ServiceBuilder::new().users(users).build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateUserCommand {
                    display_name: Some(DisplayName::parse("Renamed").unwrap()),
                    ..update_command_without_changes(id)
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_to_taken_username_returns_username_taken() {
        let id = uuid::Uuid::now_v7();
        let mut users = user_repository_with_other_administrator(id);
        users
            .expect_update()
            .returning(|_, _| Err(RepositoryError::AlreadyExists));
        let service = ServiceBuilder::new().users(users).build();

        let updated = service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateUserCommand {
                    username: Some(Username::parse("taken").unwrap()),
                    ..update_command_without_changes(id)
                },
            )
            .await;

        assert_matches!(updated.err(), Some(UpdateUserError::UsernameTaken));
    }

    #[tokio::test]
    async fn update_own_display_name_without_permission_succeeds() {
        let user = identity::user_builder().roles(&[]).build();
        let id = user.id;
        let actor = identity::actor_for(user);
        let mut users = user_repository_with_other_administrator(id);
        users.expect_update().returning(|_, user| Ok(user.clone()));
        let service = ServiceBuilder::new().users(users).build();

        service
            .update(
                &mut TxContext::none(),
                &actor,
                UpdateUserCommand {
                    display_name: Some(DisplayName::parse("Renamed").unwrap()),
                    ..update_command_without_changes(id)
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_own_privileged_field_without_permission_returns_denied() {
        let user = identity::user_builder().roles(&[]).build();
        let id = user.id;
        let actor = identity::actor_for(user);
        let service = ServiceBuilder::new().build();

        for (field, privileged_change) in [
            (
                "roles",
                UpdateUserCommand {
                    roles: Some(BTreeSet::from([Role::Administrator])),
                    ..update_command_without_changes(id)
                },
            ),
            (
                "status",
                UpdateUserCommand {
                    status: Some(UserStatus::Active),
                    ..update_command_without_changes(id)
                },
            ),
            (
                "username",
                UpdateUserCommand {
                    username: Some(Username::parse("someone").unwrap()),
                    ..update_command_without_changes(id)
                },
            ),
        ] {
            assert_matches!(
                service
                    .update(&mut TxContext::none(), &actor, privileged_change)
                    .await
                    .err(),
                Some(UpdateUserError::Denied(_)),
                "{field}"
            );
        }
    }

    #[tokio::test]
    async fn update_other_user_display_name_without_permission_returns_denied() {
        let service = ServiceBuilder::new().build();

        let updated = service
            .update(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[]),
                UpdateUserCommand {
                    display_name: Some(DisplayName::parse("Renamed").unwrap()),
                    ..update_command_without_changes(uuid::Uuid::now_v7())
                },
            )
            .await;

        assert_matches!(updated.err(), Some(UpdateUserError::Denied(_)));
    }

    #[tokio::test]
    async fn update_removing_last_administrator_returns_last_administrator() {
        let id = uuid::Uuid::now_v7();
        let mut users = user_repository_returning(move || identity::user_builder().id(id).build());
        users.expect_count().returning(|_, _| Ok(0));
        let service = ServiceBuilder::new().users(users).build();

        for (case, removal) in [
            (
                "demoted",
                UpdateUserCommand {
                    roles: Some(BTreeSet::from([Role::Operator])),
                    ..update_command_without_changes(id)
                },
            ),
            (
                "suspended",
                UpdateUserCommand {
                    status: Some(UserStatus::Suspended),
                    ..update_command_without_changes(id)
                },
            ),
        ] {
            assert_matches!(
                service
                    .update(
                        &mut TxContext::none(),
                        &identity::actor_with_roles(&[Role::Administrator]),
                        removal
                    )
                    .await
                    .err(),
                Some(UpdateUserError::LastAdministrator),
                "{case}"
            );
        }
    }

    #[tokio::test]
    async fn update_by_the_system_removing_the_last_administrator_succeeds() {
        let id = uuid::Uuid::now_v7();
        let mut users = user_repository_returning(move || identity::user_builder().id(id).build());
        users.expect_update().returning(|_, user| Ok(user.clone()));
        let service = ServiceBuilder::new().users(users).build();

        let updated = service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateUserCommand {
                    roles: Some(BTreeSet::from([Role::Operator])),
                    ..update_command_without_changes(id)
                },
            )
            .await;

        assert_matches!(updated, Ok(_));
    }

    #[tokio::test]
    async fn update_demoting_administrator_while_another_remains_succeeds() {
        let id = uuid::Uuid::now_v7();
        let mut users = user_repository_with_other_administrator(id);
        users.expect_update().returning(|_, user| Ok(user.clone()));
        let service = ServiceBuilder::new().users(users).build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateUserCommand {
                    roles: Some(BTreeSet::from([Role::Operator])),
                    ..update_command_without_changes(id)
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_without_permission_returns_denied() {
        let service = ServiceBuilder::new().build();

        let deleted = service
            .delete(
                &mut TxContext::none(),
                &identity::actor_with_roles(&[Role::Operator]),
                uuid::Uuid::now_v7(),
            )
            .await;

        assert_matches!(deleted.err(), Some(DeleteUserError::Denied(_)));
    }

    #[tokio::test]
    async fn delete_last_administrator_returns_last_administrator() {
        let id = uuid::Uuid::now_v7();
        let mut users = user_repository_returning(move || identity::user_builder().id(id).build());
        users.expect_count().returning(|_, _| Ok(0));
        let service = ServiceBuilder::new().users(users).build();

        let deleted = service
            .delete(&mut TxContext::none(), &Actor::System, id)
            .await;

        assert_matches!(deleted.err(), Some(DeleteUserError::LastAdministrator));
    }

    #[tokio::test]
    async fn create_records_the_created_user() {
        let mut users = MockUserRepository::new();
        users.expect_create().returning(|_, user| Ok(user.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .users(users)
            .audit(Arc::clone(&audited))
            .build();

        let created = service
            .create(
                &mut TxContext::none(),
                &Actor::System,
                create_user_command(),
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
                AuditKind::Created(UserSnapshot::from(&created)).into()
            )]
        );
    }

    #[tokio::test]
    async fn update_records_the_state_before_and_after() {
        let id = uuid::Uuid::now_v7();
        let stored = identity::user_builder().id(id).build();
        let mut users = user_repository_with_other_administrator(id);
        users.expect_update().returning(|_, user| Ok(user.clone()));
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .users(users)
            .audit(Arc::clone(&audited))
            .build();

        service
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateUserCommand {
                    display_name: Some(DisplayName::parse("Renamed").unwrap()),
                    ..update_command_without_changes(id)
                },
            )
            .await
            .unwrap();

        assert_eq!(
            audited.recorded_kinds(),
            [AuditKind::Updated {
                before: UserSnapshot::from(&stored),
                after: UserSnapshot {
                    display_name: "Renamed".to_owned(),
                    ..UserSnapshot::from(&stored)
                },
            }
            .into()]
        );
    }

    #[tokio::test]
    async fn update_to_suspended_ends_the_sessions_and_records_the_suspension() {
        let id = uuid::Uuid::now_v7();
        let stored = identity::user_builder().id(id).build();
        let administrator = identity::user_builder()
            .username("administrator")
            .roles(&[Role::Administrator])
            .build();
        let mut users = user_repository_with_other_administrator(id);
        users.expect_update().returning(|_, user| Ok(user.clone()));
        let mut sessions = MockSessionRepository::new();
        sessions
            .expect_delete_for_user()
            .once()
            .withf(move |_, user_id, keep_newest, except| {
                *user_id == id && *keep_newest == 0 && except.is_none()
            })
            .returning(move |_, _, _, _| {
                Ok(vec![identity::session_for(
                    &identity::user_builder().id(id).build(),
                )])
            });
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .users(users)
            .sessions(sessions)
            .audit(Arc::clone(&audited))
            .build();

        service
            .update(
                &mut TxContext::none(),
                &identity::actor_for(administrator.clone()),
                UpdateUserCommand {
                    status: Some(UserStatus::Suspended),
                    ..update_command_without_changes(id)
                },
            )
            .await
            .unwrap();

        let by_administrator = audit::Actor::Entity(Entity::from(&administrator));
        assert_eq!(
            audited
                .recorded()
                .into_iter()
                .map(|entry| (entry.actor, entry.kind))
                .collect::<Vec<_>>(),
            [(
                by_administrator,
                AuditKind::Updated {
                    before: UserSnapshot::from(&stored),
                    after: UserSnapshot {
                        status: UserStatus::Suspended,
                        ..UserSnapshot::from(&stored)
                    },
                }
                .into()
            ),]
        );
    }

    #[tokio::test]
    async fn delete_records_the_deletion() {
        let id = uuid::Uuid::now_v7();
        let mut users = user_repository_with_other_administrator(id);
        users
            .expect_delete()
            .returning(move |_, _| Ok(identity::user_builder().id(id).build()));
        let mut sessions = MockSessionRepository::new();
        sessions
            .expect_delete_for_user()
            .returning(move |_, _, _, _| {
                Ok(vec![identity::session_for(
                    &identity::user_builder().id(id).build(),
                )])
            });
        let audited = events::recorded_audit();
        let service = ServiceBuilder::new()
            .users(users)
            .sessions(sessions)
            .audit(Arc::clone(&audited))
            .build();

        service
            .delete(&mut TxContext::none(), &Actor::System, id)
            .await
            .unwrap();

        assert_eq!(audited.recorded_kinds(), [AuditKind::Deleted.into()]);
    }
}

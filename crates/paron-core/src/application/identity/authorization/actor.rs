use super::authorize;
use crate::domain::user::{Permission, User};

#[derive(Debug, thiserror::Error)]
pub enum Denied {
    #[error("this action needs the {0} permission")]
    MissingPermission(Permission),

    #[error("you cannot perform this action")]
    NotAllowed,

    #[error("this action needs a recent authentication")]
    RecentAuthenticationRequired,
}

pub enum Actor {
    System,
    User(AuthenticatedUser),
}

#[derive(Clone)]
#[cfg_attr(test, derive(Debug))]
pub struct AuthenticatedUser {
    pub session_id: uuid::Uuid,
    pub recently_authenticated_until: jiff::Timestamp,
    pub user: User,
}

impl Actor {
    pub fn authorize(&self, required: Permission) -> Result<(), Denied> {
        let permitted = match self {
            Self::System => true,
            Self::User(authenticated) => authorize(&authenticated.user, required),
        };

        if permitted {
            return Ok(());
        }

        Err(Denied::MissingPermission(required))
    }

    pub fn require_recent_authentication(&self, now: jiff::Timestamp) -> Result<(), Denied> {
        match self {
            Self::User(authenticated) if now >= authenticated.recently_authenticated_until => {
                Err(Denied::RecentAuthenticationRequired)
            }
            Self::System | Self::User(_) => Ok(()),
        }
    }

    pub fn is_user(&self, user_id: uuid::Uuid) -> bool {
        match self {
            Self::System => false,
            Self::User(authenticated) => {
                authenticated.user.is_active() && authenticated.user.id == user_id
            }
        }
    }

    pub fn user(&self) -> Option<&AuthenticatedUser> {
        match self {
            Self::System => None,
            Self::User(user) => Some(user),
        }
    }
}

impl std::fmt::Display for Actor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => f.write_str("system"),
            Self::User(authenticated) => f.write_str(&authenticated.user.username),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::domain::user::{Role, UserStatus};
    use crate::testing::clock::timestamp;
    use crate::testing::identity;

    #[test]
    fn authorize_as_system_permits_every_permission() {
        for permission in Permission::ALL {
            assert_matches!(Actor::System.authorize(*permission), Ok(()), "{permission}");
        }
    }

    fn actor_recent_until(recently_authenticated_until: jiff::Timestamp) -> Actor {
        Actor::User(AuthenticatedUser {
            session_id: uuid::Uuid::now_v7(),
            recently_authenticated_until,
            user: identity::user_builder().build(),
        })
    }

    #[test]
    fn require_recent_authentication_before_the_end_of_the_window_succeeds() {
        let actor = actor_recent_until(timestamp(100));

        let required = actor.require_recent_authentication(timestamp(99));

        assert_matches!(required, Ok(()));
    }

    #[test]
    fn require_recent_authentication_at_the_end_of_the_window_returns_recent_authentication_required()
     {
        let actor = actor_recent_until(timestamp(100));

        let required = actor.require_recent_authentication(timestamp(100));

        assert_matches!(required, Err(Denied::RecentAuthenticationRequired));
    }

    #[test]
    fn require_recent_authentication_as_system_succeeds() {
        assert_matches!(
            Actor::System.require_recent_authentication(timestamp(100)),
            Ok(())
        );
    }

    #[test]
    fn authorize_as_user_with_granted_permission_succeeds() {
        let actor = identity::actor_with_roles(&[Role::Operator]);

        let authorized = actor.authorize(Permission::MachineRead);

        assert_matches!(authorized, Ok(()));
    }

    #[test]
    fn authorize_as_user_without_granted_permission_returns_denied() {
        let actor = identity::actor_with_roles(&[Role::Operator]);

        let authorized = actor.authorize(Permission::UserWrite);

        assert_matches!(
            authorized,
            Err(Denied::MissingPermission(Permission::UserWrite))
        );
    }

    #[test]
    fn authorize_as_suspended_user_returns_denied() {
        let actor = identity::actor_for(
            identity::user_builder()
                .roles(&[Role::Administrator])
                .status(UserStatus::Suspended)
                .build(),
        );

        for permission in Permission::ALL {
            assert_matches!(
                actor.authorize(*permission),
                Err(Denied::MissingPermission(_)),
                "{permission}"
            );
        }
    }

    #[test]
    fn is_user_with_the_own_id_returns_true() {
        let user = identity::user_builder().build();
        let user_id = user.id;

        let is_own = identity::actor_for(user).is_user(user_id);

        assert!(is_own);
    }

    #[test]
    fn is_user_with_another_id_returns_false() {
        let actor = identity::actor_with_roles(&[]);

        let is_own = actor.is_user(uuid::Uuid::now_v7());

        assert!(!is_own);
    }

    #[test]
    fn is_user_as_a_suspended_user_with_the_own_id_returns_false() {
        let user = identity::user_builder()
            .status(UserStatus::Suspended)
            .build();
        let user_id = user.id;

        let is_own = identity::actor_for(user).is_user(user_id);

        assert!(!is_own);
    }

    #[test]
    fn is_user_as_the_system_returns_false() {
        let is_own = Actor::System.is_user(uuid::Uuid::now_v7());

        assert!(!is_own);
    }
}

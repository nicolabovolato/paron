use std::collections::BTreeSet;
use std::time::Duration;

use secrecy::SecretString;

use crate::application::identity::authentication::methods::password::{Password, PasswordPhc};
use crate::application::identity::authentication::sessions::Session;
use crate::application::identity::authentication::sessions::SessionPolicy;
use crate::application::identity::authorization::actor::{Actor, AuthenticatedUser};
use crate::domain::token_mac::TokenMac;
use crate::domain::user::{DisplayName, Role, User, UserStatus, Username};

pub const USERNAME: &str = "nicola";
pub const PASSWORD: &str = "correct horse battery staple";
pub const PASSWORD_PHC: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHR2YWx1ZQ$3Vh0YkFDR0hJSktMTU5PUFFSU1RVVldY";

pub fn password() -> Password {
    Password::parse(SecretString::from(PASSWORD)).unwrap()
}

pub fn password_phc() -> PasswordPhc {
    PasswordPhc::parse(PASSWORD_PHC.to_owned()).unwrap()
}

pub fn session_policy() -> SessionPolicy {
    SessionPolicy {
        lifetime: Duration::from_hours(24 * 7),
        max_idle_timeout: Duration::from_hours(12),
        touch_interval: Duration::from_mins(5),
        expired_sessions_per_delete_batch: 1000,
        recent_authentication_window: Duration::from_mins(15),
        max_sessions_per_user: 3,
    }
}

pub fn username() -> Username {
    Username::parse(USERNAME).unwrap()
}

pub fn user_builder() -> UserBuilder {
    UserBuilder {
        user: User {
            id: uuid::Uuid::now_v7(),
            username: username(),
            display_name: DisplayName::parse("Nicola").unwrap(),
            roles: BTreeSet::from([Role::Administrator]),
            status: UserStatus::Active,
            created_at: jiff::Timestamp::UNIX_EPOCH,
        },
    }
}

pub struct UserBuilder {
    user: User,
}

impl UserBuilder {
    pub fn id(mut self, id: uuid::Uuid) -> Self {
        self.user.id = id;
        self
    }

    pub fn username(mut self, username: &str) -> Self {
        self.user.username = Username::parse(username).unwrap();
        self
    }

    pub fn display_name(mut self, display_name: &str) -> Self {
        self.user.display_name = DisplayName::parse(display_name).unwrap();
        self
    }

    pub fn roles(mut self, roles: &[Role]) -> Self {
        self.user.roles = roles.iter().copied().collect();
        self
    }

    pub fn status(mut self, status: UserStatus) -> Self {
        self.user.status = status;
        self
    }

    pub fn build(self) -> User {
        self.user
    }
}

pub fn actor_with_roles(roles: &[Role]) -> Actor {
    actor_for(user_builder().roles(roles).build())
}

pub fn actor_for(user: User) -> Actor {
    Actor::User(AuthenticatedUser {
        session_id: uuid::Uuid::now_v7(),
        recently_authenticated_until: jiff::Timestamp::MAX,
        user,
    })
}

pub fn session_for(user: &User) -> Session {
    Session {
        id: uuid::Uuid::now_v7(),
        user_id: user.id,
        mac: TokenMac::from(vec![1; 32]),
        issued_at: jiff::Timestamp::UNIX_EPOCH,
        recently_authenticated_until: jiff::Timestamp::UNIX_EPOCH,
        last_used_at: jiff::Timestamp::UNIX_EPOCH,
        expires_at: jiff::Timestamp::UNIX_EPOCH + jiff::SignedDuration::from_hours(24),
    }
}

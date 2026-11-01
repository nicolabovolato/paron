pub mod failed_sign_in_attempts;
pub mod passkeys;
pub mod passwords;
pub mod pending_account_recoveries;
pub mod pending_sign_ins;
pub mod recovery_codes;
pub mod sessions;
pub mod totps;
pub mod users;

use std::collections::BTreeSet;

use crate::domain::user::{DisplayName, Role, User, UserStatus, Username};

type Error = Box<dyn std::error::Error + Send + Sync>;

extract_discriminant! {
    #[derive(sqlx::Type, Clone, Copy)]
    #[sqlx(rename_all = "snake_case")]
    enum UserStatusKind from UserStatus { Active, Suspended }
}

impl From<UserStatusKind> for UserStatus {
    fn from(kind: UserStatusKind) -> Self {
        match kind {
            UserStatusKind::Active => Self::Active,
            UserStatusKind::Suspended => Self::Suspended,
        }
    }
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: uuid::Uuid,
    username: String,
    display_name: String,
    status: UserStatusKind,
    created_at_unix_ms: i64,

    roles: sqlx::types::Json<Vec<String>>,
}

impl UserRow {
    fn from_entity(user: &User) -> Self {
        Self {
            id: user.id,
            username: user.username.to_string(),
            display_name: user.display_name.to_string(),
            status: UserStatusKind::from(&user.status),
            created_at_unix_ms: user.created_at.as_millisecond(),
            roles: sqlx::types::Json(
                user.roles
                    .iter()
                    .map(|role| role.as_str().to_owned())
                    .collect(),
            ),
        }
    }

    fn try_into_entity(self) -> Result<User, Error> {
        let roles = self
            .roles
            .0
            .iter()
            .map(|role| role.parse())
            .collect::<Result<BTreeSet<Role>, _>>()?;

        Ok(User {
            id: self.id,
            username: Username::parse(&self.username)?,
            display_name: DisplayName::parse(self.display_name)?,
            roles,
            status: self.status.into(),
            created_at: jiff::Timestamp::from_millisecond(self.created_at_unix_ms)?,
        })
    }
}

use std::collections::BTreeSet;

use super::text::LimitedText;

string_enum! {
    pub enum Permission {
        MachineRead => "machine:read",
        MachineWrite => "machine:write",
        MachineDelete => "machine:delete",
        MachineOperate => "machine:operate",
        MachineEnroll => "machine:enroll",
        UserRead => "user:read",
        UserWrite => "user:write",
    }
}

string_enum! {
    pub enum Role {
        Administrator => "administrator",
        Operator => "operator",
    }
}

impl Role {
    pub fn all_granting(permission: Permission) -> std::collections::BTreeSet<Self> {
        Self::ALL
            .iter()
            .copied()
            .filter(|role| role.permissions().contains(&permission))
            .collect()
    }

    pub const fn permissions(self) -> &'static [Permission] {
        match self {
            Self::Administrator => Permission::ALL,
            Self::Operator => &[
                Permission::MachineRead,
                Permission::MachineWrite,
                Permission::MachineDelete,
                Permission::MachineOperate,
                Permission::MachineEnroll,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UserStatus {
    Active,
    Suspended,
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidUsername {
    #[error("a username is between {min} and {max} characters", min = Username::MIN, max = Username::MAX)]
    Length,

    #[error(
        "a username holds only lowercase letters, digits, and {}",
        Username::separators()
    )]
    Charset,

    #[error(
        "a username starts and ends with a letter or digit, and never places two of {} in a row",
        Username::separators()
    )]
    Shape,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, derive_more::Deref)]
#[deref(forward)]
pub struct Username(String);

impl Username {
    pub const MIN: usize = 3;
    pub const MAX: usize = 32;

    pub const SEPARATORS: [char; 3] = ['.', '-', '_'];

    fn separators() -> String {
        Self::SEPARATORS.map(|c| format!("'{c}'")).join(", ")
    }

    pub fn parse(raw: &str) -> Result<Self, InvalidUsername> {
        let normalised = raw.to_lowercase();

        if !(Self::MIN..=Self::MAX).contains(&normalised.chars().count()) {
            return Err(InvalidUsername::Length);
        }

        let is_separator = |c: char| Self::SEPARATORS.contains(&c);

        if !normalised
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || is_separator(c))
        {
            return Err(InvalidUsername::Charset);
        }

        let touches_an_edge =
            normalised.starts_with(is_separator) || normalised.ends_with(is_separator);
        let doubles_up = normalised
            .chars()
            .zip(normalised.chars().skip(1))
            .any(|(left, right)| is_separator(left) && is_separator(right));

        if touches_an_edge || doubles_up {
            return Err(InvalidUsername::Shape);
        }

        Ok(Self(normalised))
    }
}

pub type DisplayName = LimitedText<1, 64>;

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct User {
    pub id: uuid::Uuid,
    pub username: Username,
    pub display_name: DisplayName,
    pub roles: BTreeSet<Role>,
    pub status: UserStatus,
    pub created_at: jiff::Timestamp,
}

impl User {
    pub fn permissions(&self) -> BTreeSet<Permission> {
        self.roles
            .iter()
            .flat_map(|role| role.permissions())
            .copied()
            .collect()
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, UserStatus::Active)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::identity;

    #[test]
    fn parse_lowercases() {
        let username = Username::parse("Nicola").unwrap();

        assert_eq!(&*username, "nicola");
    }

    #[test]
    fn parse_within_the_rules_succeeds() {
        for valid_username in [
            "a".repeat(Username::MIN),
            "a".repeat(Username::MAX),
            "nic.ola_2-x".to_owned(),
        ] {
            assert_matches!(
                Username::parse(&valid_username),
                Ok(_),
                "{valid_username:?}"
            );
        }
    }

    #[test]
    fn parse_of_wrong_length_returns_length_error() {
        for invalid_username in ["a".repeat(Username::MIN - 1), "a".repeat(Username::MAX + 1)] {
            assert_matches!(
                Username::parse(&invalid_username),
                Err(InvalidUsername::Length),
                "{invalid_username:?}"
            );
        }
    }

    #[test]
    fn parse_with_forbidden_character_returns_charset_error() {
        for invalid_username in ["nic ola", " nicola", "nicolà", "nic@ola"] {
            assert_matches!(
                Username::parse(invalid_username),
                Err(InvalidUsername::Charset),
                "{invalid_username:?}"
            );
        }
    }

    #[test]
    fn parse_with_misplaced_separator_returns_shape_error() {
        for invalid_username in ["-nicola", "nicola.", "...", "nic._ola"] {
            assert_matches!(
                Username::parse(invalid_username),
                Err(InvalidUsername::Shape),
                "{invalid_username:?}"
            );
        }
    }

    #[test]
    fn permissions_with_several_roles_returns_their_union() {
        let user = identity::user_builder()
            .roles(&[Role::Operator, Role::Administrator])
            .build();

        let permissions = user.permissions();

        assert_eq!(
            permissions,
            Role::Administrator.permissions().iter().copied().collect()
        );
    }

    #[test]
    fn permissions_of_administrator_holds_every_permission() {
        assert_eq!(Role::Administrator.permissions(), Permission::ALL);
    }

    #[test]
    fn all_granting_returns_only_the_roles_that_hold_the_permission() {
        assert_eq!(
            Role::all_granting(Permission::UserWrite),
            BTreeSet::from([Role::Administrator])
        );
        assert_eq!(
            Role::all_granting(Permission::MachineWrite),
            BTreeSet::from([Role::Administrator, Role::Operator])
        );
        assert_eq!(
            Role::all_granting(Permission::MachineRead),
            Role::ALL.iter().copied().collect()
        );
    }
}

pub mod actor;

use crate::domain::user::{Permission, User};

/// Only `Actor` and the last-administrator check of the user service decide a permission on a
/// raw `User`.
pub(super) fn authorize(user: &User, required: Permission) -> bool {
    user.is_active() && user.permissions().contains(&required)
}

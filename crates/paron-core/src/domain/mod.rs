#[derive(Debug, thiserror::Error)]
#[error("{value:?} is not a recognised {kind}")]
pub struct UnknownStringEnumValue {
    pub(crate) kind: &'static str,
    pub(crate) value: String,
}

pub mod entity;
pub mod machine;
pub mod text;
pub mod token_mac;
pub mod user;

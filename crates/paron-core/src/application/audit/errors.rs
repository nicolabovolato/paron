#[derive(Debug, thiserror::Error)]
pub enum ListAuditEntriesError {
    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

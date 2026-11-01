/// What can go wrong, split as STM-8 requires: refusal by policy, absence of a connection, and
/// transport or environment failure.
#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    /// A policy declined — an unrecognised claim, the wrong private key, a bad token, a selector
    /// the agent does not serve. Retrying changes nothing.
    #[error("refused: a policy declined this claim, token or selector")]
    Refused,

    #[error("not connected: no tunnel is currently open to that agent")]
    NotConnected,

    /// The carrier or the environment failed. Reached from inside a live tunnel too: a stream
    /// that cannot be opened on a healthy connection lands here rather than under `Refused`.
    #[error("the tunnel failed: {0}")]
    Unexpected(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl TunnelError {
    /// Wrap an underlying error as [`TunnelError::Unexpected`].
    ///
    /// You can build that variant yourself — it is public and so is its payload. This exists
    /// because it does the boxing for you and, more usefully, because it can be handed to
    /// `map_err` by name:
    ///
    /// ```
    /// use paron_tunnel_protocol::TunnelError;
    /// use std::net::{SocketAddr, TcpListener};
    ///
    /// fn address(listener: &TcpListener) -> Result<SocketAddr, TunnelError> {
    ///     listener.local_addr().map_err(TunnelError::unexpected)
    /// }
    /// ```
    ///
    /// against the closure you would otherwise write out at every call site.
    pub fn unexpected(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Unexpected(Box::new(error))
    }

    /// Our own bug, or a configuration that cannot work — a message with nothing underneath it.
    ///
    /// Unlike [`unexpected`](TunnelError::unexpected) this one is not redundant with the variant:
    /// [`Fault`] is private, so nothing outside this crate can produce one. That is deliberate. A
    /// `TunnelError` carrying a `Fault` means the fault is ours; if callers could mint them, the
    /// distinction would be worth nothing.
    pub(crate) fn fault(what: &'static str) -> Self {
        Self::Unexpected(Box::new(Fault(what)))
    }
}

/// A message with no underlying error: something this crate got wrong, or was configured to do
/// and cannot.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct Fault(&'static str);

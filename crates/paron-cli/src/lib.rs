#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[cfg(target_env = "musl")]
use musl_nss_mdns as _;

mod bootstrap;
pub mod cli;
pub mod daemon_manager;
pub mod lifecycle;
mod migration;
pub mod platform;
mod server;
#[cfg(test)]
mod testing;
mod tunnel;
pub mod ui;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

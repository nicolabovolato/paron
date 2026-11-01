#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[macro_use]
mod macros;

pub mod application;
pub mod domain;
pub mod infrastructure;

#[cfg(test)]
mod testing;

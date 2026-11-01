pub use clap_extra_macros::{ArgNames, ArgValues};

/// The two names one clap argument goes by.
///
/// A command line spells it `--flag`. A service definition sets `env`, because that is the only
/// way systemd and launchd pass a setting to the program they start. `None` for an argument that
/// only a command line gives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ArgName {
    pub flag: &'static str,
    pub env: Option<&'static str>,
}

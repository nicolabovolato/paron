use std::collections::BTreeMap;

mod capability;
pub mod ssh;
pub mod tunnel;

use super::text::LimitedText;
use capability::Capability;
pub use capability::{Capabilities, CapabilityUnavailable, ConnectionTest, Provider, Terminal};

pub type MachineName = LimitedText<1, 64>;

#[derive(Clone)]
pub struct Machine {
    pub id: uuid::Uuid,
    pub name: MachineName,
    pub connectivity: Connectivity,
    pub protocols: Protocols,
}

impl Machine {
    pub fn capabilities(&self) -> Capabilities {
        fn available<C>(
            providers: BTreeMap<Provider, Result<C, CapabilityUnavailable>>,
        ) -> Vec<Provider> {
            providers
                .into_iter()
                .filter_map(|(provider, found)| found.is_ok().then_some(provider))
                .collect()
        }

        Capabilities {
            connection_test: available(ConnectionTest::from_machine(self)),
            terminal: available(Terminal::from_machine(self)),
        }
    }

    pub fn capability<C: Capability>(
        &self,
        provider: Provider,
    ) -> Result<C, CapabilityUnavailable> {
        C::from_machine(self)
            .remove(&provider)
            .unwrap_or(Err(CapabilityUnavailable::Unsupported(provider)))
    }
}

#[derive(Clone)]
pub enum Connectivity {
    Direct { host: Host },
    Tunneled(tunnel::Enrollment),
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidHost {
    #[error("expected a hostname or IP address")]
    Malformed,

    #[error("expected a hostname of at most {} characters", Host::MAX)]
    Length,
}

#[derive(Debug, Clone, PartialEq, Eq, derive_more::Deref)]
pub struct Host(url::Host);

impl Host {
    /// The length of a name in DNS (RFC 1035 3.1), with the length octets.
    pub const MAX: usize = 255;

    pub fn parse(raw: &str) -> Result<Self, InvalidHost> {
        let host = url::Host::parse(raw).map_err(|_| InvalidHost::Malformed)?;

        if host.to_string().len() > Self::MAX {
            return Err(InvalidHost::Length);
        }

        Ok(Self(host))
    }
}

impl std::fmt::Display for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Default)]
pub struct Protocols {
    pub ssh: Option<ssh::Access>,
}

string_enum! {
    pub enum ProtocolKind {
        Ssh => "ssh",
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::machine;

    const SSH: Provider = Provider::Protocol(ProtocolKind::Ssh);

    #[test]
    fn capability_via_ssh_on_machine_with_ssh_returns_its_access() {
        let machine = machine::direct_machine();

        let Ok(Terminal::Ssh(access)) = machine.capability::<Terminal>(SSH) else {
            panic!("expected the terminal via ssh");
        };
        assert_eq!(&*access.user, machine::SSH_USER);
    }

    #[test]
    fn capability_via_ssh_on_machine_without_ssh_returns_missing() {
        let machine = machine::machine_builder().without_ssh().build();

        assert_matches!(
            machine.capability::<Terminal>(SSH).err(),
            Some(CapabilityUnavailable::Missing(ProtocolKind::Ssh))
        );
    }

    #[test]
    fn capabilities_of_machine_with_ssh_lists_ssh_for_each_capability() {
        let Capabilities {
            connection_test,
            terminal,
        } = machine::direct_machine().capabilities();

        assert_eq!(connection_test, [SSH]);
        assert_eq!(terminal, [SSH]);
    }

    #[test]
    fn capabilities_of_machine_without_ssh_lists_none() {
        let Capabilities {
            connection_test,
            terminal,
        } = machine::machine_builder()
            .without_ssh()
            .build()
            .capabilities();

        assert_eq!(connection_test, []);
        assert_eq!(terminal, []);
    }
}

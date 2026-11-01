use rustls::pki_types::DnsName;

/// DNS-ID or IP-ID (RFC 9525 §6.4).
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Identifier {
    Dns(DnsName<'static>),
    Ip(std::net::IpAddr),
}

impl std::fmt::Display for Identifier {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dns(name) => out.write_str(name.as_ref()),
            Self::Ip(address) => write!(out, "{address}"),
        }
    }
}

impl std::fmt::Debug for Identifier {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{self}")
    }
}

impl Identifier {
    pub fn is_local(&self) -> bool {
        match self {
            Self::Ip(address) => address.is_loopback(),
            Self::Dns(name) => name
                .as_ref()
                .rsplit('.')
                .next()
                .is_some_and(|last| last.eq_ignore_ascii_case("localhost")),
        }
    }

    pub fn is_ip_address(&self) -> bool {
        matches!(self, Self::Ip(_))
    }

    /// `.local` is reserved for link-local multicast, so no authority resolves one (RFC 6762).
    pub fn is_mdns(&self) -> bool {
        matches!(self, Self::Dns(name) if name
            .as_ref()
            .rsplit_once('.')
            .is_some_and(|(_, last)| last.eq_ignore_ascii_case("local")))
    }
}

impl TryFrom<&url::Host> for Identifier {
    type Error = String;

    fn try_from(host: &url::Host) -> std::result::Result<Self, Self::Error> {
        match host {
            url::Host::Domain(domain) => domain.parse(),
            url::Host::Ipv4(address) => Ok(Self::Ip((*address).into())),
            url::Host::Ipv6(address) => Ok(Self::Ip((*address).into())),
        }
    }
}

impl std::str::FromStr for Identifier {
    type Err = String;

    fn from_str(raw: &str) -> std::result::Result<Self, Self::Err> {
        let name = raw.trim();

        if name.is_empty() {
            return Err("a certificate name cannot be blank".to_owned());
        }

        // `DnsName` refuses these too.
        if name.contains('*') {
            return Err("a certificate name cannot be a wildcard".to_owned());
        }

        // SNI carries a name without its trailing dot (RFC 6066 §3), so no client asks for one.
        let name = name.strip_suffix('.').unwrap_or(name);

        // Addresses first: `url::Host` reads a bare `::1` as a domain and refuses it.
        if let Ok(address) = name.parse::<std::net::IpAddr>() {
            return Ok(Self::Ip(address));
        }

        let normalised = match url::Host::parse(name) {
            Ok(url::Host::Domain(domain)) => domain,
            Ok(_) => name.to_owned(),
            Err(reason) => return Err(format!("{name:?} is not a domain or an address: {reason}")),
        };

        let dns = DnsName::try_from(normalised)
            .map_err(|_| format!("{name:?} is not a name a client could connect to"))?;

        Ok(Self::Dns(dns))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_local_is_true_for_loopback_and_localhost_only() {
        for local_name in ["localhost", "paron.localhost", "127.0.0.1", "::1"] {
            assert!(
                local_name.parse::<Identifier>().unwrap().is_local(),
                "{local_name}"
            );
        }

        for non_local_name in [
            "paron.example.com",
            "paron.local",
            "10.0.0.1",
            "2001:db8::1",
        ] {
            assert!(
                !non_local_name.parse::<Identifier>().unwrap().is_local(),
                "{non_local_name}"
            );
        }
    }

    #[test]
    fn parse_valid_name_returns_its_normalised_form() {
        for (name, normalised_name, is_ip_address) in [
            ("paron.example.com", "paron.example.com", false),
            ("localhost", "localhost", false),
            ("xn--80ak6aa92e.com", "xn--80ak6aa92e.com", false),
            ("пример.com", "xn--e1afmkfd.com", false),
            ("EXAMPLE.COM", "example.com", false),
            ("example.com.", "example.com", false),
            ("127.0.0.1", "127.0.0.1", true),
            ("::1", "::1", true),
        ] {
            let identifier = name.parse::<Identifier>().unwrap();

            assert_eq!(identifier.to_string(), normalised_name, "{name:?}");
            assert_eq!(identifier.is_ip_address(), is_ip_address, "{name:?}");
        }
    }

    #[test]
    fn parse_malformed_name_returns_error() {
        for invalid_name in [
            "",
            "   ",
            "a b.com",
            "-leading.com",
            "trailing-.com",
            "double..dot",
        ] {
            assert!(
                invalid_name.parse::<Identifier>().is_err(),
                "{invalid_name:?}"
            );
        }
    }

    #[test]
    fn is_mdns_is_true_for_dot_local_names_only() {
        for mdns_name in ["paron.local", "a.b.LOCAL"] {
            assert!(
                mdns_name.parse::<Identifier>().unwrap().is_mdns(),
                "{mdns_name}"
            );
        }

        for other_name in ["paron.example.com", "local", "localhost", "10.0.0.1"] {
            assert!(
                !other_name.parse::<Identifier>().unwrap().is_mdns(),
                "{other_name}"
            );
        }
    }

    #[test]
    fn parse_wildcard_returns_error() {
        for wildcard_name in ["*.example.com", "*.10.0.0.5", "a.*.example.com", "*"] {
            let error = wildcard_name
                .parse::<Identifier>()
                .expect_err("a wildcard is refused");

            assert!(error.contains("wildcard"), "{wildcard_name}: {error}");
        }
    }
}

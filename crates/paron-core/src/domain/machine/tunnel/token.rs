use derive_more::From;
use paron_tunnel_protocol::{AgentId, BrokerAddress, Fingerprint};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

const NO_SECRET: &str = "the token must have secret=<hex>";

#[derive(Clone)]
pub struct ServerEndpoint {
    pub address: BrokerAddress,
    pub machine: AgentId,
    pub fingerprint: Fingerprint,
}

/// An endpoint and the single-use secret that binds a keypair to the machine (ptp/1 RFC §5.2).
pub struct Token {
    pub server_endpoint: ServerEndpoint,
    pub secret: SecretString,
}

#[derive(Debug, thiserror::Error, From)]
#[error("{0}")]
#[from(forward)]
pub struct InvalidToken(String);

impl ServerEndpoint {
    fn parse(input: &str) -> Result<(Self, Option<SecretString>), InvalidToken> {
        let url = Url::parse(input).map_err(|_| "the token must be a valid url")?;

        if url.scheme() != "paron" {
            return Err("the token scheme must be paron://".into());
        }

        if url.fragment().is_some() {
            return Err("the token must have no # character".into());
        }

        let mut fingerprint = None;
        let mut secret = None;

        for (key, value) in url.query_pairs() {
            match key.as_ref() {
                "fingerprint" if fingerprint.is_none() => fingerprint = Some(value.into_owned()),
                "secret" if secret.is_none() => {
                    if value.is_empty() {
                        return Err(NO_SECRET.into());
                    }

                    secret = Some(SecretString::from(value.into_owned()));
                }
                "fingerprint" | "secret" => {
                    return Err(format!("the token must have only one {key}").into());
                }
                other => return Err(format!("{other} is not a token parameter").into()),
            }
        }

        let fingerprint = fingerprint.ok_or("the token must have fingerprint=sha256:<hex>")?;
        let authority = url.authority();
        let machine = url.path().trim_start_matches('/');

        Ok((
            Self {
                address: authority
                    .parse()
                    .map_err(|_| format!("{authority} is not a server address"))?,
                machine: AgentId::parse(machine)
                    .ok_or_else(|| format!("{machine} is not a machine id"))?,
                fingerprint: Fingerprint::parse(&fingerprint).map_err(|err| err.to_string())?,
            },
            secret,
        ))
    }
}

impl std::str::FromStr for ServerEndpoint {
    type Err = InvalidToken;

    fn from_str(endpoint: &str) -> Result<Self, InvalidToken> {
        let (endpoint, _secret) = Self::parse(endpoint)?;

        Ok(endpoint)
    }
}

impl std::fmt::Display for ServerEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "paron://{}/{}?fingerprint={}",
            self.address, self.machine, self.fingerprint
        )
    }
}

impl Token {
    pub fn url(&self) -> SecretString {
        SecretString::from(format!(
            "{}&secret={}",
            self.server_endpoint,
            self.secret.expose_secret()
        ))
    }
}

impl std::str::FromStr for Token {
    type Err = InvalidToken;

    fn from_str(token: &str) -> Result<Self, InvalidToken> {
        let (server_endpoint, secret) = ServerEndpoint::parse(token)?;

        Ok(Self {
            server_endpoint,
            secret: secret.ok_or(NO_SECRET)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACHINE_ID: &str = "0198f0a0-0000-7000-8000-000000000001";
    const FINGERPRINT: &str =
        "sha256:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const SECRET: &str = "s3cret";

    fn token_url(query: &str) -> String {
        format!("paron://paron.example:8443/{MACHINE_ID}?{query}")
    }

    fn token() -> Token {
        token_url(&format!("fingerprint={FINGERPRINT}&secret={SECRET}"))
            .parse()
            .unwrap()
    }

    #[test]
    fn parse_token_returns_every_part() {
        let url = token_url(&format!("fingerprint={FINGERPRINT}&secret={SECRET}"));

        let token: Token = url.parse().unwrap();

        assert_eq!(
            token.server_endpoint.address.to_string(),
            "paron.example:8443"
        );
        assert_eq!(token.server_endpoint.machine.to_string(), MACHINE_ID);
        assert_eq!(token.server_endpoint.fingerprint.to_string(), FINGERPRINT);
        assert_eq!(token.secret.expose_secret(), SECRET);
    }

    #[test]
    fn parse_endpoint_without_secret_succeeds() {
        let url_without_secret = token_url(&format!("fingerprint={FINGERPRINT}"));

        url_without_secret.parse::<ServerEndpoint>().unwrap();
    }

    #[test]
    fn parse_token_without_secret_returns_error() {
        let url_without_secret = token_url(&format!("fingerprint={FINGERPRINT}"));

        let error = url_without_secret.parse::<Token>().err().unwrap();

        assert!(error.to_string().contains("secret=<hex>"), "{error}");
    }

    #[test]
    fn parse_address_without_port_returns_error_naming_address() {
        let url =
            format!("paron://paron.example/{MACHINE_ID}?fingerprint={FINGERPRINT}&secret={SECRET}");

        let error = url.parse::<Token>().err().unwrap();

        assert!(error.to_string().contains("paron.example"), "{error}");
    }

    #[test]
    fn parse_machine_that_is_not_uuid_returns_error() {
        for invalid_url in [
            format!("paron://paron.example:8443/{MACHINE_ID}/extra?fingerprint={FINGERPRINT}"),
            format!("paron://paron.example:8443/not-a-uuid?fingerprint={FINGERPRINT}"),
            format!("paron://paron.example:8443/{{{MACHINE_ID}}}?fingerprint={FINGERPRINT}"),
            format!("paron://paron.example:8443/?fingerprint={FINGERPRINT}"),
        ] {
            assert!(
                invalid_url.parse::<ServerEndpoint>().is_err(),
                "{invalid_url} was accepted"
            );
        }
    }

    #[test]
    fn parse_without_fingerprint_returns_error() {
        let error = token_url(&format!("secret={SECRET}"))
            .parse::<Token>()
            .err()
            .unwrap();

        assert!(error.to_string().contains("fingerprint="), "{error}");
    }

    #[test]
    fn parse_unknown_parameter_returns_error_naming_parameter() {
        let error = token_url(&format!("fingerprint={FINGERPRINT}&fingerprnt=x"))
            .parse::<Token>()
            .err()
            .unwrap();

        assert!(error.to_string().contains("fingerprnt"), "{error}");
    }

    #[test]
    fn parse_repeated_parameter_returns_error() {
        for invalid_query in [
            format!("fingerprint={FINGERPRINT}&fingerprint={FINGERPRINT}"),
            format!("fingerprint={FINGERPRINT}&secret=one&secret=two"),
        ] {
            assert!(
                token_url(&invalid_query).parse::<Token>().is_err(),
                "{invalid_query} was accepted"
            );
        }
    }

    #[test]
    fn parse_fragment_returns_error() {
        for invalid_url in [
            format!("paron://paron.example:8443/{MACHINE_ID}.{SECRET}#{FINGERPRINT}"),
            format!("paron://paron.example:8443/{MACHINE_ID}?fingerprint={FINGERPRINT}#extra"),
        ] {
            assert!(
                invalid_url.parse::<ServerEndpoint>().is_err(),
                "{invalid_url} was accepted"
            );
        }
    }

    #[test]
    fn parse_malformed_token_returns_error() {
        for invalid_url in [
            format!("https://paron.example/{MACHINE_ID}?fingerprint={FINGERPRINT}"),
            "paron://paron.example:8443".to_owned(),
            token_url(&format!("fingerprint={FINGERPRINT}&secret=")),
            token_url("fingerprint=sha256:00"),
            "not a url at all".to_owned(),
        ] {
            assert!(
                invalid_url.parse::<Token>().is_err(),
                "{invalid_url} was accepted"
            );
        }
    }

    #[test]
    fn round_trip_token_through_url_keeps_every_part() {
        let url = token().url();

        let token_from_url: Token = url.expose_secret().parse().unwrap();

        assert_eq!(token_from_url.url().expose_secret(), url.expose_secret());
        assert_eq!(token_from_url.secret.expose_secret(), SECRET);
    }

    #[test]
    fn round_trip_endpoint_keeps_the_text() {
        let endpoint_text = token().server_endpoint.to_string();

        let endpoint: ServerEndpoint = endpoint_text.parse().unwrap();

        assert_eq!(endpoint.to_string(), endpoint_text);
    }

    #[test]
    fn fmt_endpoint_omits_the_secret() {
        let endpoint = token().server_endpoint;

        let endpoint_text = endpoint.to_string();

        assert!(!endpoint_text.contains(SECRET), "{endpoint_text}");
    }
}

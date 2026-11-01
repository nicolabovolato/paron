use std::net::SocketAddr;
use std::path::PathBuf;

use paron_core::domain::user::Username;
use paron_tunnel_protocol::{BrokerAddress, TunnelError};

use crate::Result;
use crate::cli::{
    ConfirmArgs, ConfirmArgsNames, InstallArgs, InstallArgsNames, PromptArgs, PromptArgsNames,
};
use crate::server::{bootstrap, run, users};

#[derive(clap::Subcommand)]
pub enum ServerCommand {
    /// Run the Paron server
    Run(ServerRunArgs),

    /// Set up a fresh installation
    Bootstrap(ServerBootstrapArgs),

    /// Manage users
    #[command(subcommand)]
    Users(ServerUsersCommand),
}

pub async fn run(command: ServerCommand) -> Result<()> {
    match command {
        ServerCommand::Run(args) => run::run(args).await,
        ServerCommand::Bootstrap(args) => bootstrap::run(args).await,
        ServerCommand::Users(command) => users::run(command).await,
    }
}

#[derive(clap::Subcommand)]
pub enum ServerUsersCommand {
    /// List the users
    List(ServerDataArgs),

    /// Suspend every user and end every session.
    Lockdown(ServerLockdownArgs),

    /// Reset the password of a user and unlock their account, removing all other sign-in methods and active sessions.
    Reset(ServerUserArgs),
}

#[derive(clap::Args, clap_extra::ArgNames)]
pub struct ServerLockdownArgs {
    #[command(flatten)]
    pub data: ServerDataArgs,

    #[command(flatten)]
    pub confirm: ConfirmArgs,
}

#[derive(clap::Args, clap_extra::ArgNames)]
pub struct ServerUserArgs {
    #[command(flatten)]
    pub data: ServerDataArgs,

    /// The username of the user.
    #[arg(
        env = "PARON_USERNAME",
        value_name = "USERNAME",
        value_parser = |raw: &str| Username::parse(raw)
    )]
    pub username: Username,

    /// Read the new password from the standard input.
    #[arg(long, env = "PARON_PASSWORD_STDIN")]
    pub password_stdin: bool,
}

#[derive(clap::Args, clap_extra::ArgNames, clap_extra::ArgValues)]
pub struct ServerRunArgs {
    #[command(flatten)]
    pub data: ServerDataArgs,

    /// Server listen address.
    ///
    /// Example: --listen 0.0.0.0:443
    #[arg(long, env = "PARON_LISTEN", value_name = "HOST:PORT")]
    pub listen: SocketAddr,

    #[command(flatten)]
    pub public: PublicUrlArgs,

    #[cfg(debug_assertions)]
    #[arg(long = "no-dev-server")]
    pub no_frontend_dev_server: bool,

    #[command(flatten)]
    pub certificate: CertificateArgs,

    #[command(flatten)]
    pub proxy: ProxyArgs,

    #[command(flatten)]
    pub tuning: TuningArgs,
}

#[derive(clap::Args, clap_extra::ArgNames)]
pub struct ServerBootstrapArgs {
    /// Server data directory.
    ///
    /// Example: --data /var/lib/paron
    #[arg(long = "data", env = "PARON_DATA", value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    /// Server listen address.
    ///
    /// Example: --listen 0.0.0.0:8443
    #[arg(long, env = "PARON_LISTEN")]
    pub listen: Option<SocketAddr>,

    #[command(flatten)]
    pub public: PublicUrlArgs,

    #[command(flatten)]
    pub certificate: CertificateArgs,

    #[command(flatten)]
    pub proxy: ProxyArgs,

    #[command(flatten)]
    pub install: InstallArgs,

    #[command(flatten)]
    pub prompt: PromptArgs,

    #[command(flatten)]
    pub tuning: TuningArgs,
}

#[derive(clap::Args, Default, clap_extra::ArgNames, clap_extra::ArgValues)]
pub struct PublicUrlArgs {
    /// The public URL(s) of Paron, comma separated.
    ///
    /// Example: --public-url https://paron.example.com,https://10.0.0.5:8443
    #[arg(
        long = "public-url",
        env = "PARON_PUBLIC_URL",
        value_name = "URL",
        value_delimiter = ','
    )]
    pub urls: Vec<PublicUrl>,
}

#[derive(clap::Args, Default, clap_extra::ArgNames, clap_extra::ArgValues)]
pub struct CertificateArgs {
    /// Get a signed certificate from Let's Encrypt.
    ///
    /// The domains in --public-url must resolve to an address reachable from the internet on port 443
    #[arg(
        long,
        env = "PARON_LETSENCRYPT",
        value_name = "ENVIRONMENT",
        num_args = 0..=1,
        default_missing_value = "production",
        conflicts_with = "acme_directory"
    )]
    pub letsencrypt: Option<LetsEncrypt>,

    /// Get a signed certificate from a custom CA.
    ///
    /// The domains in --public-url must resolve to an address that CA can reach on port 443.
    ///
    /// Example: --acme-directory https://acme.zerossl.com/v2/DV90
    #[arg(long, env = "PARON_ACME_DIRECTORY", value_name = "URL")]
    pub acme_directory: Option<url::Url>,

    /// Send certificate expiration notices to this address.
    ///
    /// Example: --acme-contact admin@example.com
    #[arg(long, env = "PARON_ACME_CONTACT", value_name = "EMAIL")]
    pub acme_contact: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LetsEncrypt {
    /// Real signed certificates.
    Production,
    /// For testing purposes only.
    Staging,
}

impl std::fmt::Display for LetsEncrypt {
    #[expect(
        clippy::expect_used,
        reason = "each variant of LetsEncrypt has a clap value"
    )]
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let named = clap::ValueEnum::to_possible_value(self)
            .expect("could not transform letsencrypt into clap arg value");

        formatter.write_str(named.get_name())
    }
}

#[derive(clap::Args, Clone, Default, clap_extra::ArgNames, clap_extra::ArgValues)]
pub struct ProxyArgs {
    /// Configure trusted (L4 TCP) proxies running in front of Paron, comma separated.
    ///
    /// Paron supports PROXY protocol v1 and v2.
    /// If a PROXY protocol header is received from an address not present on this list, Paron drops the connection.
    ///
    /// Example: --trusted-proxies 10.0.0.7,192.168.0.0/24
    #[arg(
        long = "trusted-proxies",
        env = "PARON_TRUSTED_PROXIES",
        value_name = "ADDRESS|CIDR",
        value_delimiter = ','
    )]
    pub trusted_proxies: Option<Vec<crate::server::run::proxy::NetworkAddress>>,

    /// Configure trusted (L7 HTTP) proxies running in front of Paron, comma separated.
    ///
    /// Paron reads X-Forwarded-For, Forwarded and X-Real-IP, right to left, skipping addresses present on this list.
    /// Proxies MUST set or remove all of these headers.
    ///
    /// Example: --trusted-l7-proxies 10.0.0.7,192.168.0.0/24
    #[arg(
        long = "trusted-l7-proxies",
        env = "PARON_TRUSTED_L7_PROXIES",
        value_name = "ADDRESS|CIDR",
        value_delimiter = ','
    )]
    pub trusted_l7_proxies: Option<Vec<crate::server::run::proxy::NetworkAddress>>,
}

#[derive(clap::Args, Clone, Default, clap_extra::ArgNames, clap_extra::ArgValues)]
#[command(
    next_help_heading = "Tuning",
    after_long_help = "Paron computes the defaults from the specs of your machine."
)]
pub struct TuningArgs {
    /// Limit how many tunneled machines can connect at the same time.
    ///
    /// Raise this value if many machines reconnect together and some end up being refused the connection.
    ///
    /// Example: --max-connecting-agents 128
    #[arg(
        long = "max-connecting-agents",
        env = "PARON_MAX_CONNECTING_AGENTS",
        value_name = "COUNT"
    )]
    pub max_connecting_agents: Option<usize>,
}

#[derive(clap::Args, clap_extra::ArgNames, clap_extra::ArgValues)]
pub struct ServerDataArgs {
    /// Server data directory.
    ///
    /// Example: --data /var/lib/paron
    #[arg(long = "data", env = "PARON_DATA", value_name = "DIR")]
    pub data_dir: PathBuf,
}

#[derive(Clone)]
pub struct PublicUrl {
    host: url::Host,
    port: u16,
}

impl PublicUrl {
    pub fn new(host: url::Host, port: u16) -> Self {
        Self { host, port }
    }

    pub fn host(&self) -> &url::Host {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl From<&PublicUrl> for url::Origin {
    fn from(url: &PublicUrl) -> Self {
        Self::Tuple("https".to_owned(), url.host.clone(), url.port)
    }
}

impl TryFrom<&PublicUrl> for BrokerAddress {
    type Error = TunnelError;

    fn try_from(url: &PublicUrl) -> std::result::Result<Self, Self::Error> {
        format!("{}:{}", url.host, url.port).parse()
    }
}

impl std::fmt::Display for PublicUrl {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&url::Origin::from(self).ascii_serialization())
    }
}

impl std::str::FromStr for PublicUrl {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let origin = url::Url::parse(raw.trim())
            .ok()
            .filter(|url| url.scheme() == "https")
            .filter(|url| url.as_str() == format!("{}/", url.origin().ascii_serialization()))
            .map(|url| url.origin());

        match origin {
            Some(url::Origin::Tuple(_, host, port)) => Ok(Self { host, port }),
            _ => Err(format!(
                "{raw:?} is not an https URL with only a host and a port. Example: https://paron.example.com"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_public_url_returns_its_normalised_form() {
        for (raw, normalised) in [
            ("https://paron.example.com", "https://paron.example.com"),
            (
                "https://Paron.Example.com:443/",
                "https://paron.example.com",
            ),
            ("https://10.0.0.5:8443", "https://10.0.0.5:8443"),
            ("https://[::1]:8443", "https://[::1]:8443"),
            (" https://paron.local:8443 ", "https://paron.local:8443"),
        ] {
            let url = raw.parse::<PublicUrl>().unwrap();

            assert_eq!(url.to_string(), normalised, "{raw:?}");
        }
    }

    #[test]
    fn parse_public_url_that_is_not_a_bare_https_url_returns_error() {
        for raw in [
            "paron.example.com",
            "http://paron.example.com",
            "https://example.com/paron",
            "https://paron.example.com/?a=b",
            "https://paron.example.com/#a",
            "https://user@paron.example.com",
            "",
        ] {
            assert!(raw.parse::<PublicUrl>().is_err(), "{raw:?}");
        }
    }
}

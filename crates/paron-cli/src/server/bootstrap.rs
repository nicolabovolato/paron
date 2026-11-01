use std::collections::BTreeSet;
use std::io::IsTerminal as _;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use paron_core::application::identity::authorization::actor::Actor;
use paron_core::application::identity::users::CreateUserCommand;

use paron_core::application::identity::authentication::account_recovery::{
    RecoverAccountCommand, RecoveryAction,
};
use paron_core::application::identity::authentication::methods::password::Password;
use paron_core::application::repository::TxContext;
use paron_core::domain::user::{DisplayName, Role, Username};
use secrecy::{ExposeSecret as _, SecretBox, SecretString};

use crate::bootstrap::daemon::{self, SystemDaemon};
use crate::daemon_manager::Daemon;
use crate::server::cli::{
    CertificateArgs, LetsEncrypt, PublicUrl, PublicUrlArgs, ServerBootstrapArgs, ServerDataArgs,
    ServerRunArgs,
};
use crate::server::data_dir::{self, initial_admin_password};
use crate::server::paron::{self, Paron};
use crate::server::run::tls::acme;
use crate::server::run::tls::x509::Identifier as CertificateName;
use crate::ui::wizard::Wizard;
use crate::{Result, bootstrap, platform, ui};

#[cfg(target_os = "macos")]
const DAEMON_NAME: &str = "dev.bovolato.paron.server";
#[cfg(not(target_os = "macos"))]
const DAEMON_NAME: &str = "paron-server";
const DAEMON_DESCRIPTION: &str = "Paron server";

const DEFAULT_INSTALL_DIRECTORY: &str = "paron";
const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:443";
const DEFAULT_ADMIN_USERNAME: &str = "paron";
const DEFAULT_ADMIN_DISPLAY_NAME: &str = "Il Paron";

pub async fn run(args: ServerBootstrapArgs) -> Result<()> {
    ui::print_banner();

    if let Some(dir) = &args.data_dir
        && data_dir::installed(dir)?
    {
        println!(
            "{} is already a Paron installation, skipping bootstrap",
            dir.display()
        );

        return Ok(());
    }

    let mut wizard = Wizard::new(args.prompt.no_prompt, "server bootstrap");

    let setup = Setup::prepare(args, &mut wizard)?;

    if !wizard.yes_no("These answers are final. Install Paron?", true)? {
        wizard.cancel("cancelled");

        return Ok(());
    }

    setup.install(&mut wizard).await
}

struct Setup {
    data_dir: PathBuf,

    run_cmd: Vec<String>,
    run_args: ServerRunArgs,

    system_daemon: Option<SystemDaemon>,
}

impl Setup {
    #[allow(clippy::too_many_lines)]
    fn prepare(args: ServerBootstrapArgs, wizard: &mut Wizard) -> Result<Self> {
        let data_dir = wizard.input(
            "Server data directory",
            Some(&args.data_dir.map_or_else(
                || {
                    Path::new(platform::files::SERVICE_DATA_ROOT)
                        .join(DEFAULT_INSTALL_DIRECTORY)
                        .display()
                        .to_string()
                },
                |data| data.display().to_string(),
            )),
            None,
            bootstrap::parse_data_dir,
        )?;

        let listen = wizard.input(
            "Listen address",
            Some(&args.listen.map_or_else(
                || DEFAULT_LISTEN_ADDR.to_owned(),
                |listen| listen.to_string(),
            )),
            None,
            |typed: &str| Ok(typed.parse::<SocketAddr>()?),
        )?;
        let has_privileged_ports = listen.port() < 1024;

        let suggested_public_url: Vec<PublicUrl> = match args.public.urls.as_slice() {
            [] => host_ip_addr()
                .map(|host| PublicUrl::new(host, listen.port()))
                .into_iter()
                .collect(),
            urls => urls.to_vec(),
        };
        let origins = wizard.input(
            "Public URLs",
            Some(
                &suggested_public_url
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Some("https://paron.example.com, https://10.0.0.1:8443 (comma separated)"),
            |typed: &str| -> Result<Vec<PublicUrl>> {
                typed
                    .split(',')
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .map(|url| url.parse::<PublicUrl>().map_err(Into::into))
                    .collect()
            },
        )?;
        let can_order_a_certificate = origins.iter().any(|url| {
            CertificateName::try_from(url.host()).is_ok_and(|name| acme::can_be_ordered(&name))
        });

        let letsencrypt = match args.certificate.letsencrypt {
            given @ Some(_) => given,
            None if !can_order_a_certificate || args.certificate.acme_directory.is_some() => None,
            None => wizard
                .yes_no("Get a signed certificate from Let's Encrypt?", false)?
                .then_some(LetsEncrypt::Production),
        };

        let run_args = ServerRunArgs {
            data: ServerDataArgs {
                data_dir: data_dir.clone(),
            },
            listen,
            public: PublicUrlArgs { urls: origins },
            #[cfg(debug_assertions)]
            no_frontend_dev_server: true,
            certificate: CertificateArgs {
                letsencrypt,
                ..args.certificate
            },
            tuning: args.tuning,
            proxy: args.proxy,
        };

        let run_cmd = vec![
            std::env::current_exe()?.display().to_string(),
            "server".to_owned(),
            "run".to_owned(),
        ];

        let system_daemon = daemon::ask(
            Daemon {
                name: DAEMON_NAME,
                description: DAEMON_DESCRIPTION,
                command: run_cmd.clone(),
                environment: run_args.environment(),
                privileged_ports: has_privileged_ports,
                data_dir: data_dir.clone(),
            },
            args.install.daemon,
            #[cfg(unix)]
            &[],
            wizard,
        )?;

        Ok(Self {
            data_dir,
            run_cmd,
            run_args,
            system_daemon,
        })
    }

    async fn install(self, wizard: &mut Wizard) -> Result<()> {
        bootstrap::ask_for_root(&self.data_dir, self.system_daemon.as_ref())?;

        let step = wizard.install_step(&format!("creating {}", self.data_dir.display()));
        bootstrap::create_data_dir(&self.data_dir)?;
        let data_dir = data_dir::init(&self.data_dir)?;
        step.finish();

        let step = wizard.install_step("creating admin user");
        let administrator = Username::parse(DEFAULT_ADMIN_USERNAME)?;
        let random = SecretBox::init_with_mut(|random: &mut [u8; 16]| rand::fill(random));
        let password = SecretString::from(base16ct::lower::encode_string(random.expose_secret()));
        let paron = Paron::new(
            data_dir,
            paron::Limits::default(),
            &self.run_args.public.urls,
            self.run_args.listen.port(),
        )
        .await?;
        let administrator = paron
            .users
            .create(
                &mut TxContext::none(),
                &Actor::System,
                CreateUserCommand {
                    username: administrator,
                    display_name: DisplayName::parse(DEFAULT_ADMIN_DISPLAY_NAME)?,
                    roles: BTreeSet::from([Role::Administrator]),
                },
            )
            .await?;
        let link = paron
            .account_recovery
            .start_recovery(&mut TxContext::none(), &Actor::System, administrator.id)
            .await?;
        paron
            .account_recovery
            .recover(
                &mut TxContext::none(),
                RecoverAccountCommand {
                    token: link.token,
                    action: RecoveryAction::SetPassword(Password::parse(password.clone())?),
                },
            )
            .await?;
        paron.close().await;
        let password_file = if wizard.is_interactive() && std::io::stdout().is_terminal() {
            None
        } else {
            Some(initial_admin_password::write(&self.data_dir, &password)?)
        };
        step.finish();

        if let Some(daemon) = &self.system_daemon {
            daemon.install(wizard)?;
        }

        wizard.done("completed");

        println!();

        println!("administrator account:");
        println!("\tusername\t{}", &*administrator.username);
        if let Some(path) = &password_file {
            println!("\tpassword\tin {}", path.display());
            println!();
            println!(
                "\tthe file will be deleted in {} hours",
                initial_admin_password::LIFETIME.as_secs() / 3600
            );
        } else {
            println!("\tpassword\t{}", password.expose_secret());
            println!();
            println!("\tthe password will not be shown again");
        }

        println!();

        if let Some(daemon) = &self.system_daemon {
            println!("follow the server logs with:");
            println!("\t{}", platform::shell::join(daemon.logs_follow_command()));
        } else {
            println!("start the server with:");
            println!(
                "\t{}",
                platform::shell::join(self.run_cmd.iter().chain(&self.run_args.flags()))
            );
        }

        Ok(())
    }
}

fn host_ip_addr() -> Option<url::Host> {
    [
        ("0.0.0.0:0", "1.1.1.1:443"),
        ("[::]:0", "[2606:4700:4700::1111]:443"),
    ]
    .into_iter()
    .find_map(|(any, internet)| {
        let socket = std::net::UdpSocket::bind(any).ok()?;
        socket.connect(internet).ok()?;
        Some(match socket.local_addr().ok()?.ip() {
            IpAddr::V4(address) => url::Host::Ipv4(address),
            IpAddr::V6(address) => url::Host::Ipv6(address),
        })
    })
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn default_listen_addr_parses() {
        let parsed = DEFAULT_LISTEN_ADDR.parse::<SocketAddr>();

        assert_matches!(parsed, Ok(_));
    }

    #[test]
    fn default_admin_username_parses() {
        let parsed = Username::parse(DEFAULT_ADMIN_USERNAME);

        assert_matches!(parsed, Ok(_));
    }

    #[test]
    fn default_admin_display_name_parses() {
        let parsed = DisplayName::parse(DEFAULT_ADMIN_DISPLAY_NAME);

        assert_matches!(parsed, Ok(_));
    }
}

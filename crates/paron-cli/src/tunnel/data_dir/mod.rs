pub mod rollback;
pub mod sequence;

use std::fs;
use std::path::Path;

use paron_core::domain::machine::tunnel::token::{ServerEndpoint, Token};
use paron_tunnel_protocol::Identity;
use secrecy::{ExposeSecret as _, SecretSlice};

use crate::Result;
use crate::migration::{Migrations, Step};

const AGENT_PRIVATE_KEY_FILE: &str = "agent.key";
const SERVER_ENDPOINT_FILE: &str = "server-endpoint";

const MIGRATIONS: Migrations = Migrations::new(
    "tunnel",
    &[Step {
        version: 1,
        apply: |_| Ok(()),
    }],
);

pub fn installed(dir: &Path) -> Result<bool> {
    Ok(MIGRATIONS.current(dir)?.is_some())
}

pub struct Contents {
    pub server_endpoint: ServerEndpoint,
    pub agent_identity: Identity,
}

pub fn open(dir: &Path) -> Result<Contents> {
    MIGRATIONS.apply(dir)?;

    let server_endpoint_path = dir.join(SERVER_ENDPOINT_FILE);
    let agent_private_key_path = dir.join(AGENT_PRIVATE_KEY_FILE);

    Ok(Contents {
        server_endpoint: fs::read_to_string(&server_endpoint_path)
            .map_err(|reason| {
                format!(
                    "{} could not be read: {reason}",
                    server_endpoint_path.display()
                )
            })?
            .parse()?,
        agent_identity: fs::read(&agent_private_key_path)
            .map(SecretSlice::from)
            .map_err(|reason| {
                format!(
                    "{} could not be read: {reason}",
                    agent_private_key_path.display()
                )
            })
            .map(|stored| Identity::from_pkcs8_der(&stored))??,
    })
}

pub fn init(dir: &Path, token: &Token) -> Result<Contents> {
    let contents = Contents {
        server_endpoint: token.server_endpoint.clone(),
        agent_identity: Identity::generate()?,
    };

    let agent_private_key_path = dir.join(AGENT_PRIVATE_KEY_FILE);
    fs::write(
        &agent_private_key_path,
        contents.agent_identity.private_key_der().expose_secret(),
    )
    .map_err(|reason| {
        format!(
            "{} could not be written: {reason}",
            agent_private_key_path.display()
        )
    })?;

    update_server_endpoint(dir, &contents.server_endpoint)?;

    MIGRATIONS.stamp(dir)?;

    Ok(contents)
}

pub fn update_server_endpoint(dir: &Path, server: &ServerEndpoint) -> Result<()> {
    let path = dir.join(SERVER_ENDPOINT_FILE);
    let held = match fs::read_to_string(&path) {
        Ok(held) => Some(held),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!("{} could not be read: {error}", path.display()).into());
        }
    };

    let should_clear_rollback_index = held
        .and_then(|held| held.parse::<ServerEndpoint>().ok())
        .is_some_and(|held| held.fingerprint != server.fingerprint);

    if should_clear_rollback_index {
        rollback::clear(dir)?;
    }

    Ok(fs::write(&path, server.to_string())
        .map_err(|reason| format!("{} could not be written: {reason}", path.display()))?)
}

#[cfg(test)]
mod tests {
    use paron_tunnel_protocol::RollbackIndex as _;

    use super::*;
    use crate::testing::{ROOT_FINGERPRINT, TempDataDir};

    const MACHINE_ID: &str = "0198f0a0-0000-7000-8000-000000000001";
    const OTHER_MACHINE_ID: &str = "0198f0a0-0000-7000-8000-000000000002";

    fn enrollment_token(machine: &str) -> Token {
        format!("paron://paron.example:8443/{machine}?fingerprint={ROOT_FINGERPRINT}&secret=s3cret")
            .parse()
            .unwrap()
    }

    #[test]
    fn installed_empty_directory_returns_false() {
        let data_dir = TempDataDir::new("installation-empty");

        let is_installed = installed(data_dir.path()).unwrap();

        assert!(!is_installed);
    }

    #[test]
    fn installed_without_version_file_returns_false() {
        let data_dir = TempDataDir::new("installation-unfinished");
        init(data_dir.path(), &enrollment_token(MACHINE_ID)).unwrap();
        fs::remove_file(data_dir.join("version")).unwrap();

        let is_installed = installed(data_dir.path()).unwrap();

        assert!(!is_installed);
    }

    #[test]
    fn installed_after_init_returns_true() {
        let data_dir = TempDataDir::new("installation-installed");
        init(data_dir.path(), &enrollment_token(MACHINE_ID)).unwrap();

        let is_installed = installed(data_dir.path()).unwrap();

        assert!(is_installed);
    }

    #[test]
    fn update_server_endpoint_keeps_the_agent_key_and_stores_the_new_endpoint() {
        let data_dir = TempDataDir::new("installation-enrol");
        let contents_from_init = init(data_dir.path(), &enrollment_token(MACHINE_ID)).unwrap();

        update_server_endpoint(
            data_dir.path(),
            &enrollment_token(OTHER_MACHINE_ID).server_endpoint,
        )
        .unwrap();

        let contents_from_open = open(data_dir.path()).unwrap();
        assert_eq!(
            contents_from_open
                .agent_identity
                .private_key_der()
                .expose_secret(),
            contents_from_init
                .agent_identity
                .private_key_der()
                .expose_secret()
        );
        assert_eq!(
            contents_from_open.server_endpoint.machine.to_string(),
            OTHER_MACHINE_ID
        );
    }

    #[test]
    fn update_server_endpoint_with_the_same_fingerprint_keeps_the_rollback_index() {
        let data_dir = TempDataDir::new("installation-same-fingerprint");
        let index = rollback::OnDiskRollbackIndex::new(data_dir.path());
        init(data_dir.path(), &enrollment_token(MACHINE_ID)).unwrap();
        index.accepted(7).unwrap();

        update_server_endpoint(
            data_dir.path(),
            &enrollment_token(OTHER_MACHINE_ID).server_endpoint,
        )
        .unwrap();

        assert_eq!(index.highest().unwrap(), 7);
    }

    #[test]
    fn update_server_endpoint_with_another_fingerprint_clears_the_rollback_index() {
        let data_dir = TempDataDir::new("installation-another-fingerprint");
        let index = rollback::OnDiskRollbackIndex::new(data_dir.path());
        init(data_dir.path(), &enrollment_token(MACHINE_ID)).unwrap();
        index.accepted(7).unwrap();
        let token_with_another_fingerprint: Token = format!(
            "paron://paron.example:8443/{OTHER_MACHINE_ID}?fingerprint=sha256:{}&secret=s3cret",
            "ff".repeat(32)
        )
        .parse()
        .unwrap();

        update_server_endpoint(
            data_dir.path(),
            &token_with_another_fingerprint.server_endpoint,
        )
        .unwrap();

        assert_eq!(index.highest().unwrap(), 0);
    }

    #[test]
    fn open_without_agent_key_returns_error() {
        let data_dir = TempDataDir::new("installation-open-no-key");
        init(data_dir.path(), &enrollment_token(MACHINE_ID)).unwrap();
        fs::remove_file(data_dir.join(AGENT_PRIVATE_KEY_FILE)).unwrap();

        let opened = open(data_dir.path());

        assert!(opened.is_err());
    }

    #[test]
    fn open_empty_directory_returns_error_and_mints_no_key() {
        let data_dir = TempDataDir::new("installation-open-empty");

        let Err(error) = open(data_dir.path()) else {
            panic!("an empty directory has not enrolled");
        };

        assert!(error.to_string().contains("missing version"), "{error}");
        assert!(!data_dir.join(AGENT_PRIVATE_KEY_FILE).exists());
    }

    #[test]
    fn init_writes_an_endpoint_that_parses_back_without_the_secret() {
        let data_dir = TempDataDir::new("installation-server-round-trip");
        let token = enrollment_token(OTHER_MACHINE_ID);

        init(data_dir.path(), &token).unwrap();

        let endpoint_file = fs::read_to_string(data_dir.join(SERVER_ENDPOINT_FILE)).unwrap();
        let endpoint: ServerEndpoint = endpoint_file.parse().unwrap();

        assert_eq!(endpoint.machine, token.server_endpoint.machine);
        assert_eq!(endpoint.fingerprint, token.server_endpoint.fingerprint);
        assert_eq!(
            endpoint.address.to_string(),
            token.server_endpoint.address.to_string()
        );
        assert!(!endpoint_file.contains("s3cret"), "{endpoint_file}");
    }
}

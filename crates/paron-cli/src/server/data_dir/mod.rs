pub mod broker_ca;
pub mod database;
pub mod initial_admin_password;
pub mod keys;
pub mod self_signed_ca;

use std::path::{Path, PathBuf};

use paron_tunnel_protocol::BrokerIdentity;

use self::keys::MasterKey;
use crate::Result;
use crate::migration::{Migrations, Step};

const MIGRATIONS: Migrations = Migrations::new(
    "server",
    &[Step {
        version: 1,
        apply: |_| Ok(()),
    }],
);

pub struct Contents {
    pub keys: MasterKey,
    pub tunnel_identity: BrokerIdentity,
    pub database_path: PathBuf,
}

pub fn installed(dir: &Path) -> Result<bool> {
    Ok(MIGRATIONS.current(dir)?.is_some())
}

pub fn open(dir: &Path) -> Result<Contents> {
    MIGRATIONS.apply(dir)?;

    Ok(Contents {
        keys: keys::read(dir)?,
        tunnel_identity: broker_ca::read(dir)?,
        database_path: database::read(dir)?,
    })
}

pub fn init(dir: &Path) -> Result<Contents> {
    let opened = Contents {
        keys: keys::mint(dir)?,
        tunnel_identity: broker_ca::mint(dir)?,
        database_path: database::mint(dir)?,
    };

    MIGRATIONS.stamp(dir)?;

    Ok(opened)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::server::paron::{Limits, Paron};
    use crate::testing::TempDataDir;

    const VERSION_FILE: &str = "version";

    #[cfg(unix)]
    #[test]
    fn init_writes_every_secret_unreadable_by_anyone_else() {
        use std::os::unix::fs::PermissionsExt as _;

        let data_dir = TempDataDir::new("modes");

        init(data_dir.path()).unwrap();

        for name in [broker_ca::FILE, keys::FILE, database::FILE] {
            let mode = fs::metadata(data_dir.join(name))
                .unwrap()
                .permissions()
                .mode();

            assert_eq!(mode & 0o777, 0o600, "{name} is {:o}", mode & 0o777);
        }
    }

    #[tokio::test]
    async fn open_after_init_returns_contents_paron_starts_from() {
        let data_dir = TempDataDir::new("round-trip");
        init(data_dir.path()).unwrap();

        let contents = open(data_dir.path()).unwrap();

        Paron::new(contents, Limits::default(), &[], 443)
            .await
            .unwrap()
            .close()
            .await;
    }

    #[test]
    fn open_tunnel_directory_returns_error_that_names_it() {
        let data_dir = TempDataDir::new("wrong-half");
        fs::write(data_dir.join(VERSION_FILE), "tunnel 1\n").unwrap();

        let Err(error) = open(data_dir.path()) else {
            panic!("a tunnel directory is not a server installation");
        };

        assert_eq!(
            error.to_string(),
            format!(
                "{}: this directory is set to run Paron tunnel, not Paron server",
                data_dir.path().display()
            )
        );
    }

    #[test]
    fn open_installation_without_database_returns_error() {
        let data_dir = TempDataDir::new("no-database");
        init(data_dir.path()).unwrap();
        fs::remove_file(data_dir.join(database::FILE)).unwrap();

        let Err(error) = open(data_dir.path()) else {
            panic!("the database is missing");
        };

        assert!(error.to_string().contains(database::FILE), "{error}");
    }

    #[test]
    fn open_installation_without_version_returns_error_and_stamps_nothing() {
        let data_dir = TempDataDir::new("unversioned");
        init(data_dir.path()).unwrap();
        fs::remove_file(data_dir.join(VERSION_FILE)).unwrap();

        let Err(error) = open(data_dir.path()) else {
            panic!("the version file is missing");
        };

        assert!(error.to_string().contains("missing version"), "{error}");
        assert_eq!(MIGRATIONS.current(data_dir.path()).unwrap(), None);
    }

    #[test]
    fn open_empty_directory_returns_error_and_writes_nothing() {
        let data_dir = TempDataDir::new("open-empty");

        let Err(error) = open(data_dir.path()) else {
            panic!("an empty directory is not an installation");
        };

        assert!(error.to_string().contains("missing version"), "{error}");
        assert_eq!(fs::read_dir(data_dir.path()).unwrap().count(), 0);
    }
}

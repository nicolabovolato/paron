use std::fs;
use std::path::{Path, PathBuf};

use paron_tunnel_protocol::{RollbackIndex, TunnelError};

pub fn clear(dir: &Path) -> crate::Result<()> {
    let path = dir.join(OnDiskRollbackIndex::FILENAME);

    match fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{} could not be removed: {error}", path.display()).into())
        }
        _ => Ok(()),
    }
}

#[derive(Debug)]
pub struct OnDiskRollbackIndex {
    path: PathBuf,
    write_lock: parking_lot::Mutex<()>,
}

impl OnDiskRollbackIndex {
    const FILENAME: &str = "broker-serving-index";

    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(Self::FILENAME),
            write_lock: parking_lot::Mutex::new(()),
        }
    }
}

impl RollbackIndex for OnDiskRollbackIndex {
    fn highest(&self) -> std::result::Result<u64, TunnelError> {
        match fs::read_to_string(&self.path) {
            Ok(held) => held.trim().parse().map_err(TunnelError::unexpected),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(error) => Err(TunnelError::unexpected(std::io::Error::other(format!(
                "{} could not be read: {error}",
                self.path.display()
            )))),
        }
    }

    fn accepted(&self, index: u64) -> std::result::Result<(), TunnelError> {
        let _write_lock = self.write_lock.lock();
        if index <= self.highest()? {
            return Ok(());
        }

        crate::platform::files::atomic_replace(&self.path, index.to_string().as_bytes()).map_err(
            |error| {
                TunnelError::unexpected(std::io::Error::other(format!(
                    "{} could not be written: {error}",
                    self.path.display()
                )))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;
    use crate::testing::TempDataDir;

    #[test]
    fn highest_from_nothing_recorded_returns_zero() {
        let data_dir = TempDataDir::new("rollback-fresh");

        let highest = OnDiskRollbackIndex::new(data_dir.path()).highest();

        assert_matches!(highest, Ok(0));
    }

    #[test]
    fn highest_after_accepted_returns_the_accepted_index() {
        let data_dir = TempDataDir::new("rollback-round-trip");
        OnDiskRollbackIndex::new(data_dir.path())
            .accepted(7)
            .unwrap();

        let highest = OnDiskRollbackIndex::new(data_dir.path()).highest();

        assert_matches!(highest, Ok(7));
    }

    #[test]
    fn accepted_below_the_recorded_index_keeps_the_record() {
        let data_dir = TempDataDir::new("rollback-lower");
        let index = OnDiskRollbackIndex::new(data_dir.path());
        index.accepted(9).unwrap();

        index.accepted(7).unwrap();

        assert_matches!(index.highest(), Ok(9));
    }

    #[test]
    fn accepted_from_concurrent_writers_keeps_the_highest() {
        let data_dir = TempDataDir::new("rollback-concurrent");
        let index = OnDiskRollbackIndex::new(data_dir.path());

        std::thread::scope(|scope| {
            for written in (1..=32).rev() {
                let index = &index;
                scope.spawn(move || index.accepted(written).unwrap());
            }
        });

        assert_matches!(index.highest(), Ok(32));
    }

    #[test]
    fn highest_from_a_truncated_record_returns_error() {
        let data_dir = TempDataDir::new("rollback-truncated");
        fs::write(data_dir.join(OnDiskRollbackIndex::FILENAME), "").unwrap();

        let highest = OnDiskRollbackIndex::new(data_dir.path()).highest();

        assert_matches!(highest, Err(_));
    }

    #[cfg(unix)]
    #[test]
    fn highest_from_an_unreadable_record_returns_error() {
        let data_dir = TempDataDir::new("rollback-unreadable");
        let path = data_dir.join(OnDiskRollbackIndex::FILENAME);
        fs::write(&path, "5").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

        let highest = OnDiskRollbackIndex::new(data_dir.path()).highest();

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_matches!(highest, Err(_));
    }

    #[cfg(unix)]
    #[test]
    fn accepted_in_a_read_only_directory_returns_error_and_keeps_the_record() {
        let data_dir = TempDataDir::new("rollback-read-only");
        let path = data_dir.join(OnDiskRollbackIndex::FILENAME);
        fs::write(&path, "5").unwrap();
        fs::set_permissions(data_dir.path(), fs::Permissions::from_mode(0o500)).unwrap();

        let accepted = OnDiskRollbackIndex::new(data_dir.path()).accepted(9);

        fs::set_permissions(data_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert_matches!(accepted, Err(_));
        assert_eq!(fs::read_to_string(&path).unwrap(), "5");
    }
}

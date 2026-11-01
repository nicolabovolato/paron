use std::fs;
use std::path::{Path, PathBuf};

use paron_tunnel_protocol::{Sequence, TunnelError};

use crate::platform::files;

pub struct OnDiskSequence(PathBuf);

impl OnDiskSequence {
    const FILENAME: &str = "agent-sequence";

    pub fn new(dir: &Path) -> Self {
        Self(dir.join(Self::FILENAME))
    }
}

impl Sequence for OnDiskSequence {
    fn next(&self) -> std::result::Result<u64, TunnelError> {
        let last: u64 = match fs::read_to_string(&self.0) {
            Ok(held) => held.parse().map_err(TunnelError::unexpected)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => {
                return Err(TunnelError::unexpected(std::io::Error::other(format!(
                    "{} could not be read: {error}",
                    self.0.display()
                ))));
            }
        };

        let next = last.checked_add(1).ok_or_else(|| {
            TunnelError::unexpected(std::io::Error::other("claim sequence exhausted"))
        })?;

        files::atomic_replace(&self.0, next.to_string().as_bytes()).map_err(|error| {
            TunnelError::unexpected(std::io::Error::other(format!(
                "{} could not be written: {error}",
                self.0.display()
            )))
        })?;

        Ok(next)
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
    fn next_from_nothing_recorded_returns_one() {
        let data_dir = TempDataDir::new("sequence-fresh");

        let next = OnDiskSequence::new(data_dir.path()).next();

        assert_matches!(next, Ok(1));
    }

    #[test]
    fn next_returns_one_more_than_the_record() {
        let data_dir = TempDataDir::new("sequence-advance");
        fs::write(data_dir.join(OnDiskSequence::FILENAME), "5").unwrap();

        let next = OnDiskSequence::new(data_dir.path()).next();

        assert_matches!(next, Ok(6));
    }

    #[test]
    fn next_from_a_truncated_record_returns_error() {
        let data_dir = TempDataDir::new("sequence-truncated");
        fs::write(data_dir.join(OnDiskSequence::FILENAME), "").unwrap();

        let next = OnDiskSequence::new(data_dir.path()).next();

        assert_matches!(next, Err(_));
    }

    #[cfg(unix)]
    #[test]
    fn next_from_an_unreadable_record_returns_error() {
        let data_dir = TempDataDir::new("sequence-unreadable");
        let path = data_dir.join(OnDiskSequence::FILENAME);
        fs::write(&path, "5").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

        let next = OnDiskSequence::new(data_dir.path()).next();

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_matches!(next, Err(_));
    }

    // A sequence number that is not on disk must not be sent.
    #[cfg(unix)]
    #[test]
    fn next_in_a_read_only_directory_returns_error_and_keeps_the_record() {
        let data_dir = TempDataDir::new("sequence-read-only");
        let path = data_dir.join(OnDiskSequence::FILENAME);
        fs::write(&path, "5").unwrap();
        fs::set_permissions(data_dir.path(), fs::Permissions::from_mode(0o500)).unwrap();

        let next = OnDiskSequence::new(data_dir.path()).next();

        fs::set_permissions(data_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert_matches!(next, Err(_));
        assert_eq!(fs::read_to_string(&path).unwrap(), "5");
    }

    #[test]
    fn next_at_the_top_of_the_range_returns_error() {
        let data_dir = TempDataDir::new("sequence-exhausted");
        fs::write(
            data_dir.join(OnDiskSequence::FILENAME),
            u64::MAX.to_string(),
        )
        .unwrap();

        let next = OnDiskSequence::new(data_dir.path()).next();

        assert_matches!(next, Err(_));
    }
}

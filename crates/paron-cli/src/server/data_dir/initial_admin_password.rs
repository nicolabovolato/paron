use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use secrecy::{ExposeSecret as _, SecretString};

use crate::Result;

pub const FILE: &str = "initial-admin-password";

pub const LIFETIME: Duration = Duration::from_hours(24);

pub fn write(dir: &Path, password: &SecretString) -> Result<PathBuf> {
    use std::io::Write as _;

    let path = dir.join(FILE);

    std::fs::File::create(&path)
        .and_then(|mut file| {
            file.write_all(password.expose_secret().as_bytes())?;
            file.write_all(b"\n")
        })
        .map_err(|reason| format!("{} could not be written: {reason}", path.display()))?;

    Ok(path)
}

/// Removes the file once [`LIFETIME`] is up. Does nothing where there is no file
pub async fn expire(dir: &Path) -> Result<()> {
    let path = dir.join(FILE);

    let Some(due) = due_in(&path)? else {
        return Ok(());
    };

    tokio::time::sleep(due).await;

    match std::fs::remove_file(&path) {
        Ok(()) => tracing::info!("{} has expired and was removed", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn due_in(path: &Path) -> Result<Option<Duration>> {
    let written = match std::fs::metadata(path) {
        Ok(metadata) => metadata.modified()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };

    let age = SystemTime::now()
        .duration_since(written)
        .unwrap_or(Duration::ZERO);

    Ok(Some(LIFETIME.saturating_sub(age)))
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::TempDataDir;

    fn password() -> SecretString {
        SecretString::from("0123456789abcdef")
    }

    fn write_expired_password(dir: &Path) -> PathBuf {
        let path = write(dir, &password()).unwrap();
        let written = SystemTime::now() - LIFETIME - Duration::from_hours(1);

        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(written))
            .unwrap();

        path
    }

    #[cfg(unix)]
    #[test]
    fn write_creates_file_nobody_else_can_read() {
        use std::os::unix::fs::PermissionsExt as _;

        let data_dir = TempDataDir::new("initial-password-mode");

        let path = write(data_dir.path(), &password()).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "found {:o}", mode & 0o777);
    }

    #[test]
    fn write_stores_password_and_nothing_else() {
        let data_dir = TempDataDir::new("initial-password-contents");

        let path = write(data_dir.path(), &password()).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "0123456789abcdef\n"
        );
    }

    #[test]
    fn due_in_fresh_file_returns_about_the_whole_lifetime() {
        let data_dir = TempDataDir::new("initial-password-fresh");
        let path = write(data_dir.path(), &password()).unwrap();

        let time_until_expiry = due_in(&path).unwrap().unwrap();

        assert!(
            time_until_expiry > LIFETIME.saturating_sub(Duration::from_mins(1)),
            "{time_until_expiry:?}"
        );
    }

    #[test]
    fn due_in_expired_file_returns_zero() {
        let data_dir = TempDataDir::new("initial-password-expired");
        let path = write_expired_password(data_dir.path());

        let time_until_expiry = due_in(&path).unwrap();

        assert_eq!(time_until_expiry, Some(Duration::ZERO));
    }

    #[tokio::test]
    async fn expire_deletes_expired_file() {
        let data_dir = TempDataDir::new("initial-password-removed");
        let path = write_expired_password(data_dir.path());

        expire(data_dir.path()).await.unwrap();

        assert!(!path.exists(), "{} is still there", path.display());
    }

    #[tokio::test]
    async fn expire_without_a_file_succeeds() {
        let data_dir = TempDataDir::new("initial-password-none");

        let expired = expire(data_dir.path()).await;

        assert_matches!(expired, Ok(()));
    }
}

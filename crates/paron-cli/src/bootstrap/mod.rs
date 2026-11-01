#[cfg_attr(unix, path = "daemon_unix.rs")]
#[cfg_attr(windows, path = "daemon_windows.rs")]
pub mod daemon;

use std::path::{Path, PathBuf};

use crate::Result;
use crate::platform;
#[cfg(unix)]
use crate::ui;

pub fn ask_for_root(data_dir: &Path, daemon: Option<&daemon::SystemDaemon>) -> Result<()> {
    let needs_root = !platform::files::can_create(data_dir) || daemon.is_some();

    if !needs_root || platform::privileges::is_user_root() {
        return Ok(());
    }

    #[cfg(windows)]
    {
        Err("Paron needs administrator rights for your desired setup. Run it in a terminal as administrator".into())
    }

    #[cfg(unix)]
    {
        ui::warn("Paron needs root privileges for your desired setup");

        let Some(mut cmd) = platform::unix::privileges::request_root() else {
            return Err("could not ask for root privileges".into());
        };

        let asked = cmd.status().map_err(|reason| {
            format!(
                "{}: {reason}",
                cmd.get_program().to_string_lossy().into_owned()
            )
        })?;

        if !asked.success() {
            return Err("failed to obtain root privileges".into());
        }

        Ok(())
    }
}

pub fn create_data_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    if !platform::files::can_create(dir) {
        return platform::unix::files::install_dir(
            dir,
            &platform::accounts::current_user()?,
            0o700,
        );
    }

    std::fs::create_dir_all(dir)
        .map_err(|reason| format!("could not create {}: {reason}", dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(
            |reason| format!("could not restrict access to {}: {reason}", dir.display()),
        )?;
    }

    #[cfg(windows)]
    platform::windows::files::restrict_dir(dir)?;

    Ok(())
}

pub fn parse_data_dir(typed: &str) -> Result<PathBuf> {
    let dir = PathBuf::from(typed);

    let dir = if dir.is_absolute() {
        dir
    } else {
        std::env::current_dir()?.join(dir)
    };

    let contents = match std::fs::read_dir(&dir) {
        Err(reason) if reason.kind() == std::io::ErrorKind::NotFound => return Ok(dir),
        Err(reason) => return Err(format!("{} could not be read: {reason}", dir.display()).into()),
        Ok(reading) => reading,
    };

    for entry in contents {
        let name = entry
            .map_err(|reason| format!("{} could not be read: {reason}", dir.display()))?
            .file_name();

        if name.as_encoded_bytes().starts_with(b".") || name == "lost+found" {
            continue;
        }

        return Err(format!("{} is not empty", dir.display()).into());
    }

    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDataDir;

    #[test]
    fn parse_data_dir_with_only_dotfiles_and_lost_found_succeeds() {
        let data_dir = TempDataDir::new("occupants-mount");
        std::fs::create_dir(data_dir.join("lost+found")).unwrap();
        std::fs::write(data_dir.join(".DS_Store"), []).unwrap();

        let parsed = parse_data_dir(&data_dir.path().display().to_string()).unwrap();

        assert_eq!(parsed, data_dir.path());
    }

    #[test]
    fn parse_data_dir_with_another_file_returns_error() {
        let data_dir = TempDataDir::new("occupants-file");
        std::fs::write(data_dir.join("keys.bin"), []).unwrap();

        let error = parse_data_dir(&data_dir.path().display().to_string()).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("{} is not empty", data_dir.path().display())
        );
    }

    #[test]
    fn parse_data_dir_of_missing_directory_returns_it() {
        let missing = std::env::temp_dir().join("paron-nothing-is-here/never");

        let parsed = parse_data_dir(&missing.display().to_string()).unwrap();

        assert_eq!(parsed, missing);
    }

    #[test]
    fn parse_data_dir_relative_path_returns_it_under_the_working_directory() {
        let parsed = parse_data_dir("paron-nothing-is-here").unwrap();

        assert_eq!(
            parsed,
            std::env::current_dir()
                .unwrap()
                .join("paron-nothing-is-here")
        );
    }
}

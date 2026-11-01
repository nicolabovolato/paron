use std::path::{Path, PathBuf};

use super::{accounts, privileges};
use crate::Result;

pub fn is_unix_socket(path: &Path) -> bool {
    use std::os::unix::fs::FileTypeExt as _;

    let Ok(found) = std::fs::metadata(path) else {
        return false;
    };

    found.file_type().is_socket()
}

pub fn group_of(path: &Path) -> Result<Option<String>> {
    use std::os::unix::fs::MetadataExt as _;

    let gid = std::fs::metadata(path)
        .map_err(|reason| format!("{} could not be read: {reason}", path.display()))?
        .gid();

    accounts::group_name(gid)
}

pub fn program_exists(program: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();

    std::env::split_paths(&path)
        .chain(
            [
                "/usr/local/sbin",
                "/usr/local/bin",
                "/usr/sbin",
                "/usr/bin",
                "/sbin",
                "/bin",
            ]
            .map(PathBuf::from),
        )
        .any(|directory| {
            rustix::fs::accessat(
                rustix::fs::CWD,
                directory.join(program),
                rustix::fs::Access::EXEC_OK,
                rustix::fs::AtFlags::EACCESS,
            )
            .is_ok()
        })
}

/// Creates the directory using elevated privileges, gives it and what it holds to `owner`, and
/// gives the directory the mode
pub fn install_dir(dir: &Path, owner: &str, mode: u32) -> Result<()> {
    let path = dir.to_string_lossy();

    privileges::run_privileged("mkdir", &["-p", &path])
        .map_err(|reason| format!("{} could not be created: {reason}", dir.display()))?;

    privileges::run_privileged("chown", &["-R", owner, &path]).map_err(|reason| {
        format!(
            "could not change the owner of {} to {owner}: {reason}",
            dir.display()
        )
    })?;

    privileges::run_privileged("chmod", &[&format!("{mode:o}"), &path]).map_err(|reason| {
        format!(
            "could not change the mode of {} to {mode:o}: {reason}",
            dir.display()
        )
        .into()
    })
}

/// Writes the file using elevated privileges and gives it the mode
#[allow(clippy::missing_panics_doc)]
pub fn install_file(path: &Path, contents: &str, mode: u32) -> Result<()> {
    use std::io::Write as _;

    let mut tee = privileges::privileged_cmd("tee")?
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .map_err(|reason| format!("could not write {}: {reason}", path.display()))?;

    tee.stdin
        .take()
        .ok_or_else(|| format!("could not write {}", path.display()))?
        .write_all(contents.as_bytes())?;

    if !tee.wait()?.success() {
        return Err(format!("could not write {}", path.display()).into());
    }

    privileges::run_privileged("chmod", &[&format!("{mode:o}"), &path.to_string_lossy()]).map_err(
        |reason| {
            format!(
                "could not change the mode of {} to {mode:o}: {reason}",
                path.display()
            )
            .into()
        },
    )
}

/// Restricts every file and directory this process creates to its owner, unless the inherited
/// umask is already tighter.
pub fn restrict_new_files_to_owner() {
    let wanted_mask = rustix::fs::Mode::RWXG | rustix::fs::Mode::RWXO;

    // The umask cannot be read without setting it first.
    let previous_mask = rustix::process::umask(wanted_mask);

    // Sets the wanted mode only if the previous mode was looser.
    rustix::process::umask(previous_mask | wanted_mask);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDataDir;

    #[test]
    fn program_exists_shell_returns_true() {
        assert!(program_exists("sh"));
    }

    #[test]
    fn program_exists_unknown_program_returns_false() {
        assert!(!program_exists("paron-no-program-has-this-name-8f21c4"));
    }

    #[test]
    fn group_of_returns_the_group_that_owns_the_path() {
        use std::os::unix::fs::MetadataExt as _;

        let data_dir = TempDataDir::new("group-owning");
        let path = data_dir.join("socket");
        std::fs::write(&path, []).unwrap();
        let gid = std::fs::metadata(&path).unwrap().gid();
        let group_name = nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
            .unwrap()
            .map(|group| group.name);

        let group = group_of(&path).unwrap();

        assert_eq!(group, group_name);
    }

    #[test]
    fn group_of_missing_path_returns_error() {
        let error = group_of(Path::new("/nonexistent/paron/db.sock")).unwrap_err();

        assert!(error.to_string().contains("could not be read"), "{error}");
    }

    #[test]
    fn restrict_new_files_to_owner_hides_them_from_anyone_else() {
        use std::os::unix::fs::PermissionsExt as _;

        let data_dir = TempDataDir::new("restricted-file");
        let path = data_dir.join("secret");

        restrict_new_files_to_owner();
        std::fs::write(&path, b"key material").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "found {:o}", mode & 0o777);
    }
}

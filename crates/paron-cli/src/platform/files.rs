use std::path::Path;

use crate::Result;

/// The file that discards everything written to it
#[cfg(unix)]
pub const NULL_DEVICE: &str = "/dev/null";
#[cfg(windows)]
pub const NULL_DEVICE: &str = "NUL";

/// Where each platform expects an installed service to store its data
pub const SERVICE_DATA_ROOT: &str = cfg_select! {
    target_os = "macos" => { "/Library" }
    target_os = "linux" => { "/var/lib" }
    windows => { r"C:\ProgramData" }
    _ => { "/var/db" }
};

/// Locks the directory as `lock_directory` does. On Unix, root first becomes the owner of the
/// directory. Windows cannot change the account of a process, so there the process keeps its
/// account.
pub fn lock_directory_as_owner(dir: &Path) -> Result<DirectoryLockGuard> {
    #[cfg(unix)]
    super::unix::privileges::become_owner_of(dir)?;

    lock_directory(dir)
}

/// Errors if another process is holding the directory
pub fn lock_directory(dir: &Path) -> Result<DirectoryLockGuard> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;

        const DELETE: u32 = 0x0001_0000;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const ERROR_SHARING_VIOLATION: i32 = 32;

        match std::fs::OpenOptions::new()
            .access_mode(DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)
        {
            Ok(dir_handle) => Ok(DirectoryLockGuard(dir_handle)),
            Err(reason) if reason.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => {
                Err(format!("{dir} is in use by another process", dir = dir.display()).into())
            }
            Err(reason) if reason.kind() == std::io::ErrorKind::NotFound => {
                Err(format!("{dir} does not exist", dir = dir.display()).into())
            }
            Err(reason) => {
                Err(format!("could not lock {dir}: {reason}", dir = dir.display()).into())
            }
        }
    }

    #[cfg(unix)]
    {
        let dir_contents = std::fs::File::open(dir).map_err(|reason| {
            if reason.kind() == std::io::ErrorKind::NotFound {
                format!("{dir} does not exist", dir = dir.display())
            } else {
                format!("could not open {dir}: {reason}", dir = dir.display())
            }
        })?;

        match rustix::fs::flock(
            &dir_contents,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        ) {
            Ok(()) => Ok(DirectoryLockGuard(dir_contents)),
            Err(rustix::io::Errno::WOULDBLOCK) => {
                Err(format!("{dir} is in use by another process", dir = dir.display()).into())
            }
            Err(reason) => {
                Err(format!("could not lock {dir}: {reason}", dir = dir.display()).into())
            }
        }
    }
}

pub struct DirectoryLockGuard(#[allow(dead_code)] std::fs::File);

pub fn can_create(path: &Path) -> bool {
    path.ancestors()
        .find(|parent| parent.exists())
        .is_some_and(|existing| {
            // Windows opens the directory only if its access list gives the requested access.
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt as _;

                const FILE_ADD_FILE: u32 = 0x0000_0002;
                const FILE_ADD_SUBDIRECTORY: u32 = 0x0000_0004;
                const FILE_SHARE_READ: u32 = 0x0000_0001;
                const FILE_SHARE_WRITE: u32 = 0x0000_0002;
                const FILE_SHARE_DELETE: u32 = 0x0000_0004;
                const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

                std::fs::OpenOptions::new()
                    .access_mode(FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                    .open(existing)
                    .is_ok()
            }

            #[cfg(unix)]
            {
                rustix::fs::accessat(
                    rustix::fs::CWD,
                    existing,
                    rustix::fs::Access::WRITE_OK | rustix::fs::Access::EXEC_OK,
                    rustix::fs::AtFlags::EACCESS,
                )
                .is_ok()
            }
        })
}

/// Replaces a file's contents, or leaves what is already there untouched.
pub fn atomic_replace(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    let temporary = path.with_added_extension("tmp");
    let mut file = std::fs::File::create(&temporary)?;

    file.write_all(contents)?;
    file.sync_all()?;

    std::fs::rename(&temporary, path)?;

    // NTFS journals the rename, and Windows cannot open a directory to flush it.
    #[cfg(unix)]
    if let Some(directory) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::File::open(directory)?.sync_all()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDataDir;

    #[test]
    fn can_create_under_directory_this_user_owns_returns_true() {
        let data_dir = TempDataDir::new("can-create");

        assert!(can_create(&data_dir.join("paron/deeper")));
    }

    #[cfg(unix)]
    #[test]
    fn can_create_under_root_owned_tree_returns_false() {
        assert!(!can_create(Path::new("/nonexistent/paron/never")));
    }

    #[test]
    fn lock_directory_held_returns_error() {
        let data_dir = TempDataDir::new("lock-held");
        let _directory_lock = lock_directory(data_dir.path()).unwrap();

        let Err(error) = lock_directory(data_dir.path()) else {
            panic!("the directory is locked");
        };

        assert!(error.to_string().contains("another process"), "{error}");
    }

    #[test]
    fn lock_directory_after_drop_succeeds() {
        let data_dir = TempDataDir::new("lock-released");
        drop(lock_directory(data_dir.path()).unwrap());

        let relocked = lock_directory(data_dir.path());

        assert!(relocked.is_ok(), "{:?}", relocked.err());
    }
}

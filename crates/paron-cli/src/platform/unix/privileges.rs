use std::path::Path;
use std::process::Command;

use nix::unistd::{Gid, Uid, User};

use super::files::program_exists;
use crate::Result;
use crate::platform::privileges::is_user_root;

pub fn request_root() -> Option<Command> {
    let sudo_like = SudoLike::installed()?;

    let mut command = Command::new(sudo_like.program());
    command.arg(sudo_like.authenticate_flag());

    Some(command)
}

pub fn privileged_cmd(program: &str) -> Result<Command> {
    if is_user_root() {
        return Ok(Command::new(program));
    }

    let sudo_like = SudoLike::installed().ok_or_else(|| format!("cannot run {program} as root"))?;

    let mut running = Command::new(sudo_like.program());
    running.arg(program);

    Ok(running)
}

/// Runs the program as root, and errors with what it wrote to stderr when it fails
pub fn run_privileged(program: &str, arguments: &[&str]) -> Result<()> {
    let ran = privileged_cmd(program)?
        .args(arguments)
        .output()
        .map_err(|reason| format!("{program} could not be run: {reason}"))?;

    if !ran.status.success() {
        return Err(format!("{program}: {}", String::from_utf8_lossy(&ran.stderr).trim()).into());
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum SudoLike {
    Sudo,
    Doas,
}

impl SudoLike {
    fn installed() -> Option<Self> {
        [Self::Sudo, Self::Doas]
            .into_iter()
            .find(|sudo_like| program_exists(sudo_like.program()))
    }

    fn program(self) -> &'static str {
        match self {
            Self::Sudo => "sudo",
            Self::Doas => "doas",
        }
    }

    fn authenticate_flag(self) -> &'static str {
        match self {
            Self::Sudo => "-v",
            // doas has no validate-only flag, so we run the "true" command and trigger an auth prompt.
            Self::Doas => "true",
        }
    }
}

pub fn become_owner_of(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;

    if !is_user_root() {
        return Ok(());
    }

    let metadata = std::fs::metadata(path)
        .map_err(|reason| format!("{} could not be read: {reason}", path.display()))?;
    if metadata.uid() == 0 {
        return Ok(());
    }

    let uid = Uid::from_raw(metadata.uid());
    let account = User::from_uid(uid)
        .map_err(|reason| format!("could not get the user of uid {uid}: {reason}"))?;

    become_owner(uid, Gid::from_raw(metadata.gid()), account)
}

/// An owner with no account, for example in a container with no `/etc/passwd`, gets the group of
/// the directory and no other groups.
#[cfg(not(target_vendor = "apple"))]
fn become_owner(uid: Uid, gid: Gid, account: Option<User>) -> Result<()> {
    let gid = if let Some(account) = account {
        let name = std::ffi::CString::new(account.name)?;
        nix::unistd::initgroups(&name, account.gid)
            .map_err(|reason| format!("could not set the groups of uid {uid}: {reason}"))?;

        account.gid
    } else {
        nix::unistd::setgroups(&[])
            .map_err(|reason| format!("could not set the groups of uid {uid}: {reason}"))?;

        gid
    };
    nix::unistd::setgid(gid).map_err(|reason| format!("could not become gid {gid}: {reason}"))?;
    nix::unistd::setuid(uid).map_err(|reason| format!("could not become uid {uid}: {reason}"))?;

    Ok(())
}

/// macOS has no `initgroups`, and without it the groups of root stay.
#[cfg(target_vendor = "apple")]
fn become_owner(uid: Uid, _directory_gid: Gid, account: Option<User>) -> Result<()> {
    Err(format!(
        "run this command as {}",
        account.map_or_else(|| format!("uid {uid}"), |account| account.name)
    )
    .into())
}

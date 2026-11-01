use std::path::Path;
use std::process::Command;

use super::accounts;
use crate::Result;

/// Makes the current user the owner, and gives full access only to that user, the administrators
/// and the system. Files and directories that are created in it later get the same access list.
///
/// Anyone can create a directory in `C:\ProgramData` first, and the owner of a directory can always
/// change its access list. So the owner changes too, and the reset removes what the access list had.
/// `takeown` uses the right of the administrators to take a directory that they cannot open.
pub fn restrict_dir(dir: &Path) -> Result<()> {
    const SYSTEM: &str = "*S-1-5-18";
    const ADMINISTRATORS: &str = "*S-1-5-32-544";

    let (user, sid) = accounts::current_user_and_sid()?;
    let sid = format!("*{sid}");

    run(Command::new("takeown").arg("/f").arg(dir))
        .and_then(|()| run(Command::new("icacls").arg(dir).arg("/reset")))
        .and_then(|()| {
            run(Command::new("icacls").arg(dir).args([
                "/inheritance:r",
                "/grant:r",
                &format!("{SYSTEM}:(OI)(CI)F"),
                &format!("{ADMINISTRATORS}:(OI)(CI)F"),
                &format!("{sid}:(OI)(CI)F"),
            ]))
        })
        .map_err(|reason| {
            format!(
                "could not give {user} access to {}: {reason}",
                dir.display()
            )
            .into()
        })
}

/// Adds full access for the account to the directory, and to what it holds.
pub fn grant_full_access(dir: &Path, account: &str) -> Result<()> {
    run(Command::new("icacls")
        .arg(dir)
        .args(["/grant", &format!("{account}:(OI)(CI)F")]))
    .map_err(|reason| {
        format!(
            "could not give {account} access to {}: {reason}",
            dir.display()
        )
        .into()
    })
}

fn run(command: &mut Command) -> std::result::Result<(), String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let ran = command
        .output()
        .map_err(|reason| format!("{program} could not be run: {reason}"))?;

    if !ran.status.success() {
        return Err(String::from_utf8_lossy(&[ran.stdout, ran.stderr].concat())
            .trim()
            .to_owned());
    }

    Ok(())
}

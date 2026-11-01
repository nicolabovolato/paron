use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::Result;
#[cfg(target_os = "macos")]
use crate::daemon_manager::launchd::Launchd;
#[cfg(target_os = "linux")]
use crate::daemon_manager::systemd::Systemd;
use crate::daemon_manager::{Daemon, DaemonManager};
use crate::platform;
use crate::ui::wizard::Wizard;

#[cfg(target_os = "macos")]
const ACCOUNT_NAME: &str = "_paron";
#[cfg(not(target_os = "macos"))]
const ACCOUNT_NAME: &str = "paron";

pub struct SystemDaemon {
    daemon: Daemon,
    groups: Vec<String>,
    manager: &'static dyn DaemonManager,
}

pub fn ask(
    daemon: Daemon,
    requested: bool,
    sockets: &[PathBuf],
    wizard: &mut Wizard,
) -> Result<Option<SystemDaemon>> {
    let Some(manager) = detect_manager() else {
        if requested {
            return Err("no supported daemon manager".into());
        }

        wizard.log_input("system daemon", "no supported daemon manager detected");

        return Ok(None);
    };

    let replaces = manager.has_definition(&daemon);
    let question = format!(
        "{action} the daemon definition at {path}, which runs as {account}",
        action = if replaces { "replace" } else { "write" },
        path = manager.definition_path(&daemon).display(),
        account = ACCOUNT_NAME,
    );

    let wanted = if requested {
        wizard.log_input(&question, "yes");

        true
    } else {
        // Without a terminal, only the flag installs a daemon.
        wizard.is_interactive() && wizard.yes_no(&question, !replaces)?
    };

    if !wanted {
        return Ok(None);
    }

    let groups = sockets
        .iter()
        .map(|path| {
            platform::unix::files::group_of(path)?.ok_or_else(|| {
                format!("the group of {path} does not exist", path = path.display()).into()
            })
        })
        .collect::<Result<BTreeSet<_>>>()?;

    Ok(Some(SystemDaemon {
        daemon,
        groups: groups.into_iter().collect(),
        manager,
    }))
}

impl SystemDaemon {
    pub fn install(&self, wizard: &mut Wizard) -> Result<()> {
        let step = wizard.install_step(&format!("creating system account {ACCOUNT_NAME}"));

        if !platform::unix::accounts::exists(ACCOUNT_NAME)? {
            platform::unix::accounts::create_user(ACCOUNT_NAME)
                .map_err(|reason| format!("could not create {ACCOUNT_NAME}: {reason}"))?;
        }

        for group in &self.groups {
            platform::unix::accounts::add_user_to_group(ACCOUNT_NAME, group).map_err(|reason| {
                format!("could not add {ACCOUNT_NAME} to group {group}: {reason}")
            })?;
        }

        platform::unix::files::install_dir(&self.daemon.data_dir, ACCOUNT_NAME, 0o700)?;

        step.finish_with(&format!(
            "created system account {account}, owner of {data_dir}{groups}",
            account = ACCOUNT_NAME,
            data_dir = self.daemon.data_dir.display(),
            groups = if self.groups.is_empty() {
                String::new()
            } else {
                format!(" (groups: {names})", names = self.groups.join(", "))
            },
        ));

        let step = wizard.install_step(&format!(
            "write the daemon definition at {path}",
            path = self.manager.definition_path(&self.daemon).display(),
        ));
        self.manager.install(&self.daemon, ACCOUNT_NAME)?;
        step.finish();

        let step = wizard.install_step(&format!("start {name}", name = self.daemon.name));
        self.manager.start(&self.daemon)?;
        step.finish();

        Ok(())
    }

    pub fn logs_follow_command(&self) -> Vec<String> {
        self.manager.logs_follow_command(&self.daemon)
    }
}

#[cfg_attr(target_os = "macos", allow(clippy::unnecessary_wraps))]
fn detect_manager() -> Option<&'static dyn DaemonManager> {
    cfg_select! {
        target_os = "linux" => {
            std::path::Path::new("/run/systemd/system").is_dir().then_some(&Systemd as &dyn DaemonManager)
        }
        target_os = "macos" => { Some(&Launchd) }
        _ => { None }
    }
}

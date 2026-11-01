use crate::Result;
use crate::daemon_manager::{Daemon, windows_service};
use crate::ui::wizard::Wizard;

pub struct SystemDaemon {
    daemon: Daemon,
}

pub fn ask(daemon: Daemon, requested: bool, wizard: &mut Wizard) -> Result<Option<SystemDaemon>> {
    let replaces = windows_service::has_definition(&daemon);
    let question = format!(
        "{action} the definition of the Windows service {name}, which runs as {account}",
        action = if replaces { "replace" } else { "write" },
        name = daemon.name,
        account = windows_service::account(&daemon),
    );

    let wanted = if requested {
        wizard.log_input(&question, "yes");

        true
    } else {
        // Without a terminal, only the flag installs a daemon.
        wizard.is_interactive() && wizard.yes_no(&question, !replaces)?
    };

    Ok(wanted.then_some(SystemDaemon { daemon }))
}

impl SystemDaemon {
    pub fn install(&self, wizard: &mut Wizard) -> Result<()> {
        let step = wizard.install_step(&format!(
            "write the definition of the Windows service {name}",
            name = self.daemon.name,
        ));
        windows_service::install(&self.daemon)?;
        step.finish();

        let step = wizard.install_step(&format!("start {name}", name = self.daemon.name));
        windows_service::start(&self.daemon)?;
        step.finish();

        Ok(())
    }

    pub fn logs_follow_command(&self) -> Vec<String> {
        windows_service::logs_command(&self.daemon)
    }
}

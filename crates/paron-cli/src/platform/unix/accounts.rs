use super::privileges;
use crate::Result;

pub fn exists(name: &str) -> Result<bool> {
    let user = nix::unistd::User::from_name(name)
        .map_err(|reason| format!("could not check existence of user {name}: {reason}"))?;

    Ok(user.is_some())
}

pub fn user_name(uid: u32) -> Result<Option<String>> {
    let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
        .map_err(|reason| format!("could not get username of uid {uid}: {reason}"))?;

    if let Some(user) = user {
        return Ok(Some(user.name));
    }

    // musl reads only /etc/passwd. `id` belongs to the host, so it also knows an LDAP or SSSD account.
    let Ok(ran) = std::process::Command::new("id")
        .arg("-un")
        .arg(uid.to_string())
        .output()
    else {
        return Ok(None);
    };
    let name = String::from_utf8_lossy(&ran.stdout).trim().to_owned();

    Ok((ran.status.success() && !name.is_empty()).then_some(name))
}

pub fn group_name(gid: u32) -> Result<Option<String>> {
    let group = nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
        .map_err(|reason| format!("could not get group name of gid {gid}: {reason}"))?;

    Ok(group.map(|group| group.name))
}

pub fn create_user(name: &str) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        let shell = [
            "/usr/sbin/nologin",
            "/sbin/nologin",
            "/run/current-system/sw/bin/nologin",
            "/bin/false",
        ]
        .into_iter()
        .find(|shell| std::path::Path::new(shell).exists())
        .ok_or("could not find nologin shell")?;

        match ["useradd", "pw", "adduser"]
            .into_iter()
            .find(|program| super::files::program_exists(program))
        {
            Some(program @ "useradd") => privileges::run_privileged(
                program,
                &["--system", "--no-create-home", "--shell", shell, name],
            ),
            Some(program @ "pw") => privileges::run_privileged(
                program,
                &["useradd", "-n", name, "-s", shell, "-d", "/var/empty"],
            ),
            Some(program @ "adduser") => {
                privileges::run_privileged(program, &["-S", "-D", "-H", "-s", shell, name])
            }
            _ => Err("no program available to create user".into()),
        }
    }

    #[cfg(target_os = "macos")]
    {
        let id = (450..500)
            .find(|id| {
                matches!(
                    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(*id)),
                    Ok(None)
                ) && matches!(
                    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(*id)),
                    Ok(None)
                )
            })
            .ok_or("no free id from 450 to 499 for the account")?
            .to_string();
        let group = format!("/Groups/{name}");
        let user = format!("/Users/{name}");

        for attribute in [
            [group.as_str(), "PrimaryGroupID", &id],
            [&group, "Password", "*"],
            [&user, "UniqueID", &id],
            [&user, "PrimaryGroupID", &id],
            [&user, "UserShell", "/usr/bin/false"],
            [&user, "NFSHomeDirectory", "/var/empty"],
            [&user, "Password", "*"],
            [&user, "IsHidden", "1"],
        ] {
            privileges::run_privileged(
                "dscl",
                &[".", "-create", attribute[0], attribute[1], attribute[2]],
            )?;
        }

        Ok(())
    }
}

pub fn add_user_to_group(name: &str, group: &str) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        match ["usermod", "pw", "addgroup"]
            .into_iter()
            .find(|program| super::files::program_exists(program))
        {
            Some(program @ "usermod") => privileges::run_privileged(program, &["-aG", group, name]),
            Some(program @ "pw") => {
                privileges::run_privileged(program, &["groupmod", group, "-m", name])
            }
            Some(program @ "addgroup") => privileges::run_privileged(program, &[name, group]),
            _ => Err("no program available to add user to group".into()),
        }
    }

    #[cfg(target_os = "macos")]
    {
        privileges::run_privileged(
            "dseditgroup",
            &["-o", "edit", "-a", name, "-t", "user", group],
        )
    }
}

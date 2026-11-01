use std::path::Path;

use paron_core::application::identity::authentication::account_recovery::{
    AccountRecoveryToken, RecoverAccountCommand, RecoveryAction,
};
use paron_core::application::identity::authentication::methods::password::Password;
use paron_core::application::identity::authorization::actor::Actor;
use paron_core::application::identity::users::UpdateUserCommand;
use paron_core::application::repository::{ListQuery, Page, TxContext};
use paron_core::domain::user::UserStatus;
use secrecy::SecretString;
use tabled::settings::Style;
use tabled::{Table, Tabled};
use zeroize::Zeroizing;

use crate::server::cli::{ServerDataArgs, ServerLockdownArgs, ServerUserArgs, ServerUsersCommand};
use crate::server::data_dir;
use crate::server::paron::{self, Paron};
use crate::{Result, platform, ui};

const USERS_PER_PAGE: u32 = 100;

#[derive(Tabled)]
struct UserRow {
    #[tabled(rename = "USERNAME")]
    username: String,
    #[tabled(rename = "STATUS")]
    status: &'static str,
    #[tabled(rename = "ROLES")]
    roles: String,
}

pub async fn run(command: ServerUsersCommand) -> Result<()> {
    match command {
        ServerUsersCommand::List(args) => list(args).await,
        ServerUsersCommand::Lockdown(args) => lockdown(args).await,
        ServerUsersCommand::Reset(args) => reset(args).await,
    }
}

async fn list(args: ServerDataArgs) -> Result<()> {
    let users = with_paron(&args.data_dir, async |paron| {
        let mut users = Vec::new();

        loop {
            let page = paron
                .users
                .list(
                    &mut TxContext::none(),
                    &Actor::System,
                    &ListQuery {
                        search: None,
                        page: Page {
                            limit: USERS_PER_PAGE,
                            offset: u32::try_from(users.len())?,
                        },
                    },
                )
                .await?;

            if page.items.is_empty() {
                return Ok(users);
            }

            users.extend(page.items);
        }
    })
    .await?;

    let rows = users.iter().map(|user| UserRow {
        username: user.username.to_string(),
        status: match user.status {
            UserStatus::Active => "active",
            UserStatus::Suspended => "suspended",
        },
        roles: user
            .roles
            .iter()
            .map(|role| role.as_str())
            .collect::<Vec<_>>()
            .join(","),
    });

    println!("{}", Table::new(rows).with(Style::blank()));

    Ok(())
}

async fn lockdown(args: ServerLockdownArgs) -> Result<()> {
    ui::warn("SYSTEM LOCKDOWN");
    ui::warn("This will suspend all users. No one will be able to sign in");
    if !args.confirm.yes
        && !ui::confirm("Proceed anyway?").map_err(ui::not_interactive_needs_flag(
            ServerLockdownArgs::NAMES.confirm.yes.flag,
        ))?
    {
        return Ok(());
    }

    let suspended = with_paron(&args.data.data_dir, async |paron| {
        let mut offset = 0;
        let mut suspended = 0;
        loop {
            let page = paron
                .users
                .list(
                    &mut TxContext::none(),
                    &Actor::System,
                    &ListQuery {
                        search: None,
                        page: Page {
                            limit: USERS_PER_PAGE,
                            offset,
                        },
                    },
                )
                .await?;
            if page.items.is_empty() {
                return Ok(suspended);
            }
            offset += USERS_PER_PAGE;

            for user in page.items {
                if user.status == UserStatus::Suspended {
                    continue;
                }
                paron
                    .users
                    .update(
                        &mut TxContext::none(),
                        &Actor::System,
                        UpdateUserCommand {
                            id: user.id,
                            username: None,
                            display_name: None,
                            roles: None,
                            status: Some(UserStatus::Suspended),
                        },
                    )
                    .await?;
                suspended += 1;
            }
        }
    })
    .await?;

    println!("{suspended} users suspended.");
    println!("To restore access for a user, run:");
    println!(
        "\t{} <USERNAME> {}",
        platform::shell::join([
            std::env::current_exe()?.display().to_string(),
            "server".to_owned(),
            "users".to_owned(),
            "reset".to_owned(),
        ]),
        platform::shell::join(args.data.flags())
    );

    Ok(())
}

async fn reset(args: ServerUserArgs) -> Result<()> {
    with_paron(&args.data.data_dir, async |paron| {
        let user = paron
            .users
            .get_by_username(&mut TxContext::none(), &Actor::System, &args.username)
            .await?;

        let password = if args.password_stdin {
            let mut typed = Zeroizing::new(String::new());
            std::io::stdin().read_line(&mut typed)?;

            Password::parse(SecretString::from(typed.lines().next().unwrap_or_default()))?
        } else {
            ui::password("New password", |typed| {
                Ok(Password::parse(SecretString::from(typed))?)
            })
            .map_err(ui::not_interactive_needs_flag(
                ServerUserArgs::NAMES.password_stdin.flag,
            ))?
        };

        paron
            .users
            .update(
                &mut TxContext::none(),
                &Actor::System,
                UpdateUserCommand {
                    id: user.id,
                    username: None,
                    display_name: None,
                    roles: None,
                    status: Some(UserStatus::Active),
                },
            )
            .await?;
        let AccountRecoveryToken { token, .. } = paron
            .account_recovery
            .start_recovery(&mut TxContext::none(), &Actor::System, user.id)
            .await?;
        paron
            .account_recovery
            .recover(
                &mut TxContext::none(),
                RecoverAccountCommand {
                    token,
                    action: RecoveryAction::SetPassword(password),
                },
            )
            .await?;

        println!(
            "Password reset for {}. Other sign-in methods removed, all sessions ended.",
            &*args.username
        );

        Ok(())
    })
    .await
}

async fn with_paron<T>(
    data_dir: &Path,
    action: impl AsyncFnOnce(&Paron) -> Result<T>,
) -> Result<T> {
    let _directory = platform::files::lock_directory_as_owner(data_dir)?;

    let paron = Paron::new(
        data_dir::open(data_dir)?,
        paron::Limits::default(),
        &[],
        443,
    )
    .await?;
    let result = action(&paron).await;
    paron.close().await;

    result
}

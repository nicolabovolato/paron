use std::sync::Arc;

use paron_core::domain::machine::tunnel::token::{ServerEndpoint, Token};
use paron_tunnel_protocol::{Identity, RollbackIndex, Sequence};

use super::data_dir;
use super::data_dir::rollback::OnDiskRollbackIndex;
use super::data_dir::sequence::OnDiskSequence;
use crate::tunnel::cli::TunnelEnrolArgs;
use crate::{Result, platform, ui};

pub async fn run(args: TunnelEnrolArgs) -> Result<()> {
    let token: Token = args.token.parse()?;

    let _guard = platform::files::lock_directory_as_owner(&args.data.data_dir)?;
    let contents = data_dir::open(&args.data.data_dir)?;

    if !approved(
        &contents.server_endpoint,
        &token.server_endpoint,
        args.confirm.yes,
    )? {
        return Ok(());
    }

    println!("enrolling to {}", token.server_endpoint.address);

    data_dir::update_server_endpoint(&args.data.data_dir, &token.server_endpoint)?;
    enrol(
        &token,
        contents.agent_identity,
        Arc::new(OnDiskSequence::new(&args.data.data_dir)),
        Arc::new(OnDiskRollbackIndex::new(&args.data.data_dir)),
    )
    .await?;

    println!("enrolled as machine {}", token.server_endpoint.machine);

    Ok(())
}

pub async fn enrol(
    token: &Token,
    identity: Identity,
    sequence: Arc<dyn Sequence>,
    rollback: Arc<dyn RollbackIndex>,
) -> Result<()> {
    super::agent(
        &token.server_endpoint,
        identity,
        sequence,
        rollback,
        Vec::new(),
    )
    .join(&token.secret)
    .await
    .map_err(|reason| {
        format!(
            "could not enrol to {address}: {reason}",
            address = token.server_endpoint.address
        )
    })?;

    Ok(())
}

fn approved(old: &ServerEndpoint, new: &ServerEndpoint, yes: bool) -> Result<bool> {
    let changes: Vec<String> = [
        (
            format!("the server changed from {} to {}", old.address, new.address),
            old.address.to_string() != new.address.to_string(),
        ),
        (
            "the server fingerprint changed".to_owned(),
            old.fingerprint != new.fingerprint,
        ),
        (
            format!(
                "the machine id changed from {} to {}",
                old.machine, new.machine
            ),
            old.machine != new.machine,
        ),
    ]
    .into_iter()
    .filter_map(|(warning, changed)| changed.then_some(warning))
    .collect();

    if changes.is_empty() {
        return Ok(true);
    }

    for warning in &changes {
        ui::warn(warning);
    }

    Ok(yes
        || ui::confirm("Proceed anyway?").map_err(ui::not_interactive_needs_flag(
            TunnelEnrolArgs::NAMES.confirm.yes.flag,
        ))?)
}

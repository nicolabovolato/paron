mod background_tasks;
mod http;
mod listener;
pub mod proxy;
pub mod tls;
mod tuning;

use std::sync::Arc;

use futures_util::TryFutureExt as _;
use paron_tunnel_protocol::quic::QuicBrokerTransport;
use paron_tunnel_protocol::tcp::TcpBroker;
use tokio::net::TcpListener;

use crate::server::cli::ServerRunArgs;
use crate::server::data_dir::{self, initial_admin_password};
use crate::server::paron::Paron;
use crate::{Result, lifecycle, platform};
use background_tasks::BackgroundTasks;

pub async fn run(args: ServerRunArgs) -> Result<()> {
    tracing::info!("Paron {}", env!("CARGO_PKG_VERSION"));
    if platform::privileges::is_user_root() {
        tracing::warn!("running as root");
    }

    let _guard = platform::files::lock_directory_as_owner(&args.data.data_dir)?;
    let data_dir = data_dir::open(&args.data.data_dir)?;

    let limits = tuning::limits(&args.tuning);

    if args.public.urls.is_empty() {
        tracing::warn!(
            "{} is not set. Links use localhost, and work only on this machine",
            ServerRunArgs::NAMES.public.urls.flag,
        );
    }

    let paron = Paron::new(
        data_dir,
        limits.paron,
        &args.public.urls,
        args.listen.port(),
    )
    .await?;

    let proxies = proxy::Proxies {
        l4: proxy::L4Proxies::new(args.proxy.trusted_proxies.as_deref().unwrap_or_default()),
        l7: proxy::L7Proxies::new(args.proxy.trusted_l7_proxies.as_deref().unwrap_or_default()),
    };

    let mut background_tasks = BackgroundTasks::new();

    background_tasks.add({
        let data_dir = args.data.data_dir.clone();

        async move {
            if let Err(error) = initial_admin_password::expire(&data_dir).await {
                tracing::error!("the initial administrator password could not be removed: {error}");
            }

            std::future::pending::<()>().await;
        }
    });

    for task in paron.background_tasks().await? {
        background_tasks.add(task);
    }

    let (tls, self_signed) = tls::configure(
        &args.data.data_dir,
        &args.public.urls,
        &args.certificate,
        &mut background_tasks,
    )?;

    let router = http::build(
        &paron,
        &proxies.l7,
        &mut background_tasks,
        self_signed,
        #[cfg(debug_assertions)]
        !args.no_frontend_dev_server,
    )?;

    let quic_tunnel_listener = paron.tunnel.bind(&QuicBrokerTransport, args.listen).await?;
    let tcp_tunnel = TcpBroker::new(Arc::clone(&paron.tunnel))?;

    let tcp_listener = TcpListener::bind(args.listen).await?;

    tracing::info!("listening on {}", args.listen);
    #[cfg(debug_assertions)]
    tracing::info!("frontend dev server at https://{}", args.listen);

    let stop_signal = tokio_util::sync::CancellationToken::new();
    let mut tasks = tokio::task::JoinSet::new();

    tasks.spawn(listener::serve(
        tcp_listener,
        tls,
        router,
        tcp_tunnel,
        proxies,
        limits.listener,
        stop_signal.clone(),
    ));

    tasks.spawn(
        quic_tunnel_listener
            .with_graceful_shutdown(stop_signal.clone().cancelled_owned())
            .map_err(Into::into),
    );

    tasks.spawn(background_tasks.run(stop_signal.clone()));

    lifecycle::run(tasks, stop_signal, paron.close()).await
}

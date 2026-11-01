use std::time::Duration;

use crate::{Result, platform};

pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(15);
pub const STOP_TIMEOUT: Duration = DRAIN_TIMEOUT.saturating_add(Duration::from_secs(5));

pub async fn run(
    mut tasks: tokio::task::JoinSet<Result<()>>,
    stop_signal: tokio_util::sync::CancellationToken,
    release: impl Future<Output = ()>,
) -> Result<()> {
    let stop_result = tokio::select! {
        completed = tasks.join_next() => match completed {
            Some(Err(failed)) => Err(format!("Paron stopped unexpectedly: {failed}").into()),
            Some(Ok(failed @ Err(_))) => failed,
            _ if stop_signal.is_cancelled() => Ok(()),
            _ => Err("Paron stopped unexpectedly".into()),
        },
        signal = platform::signals::stop_requested() => match signal {
            Ok(signal) => {
                tracing::info!("{signal} received, shutting down");

                Ok(())
            }
            Err(error) => Err(format!("could not listen for the stop signals: {error}").into()),
        },
        () = stop_signal.cancelled() => Ok(()),
    };

    stop_signal.cancel();

    let drain_result = tokio::select! {
        () = async {
            while tasks.join_next().await.is_some() {}
            release.await;
        } => Ok(()),
        signal = platform::signals::stop_requested() => match signal {
            Ok(signal) => {
                tracing::warn!("{signal} received, exiting");

                Err(Stopped::Interrupted.into())
            }
            Err(error) => Err(format!("could not listen for the stop signals: {error}").into()),
        },
        () = tokio::time::sleep(DRAIN_TIMEOUT) => {
            tracing::warn!("still stopping after {DRAIN_TIMEOUT:?}, exiting");

            Err(Stopped::TimedOut.into())
        }
    };

    stop_result.and(drain_result)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stopped {
    Interrupted,
    TimedOut,
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Interrupted => "interrupted",
            Self::TimedOut => "could not stop in time",
        })
    }
}

impl std::error::Error for Stopped {}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::task::JoinSet;
    use tokio_util::sync::CancellationToken;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn run_task_ends_after_the_stop_returns_ok() {
        let stop_signal = CancellationToken::new();
        stop_signal.cancel();
        let mut tasks = JoinSet::new();
        tasks.spawn(std::future::ready(Ok(())));

        let stopped = run(tasks, stop_signal, std::future::ready(())).await;

        assert_matches!(stopped, Ok(()));
    }

    #[tokio::test(start_paused = true)]
    async fn run_task_ends_without_a_stop_returns_error() {
        let mut tasks = JoinSet::new();
        tasks.spawn(std::future::ready(Ok(())));

        let error = run(tasks, CancellationToken::new(), std::future::ready(()))
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "Paron stopped unexpectedly");
    }

    #[tokio::test(start_paused = true)]
    async fn run_task_fails_and_drain_never_finishes_returns_the_failure() {
        let mut tasks = JoinSet::new();
        tasks.spawn(std::future::ready(Err("the listener gave up".into())));
        tasks.spawn(std::future::pending());

        let error = run(tasks, CancellationToken::new(), std::future::ready(()))
            .await
            .unwrap_err();

        assert_eq!(error.to_string(), "the listener gave up");
    }

    #[tokio::test(start_paused = true)]
    async fn run_task_panics_returns_error() {
        let mut tasks = JoinSet::new();
        tasks.spawn(async { panic!("the listener came apart") });

        let error = run(tasks, CancellationToken::new(), std::future::ready(()))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("panicked"), "{error}");
    }

    #[tokio::test(start_paused = true)]
    async fn run_drain_past_the_timeout_returns_timed_out() {
        let stop_signal = CancellationToken::new();
        stop_signal.cancel();
        let mut tasks = JoinSet::new();
        tasks.spawn(std::future::pending());

        let error = run(tasks, stop_signal, std::future::ready(()))
            .await
            .unwrap_err();

        assert_eq!(error.downcast_ref::<Stopped>(), Some(&Stopped::TimedOut));
    }

    #[tokio::test(start_paused = true)]
    async fn run_drain_finishes_releases_after_the_last_task() {
        let stop_signal = CancellationToken::new();
        stop_signal.cancel();
        let task_ended = Arc::new(AtomicBool::new(false));
        let mut tasks = JoinSet::new();
        tasks.spawn({
            let task_ended = Arc::clone(&task_ended);

            async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                task_ended.store(true, Ordering::Relaxed);

                Ok(())
            }
        });
        let (released, mut released_rx) = tokio::sync::oneshot::channel();

        let stopped = run(tasks, stop_signal, async move {
            released.send(task_ended.load(Ordering::Relaxed)).unwrap();
        })
        .await;

        assert_matches!(stopped, Ok(()));
        assert_eq!(released_rx.try_recv(), Ok(true));
    }

    #[tokio::test(start_paused = true)]
    async fn run_drain_past_the_timeout_does_not_release() {
        let stop_signal = CancellationToken::new();
        stop_signal.cancel();
        let mut tasks = JoinSet::new();
        tasks.spawn(std::future::pending());
        let (released, mut released_rx) = tokio::sync::oneshot::channel();

        let error = run(tasks, stop_signal, async move {
            released.send(()).unwrap();
        })
        .await
        .unwrap_err();

        assert_eq!(error.downcast_ref::<Stopped>(), Some(&Stopped::TimedOut));
        assert_matches!(
            released_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        );
    }
}

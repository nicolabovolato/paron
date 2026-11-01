use std::future::Future;

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;

#[derive(Clone, Default)]
pub struct Tasks {
    tracker: TaskTracker,
    stopping: CancellationToken,
}

impl Tasks {
    /// Runs `work` until it ends or the tasks stop, then `cleanup` in both cases. The log records an
    /// error of `cleanup`.
    pub fn spawn(
        &self,
        work: impl Future<Output = ()> + Send + 'static,
        cleanup: impl Future<Output = anyhow::Result<()>> + Send + 'static,
    ) {
        let stopping = self.stopping.clone();

        self.tracker.spawn(
            async move {
                stopping.run_until_cancelled(work).await;
                if let Err(error) = cleanup.await {
                    let error = format!("{error:#}");

                    tracing::error!(%error, "could not clean up task");
                }
            }
            .in_current_span(),
        );
    }

    pub async fn stop(&self) {
        self.stopping.cancel();
        self.tracker.close();
        self.tracker.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn stop_with_running_work_waits_for_its_cleanup() {
        let tasks = Tasks::default();
        let cleaned_up = Arc::new(AtomicBool::new(false));

        tasks.spawn(std::future::pending(), {
            let cleaned_up = Arc::clone(&cleaned_up);

            async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                cleaned_up.store(true, Ordering::SeqCst);

                Ok(())
            }
        });
        tasks.stop().await;

        assert!(cleaned_up.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn spawn_after_stop_runs_the_cleanup_at_once() {
        let tasks = Tasks::default();
        tasks.stop().await;
        let (cleaned_up_tx, cleaned_up) = tokio::sync::oneshot::channel();

        tasks.spawn(std::future::pending(), async move {
            cleaned_up_tx.send(()).unwrap();

            Ok(())
        });

        tokio::time::timeout(Duration::from_secs(10), cleaned_up)
            .await
            .expect("the cleanup runs although the work never ends")
            .unwrap();
    }
}

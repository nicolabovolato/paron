use std::future::Future;

pub struct BackgroundTasks(Vec<futures_util::future::BoxFuture<'static, ()>>);

impl BackgroundTasks {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn add(&mut self, task: impl Future<Output = ()> + Send + 'static) {
        self.0.push(Box::pin(task));
    }

    pub async fn run(self, stop_signal: tokio_util::sync::CancellationToken) -> crate::Result<()> {
        tokio::select! {
            failure = async {
                let mut running = tokio::task::JoinSet::new();

                for task in self.0 {
                    running.spawn(task);
                }

                match running.join_next().await {
                    None => std::future::pending().await,
                    Some(Err(panicked)) => {
                        crate::Error::from(format!("background job panicked: {panicked}"))
                    }
                    Some(Ok(())) => crate::Error::from("background job stopped unexpectedly"),
                }
            } => Err(failure),
            () = stop_signal.cancelled() => Ok(()),
        }
    }
}

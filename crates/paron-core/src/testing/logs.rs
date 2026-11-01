use std::fmt::Debug;

use tokio::sync::watch;
use tracing::dispatcher::DefaultGuard;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Metadata, Subscriber};

/// The log lines that the current thread writes from [`CapturedLogs::capture`] until the value
/// is dropped.
pub struct CapturedLogs {
    recorded_lines: watch::Receiver<Vec<String>>,
    _default_subscriber: DefaultGuard,
}

impl CapturedLogs {
    pub fn capture() -> Self {
        let (recorded_lines_tx, recorded_lines) = watch::channel(Vec::new());

        Self {
            _default_subscriber: tracing::subscriber::set_default(Recorder {
                recorded_lines: recorded_lines_tx,
            }),
            recorded_lines,
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.recorded_lines.borrow().clone()
    }

    pub async fn wait_for_line(&self, matches: impl Fn(&str) -> bool) -> String {
        self.recorded_lines
            .clone()
            .wait_for(|lines| lines.iter().any(|line| matches(line)))
            .await
            .unwrap()
            .iter()
            .find(|line| matches(line))
            .unwrap()
            .clone()
    }
}

struct Recorder {
    recorded_lines: watch::Sender<Vec<String>>,
}

impl Subscriber for Recorder {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut line = Line(String::new());
        event.record(&mut line);

        self.recorded_lines
            .send_modify(|recorded_lines| recorded_lines.push(line.0));
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

/// The message first, then each field as `name=value`.
struct Line(String);

impl Visit for Line {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?} {}", self.0);
        } else {
            self.0 = format!("{} {}={value:?}", self.0, field.name());
        }
    }
}

use async_trait::async_trait;

use crate::application::events::AnyEvent;
use crate::domain::entity::EntityKind;

#[derive(Debug, thiserror::Error)]
#[error("the event stream failed: {0}")]
pub struct EventStreamError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

#[async_trait]
pub trait Subscription: Send {
    async fn next(&mut self) -> Result<AnyEvent, EventStreamError>;
}

#[cfg_attr(test, mockall::automock)]
pub trait EventPublisher: Send + Sync {
    fn publish(&self, event: &AnyEvent);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionScope {
    One(uuid::Uuid),
    All,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EventStream: Send + Sync {
    async fn subscribe(
        &self,
        entity: EntityKind,
        scope: SubscriptionScope,
    ) -> Result<Box<dyn Subscription>, EventStreamError>;
}

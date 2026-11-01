use std::marker::PhantomData;
use std::sync::Arc;

use super::event::{AnyEvent, EntityEvent, Event};
use super::ports::{
    EventPublisher, EventStream, EventStreamError, Subscription, SubscriptionScope,
};
use crate::application::clock::Clock;

pub struct EventSubscription<K: EntityEvent> {
    events: Box<dyn Subscription>,
    kind: PhantomData<K>,
}

impl<K: EntityEvent> EventSubscription<K>
where
    Event<K>: TryFrom<AnyEvent>,
{
    pub async fn next(&mut self) -> Result<Event<K>, EventStreamError> {
        Event::try_from(self.events.next().await?)
            .map_err(|_| EventStreamError("an event of a different kind of entity".into()))
    }
}

pub struct EventService {
    publisher: Arc<dyn EventPublisher>,
    stream: Arc<dyn EventStream>,
    clock: Arc<dyn Clock>,
}

impl EventService {
    pub fn new(
        publisher: Arc<dyn EventPublisher>,
        stream: Arc<dyn EventStream>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            publisher,
            stream,
            clock,
        }
    }

    pub fn publish<K: EntityEvent>(&self, entity_id: uuid::Uuid, kind: K)
    where
        Event<K>: Into<AnyEvent>,
    {
        self.publisher.publish(
            &Event {
                id: uuid::Uuid::now_v7(),
                created_at: self.clock.now(),
                entity_id,
                kind,
            }
            .into(),
        );
    }

    pub async fn subscribe<K: EntityEvent>(
        &self,
        scope: SubscriptionScope,
    ) -> Result<EventSubscription<K>, EventStreamError> {
        Ok(EventSubscription {
            events: self.stream.subscribe(K::ENTITY, scope).await?,
            kind: PhantomData,
        })
    }
}

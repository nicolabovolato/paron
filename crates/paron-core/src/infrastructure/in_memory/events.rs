use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::application::events::{
    AnyEvent, EventPublisher, EventStream, EventStreamError, Subscription, SubscriptionScope,
};
use crate::domain::entity::{EntityKind, EntityRef};

#[derive(Default)]
pub struct InMemoryEventStream {
    subscribers: Mutex<Vec<Subscriber>>,
}

struct Subscriber {
    entity: EntityKind,
    scope: SubscriptionScope,
    events: mpsc::UnboundedSender<AnyEvent>,
}

impl Subscriber {
    fn includes(&self, entity: EntityRef) -> bool {
        self.entity == entity.kind
            && match self.scope {
                SubscriptionScope::One(id) => id == entity.id,
                SubscriptionScope::All => true,
            }
    }
}

impl EventPublisher for InMemoryEventStream {
    fn publish(&self, event: &AnyEvent) {
        self.subscribers.lock().retain(|subscriber| {
            if subscriber.includes(event.entity()) {
                subscriber.events.send(event.clone()).is_ok()
            } else {
                !subscriber.events.is_closed()
            }
        });
    }
}

#[async_trait]
impl EventStream for InMemoryEventStream {
    async fn subscribe(
        &self,
        entity: EntityKind,
        scope: SubscriptionScope,
    ) -> Result<Box<dyn Subscription>, EventStreamError> {
        let (events, received) = mpsc::unbounded_channel();
        self.subscribers.lock().push(Subscriber {
            entity,
            scope,
            events,
        });

        Ok(Box::new(received))
    }
}

#[async_trait]
impl Subscription for mpsc::UnboundedReceiver<AnyEvent> {
    async fn next(&mut self) -> Result<AnyEvent, EventStreamError> {
        self.recv()
            .await
            .ok_or_else(|| EventStreamError("the stream of events ended".into()))
    }
}

#[cfg(test)]
mod tests {
    use futures_util::FutureExt as _;

    use super::*;
    use crate::application::events::Event;
    use crate::application::identity::event::EventKind;
    use crate::application::machine;
    use crate::domain::entity::EntityKind;

    /// Each call gives the event of a new user.
    fn sessions_deleted_event() -> AnyEvent {
        Event {
            id: uuid::Uuid::now_v7(),
            created_at: jiff::Timestamp::UNIX_EPOCH,
            entity_id: uuid::Uuid::now_v7(),
            kind: EventKind::SessionsDeleted {
                session_ids: vec![uuid::Uuid::now_v7()],
            },
        }
        .into()
    }

    #[tokio::test]
    async fn publish_gives_a_subscriber_only_the_events_of_its_entity() {
        let stream = InMemoryEventStream::default();
        let event_of_other_user = sessions_deleted_event();
        let event_of_user = sessions_deleted_event();
        let mut subscription = stream
            .subscribe(
                EntityKind::User,
                SubscriptionScope::One(event_of_user.entity().id),
            )
            .await
            .unwrap();

        stream.publish(&event_of_other_user);
        stream.publish(&event_of_user);

        assert_eq!(subscription.next().await.unwrap(), event_of_user);
    }

    #[tokio::test]
    async fn publish_gives_a_subscriber_to_a_kind_the_events_of_each_entity_of_that_kind() {
        let stream = InMemoryEventStream::default();
        let machine_deleted = AnyEvent::from(Event {
            id: uuid::Uuid::now_v7(),
            created_at: jiff::Timestamp::UNIX_EPOCH,
            entity_id: uuid::Uuid::now_v7(),
            kind: machine::event::EventKind::Deleted,
        });
        let event_of_one_user = sessions_deleted_event();
        let event_of_another_user = sessions_deleted_event();
        let mut subscription = stream
            .subscribe(EntityKind::User, SubscriptionScope::All)
            .await
            .unwrap();

        stream.publish(&machine_deleted);
        stream.publish(&event_of_one_user);
        stream.publish(&event_of_another_user);

        assert_eq!(subscription.next().await.unwrap(), event_of_one_user);
        assert_eq!(subscription.next().await.unwrap(), event_of_another_user);
    }

    #[tokio::test]
    async fn subscribe_does_not_give_an_event_that_was_published_before() {
        let stream = InMemoryEventStream::default();
        let event = sessions_deleted_event();
        stream.publish(&event);

        let mut subscription = stream
            .subscribe(EntityKind::User, SubscriptionScope::One(event.entity().id))
            .await
            .unwrap();

        assert!(subscription.next().now_or_never().is_none());
    }

    #[tokio::test]
    async fn publish_after_a_subscriber_left_removes_it() {
        let stream = InMemoryEventStream::default();
        let event = sessions_deleted_event();
        drop(
            stream
                .subscribe(EntityKind::User, SubscriptionScope::One(event.entity().id))
                .await
                .unwrap(),
        );

        stream.publish(&event);

        assert!(stream.subscribers.lock().is_empty());
    }
}

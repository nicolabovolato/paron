use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use async_trait::async_trait;
use secrecy::SecretString;

use super::ports::SessionRepository;
use crate::application::clock::Clock;
use crate::application::events::EventService;
use crate::application::identity::authorization::actor::Actor;
use crate::application::identity::errors::TouchSessionError;
use crate::application::identity::event::EventKind;
use crate::application::identity::use_cases::Sessions;
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _};
use crate::application::tokens::Tokens;
use crate::domain::token_mac::TokenMac;

#[derive(Clone)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct Session {
    pub id: uuid::Uuid,
    pub user_id: uuid::Uuid,
    pub mac: TokenMac,
    pub issued_at: jiff::Timestamp,
    pub recently_authenticated_until: jiff::Timestamp,
    pub last_used_at: jiff::Timestamp,
    pub expires_at: jiff::Timestamp,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session ended")]
    Ended,

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Copy)]
pub struct SessionPolicy {
    pub lifetime: Duration,
    pub max_idle_timeout: Duration,
    pub touch_interval: Duration,
    pub expired_sessions_per_delete_batch: usize,
    pub recent_authentication_window: Duration,
    pub max_sessions_per_user: usize,
}

pub struct SessionService<Tx> {
    repository: Arc<dyn SessionRepository<Tx>>,
    tokens: Arc<dyn Tokens>,
    clock: Arc<dyn Clock>,
    unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
    events: Arc<EventService>,
    policy: SessionPolicy,
}

impl<Tx: Send + Sync + 'static> SessionService<Tx> {
    const SESSION_IDS_PER_EVENT: usize = 100;

    pub fn new(
        repository: Arc<dyn SessionRepository<Tx>>,
        tokens: Arc<dyn Tokens>,
        clock: Arc<dyn Clock>,
        unit_of_work: Arc<dyn UnitOfWork<Tx = Tx>>,
        events: Arc<EventService>,
        policy: SessionPolicy,
    ) -> Self {
        Self {
            repository,
            tokens,
            clock,
            unit_of_work,
            events,
            policy,
        }
    }

    pub async fn create(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<(SecretString, Session), anyhow::Error> {
        let now = self.clock.now();
        let (token, mac, recently_authenticated_until) = self.issue_token(now);
        let (session, ended) = self
            .unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let session = self
                    .repository
                    .create(
                        ctx,
                        &Session {
                            id: uuid::Uuid::now_v7(),
                            user_id,
                            mac,
                            issued_at: now,
                            recently_authenticated_until,
                            last_used_at: now,
                            expires_at: now + self.policy.lifetime,
                        },
                    )
                    .await
                    .context("could not create session")?;
                let ended = self
                    .repository
                    .delete_for_user(ctx, user_id, self.policy.max_sessions_per_user, None)
                    .await
                    .context("could not end the oldest sessions")?;

                Ok::<_, anyhow::Error>((session, ended))
            })
            .await?;

        self.publish_deleted(&ended);

        Ok((token, session))
    }

    pub async fn renew(
        &self,
        ctx: &mut TxContext<Tx>,
        session_id: uuid::Uuid,
    ) -> Result<(SecretString, Session), SessionError> {
        let now = self.clock.now();
        let (token, mac, recently_authenticated_until) = self.issue_token(now);

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let session = self.get_active(ctx, session_id, now).await?;
                let session = self
                    .repository
                    .update(
                        ctx,
                        &Session {
                            mac,
                            recently_authenticated_until,
                            ..session
                        },
                    )
                    .await
                    .context("could not update session")?;

                Ok((token, session))
            })
            .await
    }

    pub async fn get_by_token(
        &self,
        ctx: &mut TxContext<Tx>,
        token: &SecretString,
    ) -> Result<Session, SessionError> {
        match self
            .repository
            .get_by_mac(ctx, &self.tokens.mac(token))
            .await
        {
            Ok(session) => Ok(session),
            Err(RepositoryError::NotFound) => Err(SessionError::Ended),
            Err(error) => Err(anyhow::Error::new(error)
                .context("could not get session by mac")
                .into()),
        }
    }

    pub async fn touch(
        &self,
        ctx: &mut TxContext<Tx>,
        session_id: uuid::Uuid,
    ) -> Result<(), SessionError> {
        let now = self.clock.now();

        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let session = self.get_active(ctx, session_id, now).await?;
                if now < session.last_used_at + self.policy.touch_interval {
                    return Ok(());
                }

                self.repository
                    .update(
                        ctx,
                        &Session {
                            last_used_at: now,
                            ..session
                        },
                    )
                    .await
                    .context("could not update session")?;

                Ok(())
            })
            .await
    }

    pub async fn delete(
        &self,
        ctx: &mut TxContext<Tx>,
        session_id: uuid::Uuid,
    ) -> Result<Session, SessionError> {
        let session = match self.repository.delete(ctx, session_id).await {
            Ok(session) => session,
            Err(RepositoryError::NotFound) => return Err(SessionError::Ended),
            Err(error) => {
                return Err(anyhow::Error::new(error)
                    .context("could not delete session")
                    .into());
            }
        };

        self.publish_deleted(std::slice::from_ref(&session));

        Ok(session)
    }

    pub async fn delete_for_user(
        &self,
        ctx: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
        except: Option<uuid::Uuid>,
    ) -> Result<Vec<Session>, anyhow::Error> {
        let deleted = self
            .repository
            .delete_for_user(ctx, user_id, 0, except)
            .await
            .context("could not delete sessions for user")?;

        self.publish_deleted(&deleted);

        Ok(deleted)
    }

    pub async fn delete_expired(&self, ctx: &mut TxContext<Tx>) -> Result<(), anyhow::Error> {
        loop {
            let now = self.clock.now();
            let unused_since = now - self.policy.max_idle_timeout;
            let expired = self
                .repository
                .delete_expired(
                    ctx,
                    now,
                    unused_since,
                    self.policy.expired_sessions_per_delete_batch,
                )
                .await?;
            let is_last_batch = expired.len() < self.policy.expired_sessions_per_delete_batch;
            self.publish_deleted(&expired);

            if is_last_batch {
                return Ok(());
            }
        }
    }

    fn issue_token(&self, now: jiff::Timestamp) -> (SecretString, TokenMac, jiff::Timestamp) {
        let (token, mac) = self.tokens.issue();

        (token, mac, now + self.policy.recent_authentication_window)
    }

    async fn get_active(
        &self,
        ctx: &mut TxContext<Tx>,
        session_id: uuid::Uuid,
        now: jiff::Timestamp,
    ) -> Result<Session, SessionError> {
        match self.repository.get_by_id(ctx, session_id).await {
            Ok(session) if !self.has_ended(&session, now) => Ok(session),
            Ok(_) | Err(RepositoryError::NotFound) => Err(SessionError::Ended),
            Err(error) => Err(anyhow::Error::new(error)
                .context("could not get session by id")
                .into()),
        }
    }

    fn has_ended(&self, session: &Session, now: jiff::Timestamp) -> bool {
        now >= session.expires_at || now >= session.last_used_at + self.policy.max_idle_timeout
    }

    fn publish_deleted(&self, sessions: &[Session]) {
        let mut session_ids_by_user = BTreeMap::<uuid::Uuid, Vec<uuid::Uuid>>::new();
        for session in sessions {
            session_ids_by_user
                .entry(session.user_id)
                .or_default()
                .push(session.id);
        }

        for (user_id, session_ids) in session_ids_by_user {
            for chunk in session_ids.chunks(Self::SESSION_IDS_PER_EVENT) {
                self.events.publish(
                    user_id,
                    EventKind::SessionsDeleted {
                        session_ids: chunk.to_vec(),
                    },
                );
            }
        }
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> Sessions for SessionService<Tx> {
    async fn touch_session(&self, actor: &Actor) -> Result<(), TouchSessionError> {
        let Some(authenticated) = actor.user() else {
            return Ok(());
        };

        match Self::touch(self, &mut TxContext::none(), authenticated.session_id).await {
            Ok(()) => Ok(()),
            Err(SessionError::Ended) => Err(TouchSessionError::SessionEnded),
            Err(SessionError::Unexpected(cause)) => Err(cause.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use futures_util::FutureExt as _;

    use super::*;
    use crate::application::events::{Event, EventStream as _, SubscriptionScope};
    use crate::application::identity::authentication::ports::MockSessionRepository;
    use crate::application::tokens::MockTokens;
    use crate::domain::entity::EntityKind;
    use crate::infrastructure::in_memory::events::InMemoryEventStream;
    use crate::testing::mocks::transactions_that_commit;
    use crate::testing::{clock, events, identity};

    #[tokio::test]
    async fn touch_session_of_an_actor_whose_session_ended_returns_session_ended() {
        let user = identity::user_builder().build();
        let mut sessions = MockSessionRepository::new();
        sessions
            .expect_get_by_id()
            .once()
            .returning(|_, _| Err(RepositoryError::NotFound));
        let service = SessionService::new(
            Arc::new(sessions),
            Arc::new(MockTokens::new()),
            clock::clock_at(jiff::Timestamp::UNIX_EPOCH),
            Arc::new(transactions_that_commit()),
            events::event_service_on(
                Arc::new(InMemoryEventStream::default()),
                clock::clock_at(jiff::Timestamp::UNIX_EPOCH),
            ),
            identity::session_policy(),
        );

        let touched = service.touch_session(&identity::actor_for(user)).await;

        assert_matches!(touched, Err(TouchSessionError::SessionEnded));
    }

    #[tokio::test]
    async fn delete_expired_publishes_the_deleted_sessions() {
        let user = identity::user_builder().build();
        let expired = identity::session_for(&user);
        let mut sessions = MockSessionRepository::new();
        sessions.expect_delete_expired().once().returning({
            let expired = expired.clone();
            move |_, _, _, _| Ok(vec![expired.clone()])
        });
        let stream = Arc::new(InMemoryEventStream::default());
        let mut user_events = stream
            .subscribe(EntityKind::User, SubscriptionScope::One(user.id))
            .await
            .unwrap();
        let service = SessionService::new(
            Arc::new(sessions),
            Arc::new(MockTokens::new()),
            clock::clock_at(jiff::Timestamp::UNIX_EPOCH),
            Arc::new(transactions_that_commit()),
            events::event_service_on(stream, clock::clock_at(jiff::Timestamp::UNIX_EPOCH)),
            identity::session_policy(),
        );

        service
            .delete_expired(&mut TxContext::none())
            .await
            .unwrap();

        let event: Event<EventKind> = user_events
            .next()
            .now_or_never()
            .unwrap()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            event.kind,
            EventKind::SessionsDeleted {
                session_ids: vec![expired.id]
            }
        );
    }

    #[tokio::test]
    async fn create_past_the_limit_of_the_user_publishes_the_ended_sessions() {
        let user = identity::user_builder().build();
        let oldest = identity::session_for(&user);
        let mut sessions = MockSessionRepository::new();
        sessions
            .expect_create()
            .once()
            .returning(|_, session| Ok(session.clone()));
        sessions
            .expect_delete_for_user()
            .once()
            .withf(move |_, user_id, keep_newest, except| {
                *user_id == user.id
                    && *keep_newest == identity::session_policy().max_sessions_per_user
                    && except.is_none()
            })
            .returning({
                let oldest = oldest.clone();
                move |_, _, _, _| Ok(vec![oldest.clone()])
            });
        let mut tokens = MockTokens::new();
        tokens
            .expect_issue()
            .returning(|| (SecretString::from("a-token"), TokenMac::from(vec![1; 32])));
        let stream = Arc::new(InMemoryEventStream::default());
        let mut user_events = stream
            .subscribe(EntityKind::User, SubscriptionScope::One(user.id))
            .await
            .unwrap();
        let service = SessionService::new(
            Arc::new(sessions),
            Arc::new(tokens),
            clock::clock_at(jiff::Timestamp::UNIX_EPOCH),
            Arc::new(transactions_that_commit()),
            events::event_service_on(stream, clock::clock_at(jiff::Timestamp::UNIX_EPOCH)),
            identity::session_policy(),
        );

        service
            .create(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        let event: Event<EventKind> = user_events
            .next()
            .now_or_never()
            .unwrap()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(
            event.kind,
            EventKind::SessionsDeleted {
                session_ids: vec![oldest.id]
            }
        );
    }

    #[tokio::test]
    async fn touch_within_the_touch_interval_keeps_the_last_use() {
        let session = identity::session_for(&identity::user_builder().build());
        let mut sessions = MockSessionRepository::new();
        sessions.expect_get_by_id().returning({
            let session = session.clone();
            move |_, _| Ok(session.clone())
        });
        sessions.expect_update().never();
        let service = SessionService::new(
            Arc::new(sessions),
            Arc::new(MockTokens::new()),
            clock::clock_at(session.last_used_at + jiff::SignedDuration::from_secs(50)),
            Arc::new(transactions_that_commit()),
            events::event_service(),
            identity::session_policy(),
        );

        service
            .touch(&mut TxContext::none(), session.id)
            .await
            .unwrap();
    }
}

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::application::clock::Clock;
use crate::application::identity::authentication::methods::passkey::PasskeyRegistration;
use crate::application::identity::authentication::methods::ports::{
    PasskeyRegistrationRepository, TotpRegistrationRepository,
};
use crate::application::identity::authentication::methods::totp::TotpRegistration;
use crate::application::repository::{RepositoryError, TxContext};

/// The credential setups in progress of one process, one for each user. A restart ends them, and
/// the user starts the setup again.
pub struct InMemoryRegistrationRepository<Registration> {
    registrations: Mutex<HashMap<uuid::Uuid, Registration>>,
    clock: Arc<dyn Clock>,
}

pub trait Registration: Clone + Send {
    fn user_id(&self) -> uuid::Uuid;
    fn expires_at(&self) -> jiff::Timestamp;
}

impl Registration for PasskeyRegistration {
    fn user_id(&self) -> uuid::Uuid {
        self.user_id
    }

    fn expires_at(&self) -> jiff::Timestamp {
        self.expires_at
    }
}

impl Registration for TotpRegistration {
    fn user_id(&self) -> uuid::Uuid {
        self.user_id
    }

    fn expires_at(&self) -> jiff::Timestamp {
        self.expires_at
    }
}

impl<R: Registration> InMemoryRegistrationRepository<R> {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            registrations: Mutex::default(),
            clock,
        }
    }

    fn store(&self, registration: &R) -> R {
        let now = self.clock.now();
        let mut registrations = self.registrations.lock();

        registrations.retain(|_, stored| now < stored.expires_at());
        registrations.insert(registration.user_id(), registration.clone());

        registration.clone()
    }

    fn find(&self, user_id: uuid::Uuid) -> Result<R, RepositoryError> {
        self.registrations
            .lock()
            .get(&user_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)
    }

    fn take(&self, user_id: uuid::Uuid) -> Result<R, RepositoryError> {
        self.registrations
            .lock()
            .remove(&user_id)
            .ok_or(RepositoryError::NotFound)
    }

    pub fn remove_expired(&self) {
        let now = self.clock.now();

        self.registrations
            .lock()
            .retain(|_, stored| now < stored.expires_at());
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> PasskeyRegistrationRepository<Tx>
    for InMemoryRegistrationRepository<PasskeyRegistration>
{
    async fn upsert(
        &self,
        _: &mut TxContext<Tx>,
        registration: &PasskeyRegistration,
    ) -> Result<PasskeyRegistration, RepositoryError> {
        Ok(self.store(registration))
    }

    async fn delete(
        &self,
        _: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<PasskeyRegistration, RepositoryError> {
        self.take(user_id)
    }
}

#[async_trait]
impl<Tx: Send + Sync + 'static> TotpRegistrationRepository<Tx>
    for InMemoryRegistrationRepository<TotpRegistration>
{
    async fn upsert(
        &self,
        _: &mut TxContext<Tx>,
        registration: &TotpRegistration,
    ) -> Result<TotpRegistration, RepositoryError> {
        Ok(self.store(registration))
    }

    async fn get(
        &self,
        _: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<TotpRegistration, RepositoryError> {
        self.find(user_id)
    }

    async fn delete(
        &self,
        _: &mut TxContext<Tx>,
        user_id: uuid::Uuid,
    ) -> Result<TotpRegistration, RepositoryError> {
        self.take(user_id)
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::application::identity::authentication::methods::passkey::Challenge;
    use crate::testing::clock::{clock_at, timestamp};

    fn registration(
        user_id: uuid::Uuid,
        challenge_byte: u8,
        expires_at: i64,
    ) -> PasskeyRegistration {
        PasskeyRegistration {
            user_id,
            challenge: Challenge([challenge_byte; 32]),
            expires_at: timestamp(expires_at),
        }
    }

    #[tokio::test]
    async fn delete_after_two_upserts_returns_the_second_then_not_found() {
        let repository = InMemoryRegistrationRepository::new(clock_at(timestamp(10)));
        let user_id = uuid::Uuid::now_v7();
        for challenge_byte in [1, 2] {
            PasskeyRegistrationRepository::<()>::upsert(
                &repository,
                &mut TxContext::none(),
                &registration(user_id, challenge_byte, 100),
            )
            .await
            .unwrap();
        }

        let first_delete = PasskeyRegistrationRepository::<()>::delete(
            &repository,
            &mut TxContext::none(),
            user_id,
        )
        .await;
        let second_delete = PasskeyRegistrationRepository::<()>::delete(
            &repository,
            &mut TxContext::none(),
            user_id,
        )
        .await;

        assert_eq!(first_delete.unwrap(), registration(user_id, 2, 100));
        assert_matches!(second_delete, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn upsert_removes_the_expired_registrations() {
        let repository = InMemoryRegistrationRepository::new(clock_at(timestamp(50)));
        let expired = registration(uuid::Uuid::now_v7(), 1, 20);
        PasskeyRegistrationRepository::<()>::upsert(&repository, &mut TxContext::none(), &expired)
            .await
            .unwrap();

        PasskeyRegistrationRepository::<()>::upsert(
            &repository,
            &mut TxContext::none(),
            &registration(uuid::Uuid::now_v7(), 2, 100),
        )
        .await
        .unwrap();

        let expired_delete = PasskeyRegistrationRepository::<()>::delete(
            &repository,
            &mut TxContext::none(),
            expired.user_id,
        )
        .await;
        assert_matches!(expired_delete, Err(RepositoryError::NotFound));
    }
}

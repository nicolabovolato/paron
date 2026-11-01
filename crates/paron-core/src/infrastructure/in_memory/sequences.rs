use std::collections::HashMap;
use std::collections::hash_map::Entry;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::application::repository::RepositoryError;
use crate::infrastructure::transport::tunnel::SequenceStore;

#[derive(Default)]
pub struct InMemorySequenceStore(Mutex<HashMap<uuid::Uuid, u64>>);

impl InMemorySequenceStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SequenceStore for InMemorySequenceStore {
    async fn advance(&self, machine: uuid::Uuid, sequence: u64) -> Result<bool, RepositoryError> {
        let mut marks = self.0.lock();

        Ok(match marks.entry(machine) {
            Entry::Occupied(mut mark) if sequence > *mark.get() => {
                mark.insert(sequence);

                true
            }
            Entry::Occupied(_) => false,
            Entry::Vacant(mark) => {
                mark.insert(sequence);

                true
            }
        })
    }

    async fn clear(&self, machine: uuid::Uuid) -> Result<(), RepositoryError> {
        self.0.lock().remove(&machine);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn advance_higher_sequence_returns_true() {
        let store = InMemorySequenceStore::new();
        let machine = uuid::Uuid::now_v7();

        for sequence in [1, 2, 900, u64::MAX] {
            let advanced = store.advance(machine, sequence).await.unwrap();

            assert!(advanced, "{sequence}");
        }
    }

    #[tokio::test]
    async fn advance_repeated_or_lower_sequence_returns_false_and_keeps_the_mark() {
        let store = InMemorySequenceStore::new();
        let machine = uuid::Uuid::now_v7();
        store.advance(machine, 50).await.unwrap();

        for sequence in [50, 4] {
            let advanced = store.advance(machine, sequence).await.unwrap();

            assert!(!advanced, "{sequence}");
        }
        assert!(store.advance(machine, 51).await.unwrap());
    }

    #[tokio::test]
    async fn advance_ignores_the_mark_of_another_machine() {
        let store = InMemorySequenceStore::new();
        let machine = uuid::Uuid::now_v7();
        store.advance(uuid::Uuid::now_v7(), 50).await.unwrap();

        let advanced = store.advance(machine, 1).await.unwrap();

        assert!(advanced);
    }

    #[tokio::test]
    async fn advance_after_clear_returns_true() {
        let store = InMemorySequenceStore::new();
        let machine = uuid::Uuid::now_v7();
        store.advance(machine, u64::MAX).await.unwrap();
        store.clear(machine).await.unwrap();

        let advanced = store.advance(machine, 1).await.unwrap();

        assert!(advanced);
    }
}

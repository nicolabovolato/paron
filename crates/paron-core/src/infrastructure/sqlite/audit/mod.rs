pub mod repository;

use crate::application::audit::{Actor, AuditEntry, AuditKind, Entity, UnknownAuditKind};
use crate::domain::entity::{EntityKind, EntityRef};
use crate::infrastructure::audit::TypedPayload;

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(sqlx::Type, Clone, Copy)]
#[sqlx(rename_all = "snake_case")]
enum ActorType {
    System,
    User,
    Machine,
}

impl From<EntityKind> for ActorType {
    fn from(kind: EntityKind) -> Self {
        match kind {
            EntityKind::User => Self::User,
            EntityKind::Machine => Self::Machine,
        }
    }
}

#[derive(sqlx::Type, serde::Serialize, Clone, Copy)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
enum TargetType {
    User,
    Machine,
}

impl From<EntityKind> for TargetType {
    fn from(kind: EntityKind) -> Self {
        match kind {
            EntityKind::User => Self::User,
            EntityKind::Machine => Self::Machine,
        }
    }
}

impl From<TargetType> for EntityKind {
    fn from(target_type: TargetType) -> Self {
        match target_type {
            TargetType::User => Self::User,
            TargetType::Machine => Self::Machine,
        }
    }
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: uuid::Uuid,
    created_at_unix_ms: i64,

    actor_type: ActorType,
    actor_id: Option<uuid::Uuid>,
    actor_name: Option<String>,

    target_type: TargetType,
    target_id: uuid::Uuid,
    target_name: String,

    #[sqlx(rename = "type")]
    entry_type: String,
    payload: sqlx::types::Json<serde_json::Value>,
}

impl AuditRow {
    fn from_entity(entry: &AuditEntry) -> Self {
        let TypedPayload {
            entry_type,
            payload,
        } = TypedPayload::from(&entry.kind);
        let (actor_type, actor_id, actor_name) = match &entry.actor {
            Actor::System => (ActorType::System, None, None),
            Actor::Entity(actor) => (
                ActorType::from(actor.reference.kind),
                Some(actor.reference.id),
                Some(actor.name.clone()),
            ),
        };

        Self {
            id: entry.id,
            created_at_unix_ms: entry.created_at.as_millisecond(),

            actor_type,
            actor_id,
            actor_name,

            target_type: TargetType::from(entry.target.reference.kind),
            target_id: entry.target.reference.id,
            target_name: entry.target.name.clone(),

            entry_type,
            payload: sqlx::types::Json(payload),
        }
    }

    fn try_into_entity(self) -> Result<AuditEntry<Result<AuditKind, UnknownAuditKind>>, Error> {
        use anyhow::Context as _;

        const NULL: &str = "a column required by the actor type is null";

        let actor_kind = match self.actor_type {
            ActorType::System => None,
            ActorType::User => Some(EntityKind::User),
            ActorType::Machine => Some(EntityKind::Machine),
        };
        let actor = match actor_kind {
            None => Actor::System,
            Some(kind) => Actor::Entity(Entity {
                reference: EntityRef {
                    kind,
                    id: self.actor_id.context(NULL)?,
                },
                name: self.actor_name.context(NULL)?,
            }),
        };

        let typed = TypedPayload {
            entry_type: self.entry_type,
            payload: self.payload.0,
        };
        let kind = AuditKind::try_from(&typed).map_err(|error| {
            tracing::warn!(
                "could not read the payload of audit entry {}: {error}",
                self.id
            );

            UnknownAuditKind {
                entry_type: typed.entry_type,
                payload: typed.payload.to_string(),
            }
        });

        Ok(AuditEntry {
            id: self.id,
            created_at: jiff::Timestamp::from_millisecond(self.created_at_unix_ms)?,
            actor,
            target: Entity {
                reference: EntityRef {
                    kind: self.target_type.into(),
                    id: self.target_id,
                },
                name: self.target_name,
            },
            kind,
        })
    }
}

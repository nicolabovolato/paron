use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_qs::axum::QsQuery;
use serde_utoipa_validate::{newtype, validate};
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::application::audit::{
    Actor, AuditEntry, AuditEntryType, AuditKind, AuditOrder, AuditQuery, Entity, EntityFilter,
    EntityRole, ListAuditEntriesError, UnknownAuditKind,
};
use crate::domain::entity::{EntityKind, EntityRef};
use crate::infrastructure::audit::AuditPayload;
use crate::infrastructure::http::auth::CurrentUser;
use crate::infrastructure::http::error;
use crate::infrastructure::http::error::ApiError;
use crate::infrastructure::http::pagination::Limit;
use crate::infrastructure::http::state::Deps;

pub fn routes() -> OpenApiRouter<Deps> {
    OpenApiRouter::new().routes(routes!(list))
}

#[utoipa::path(
    get,
    path = "/audit",
    tag = "audit",
    operation_id = "list_audit_entries",
    params(AuditListQuery),
    responses((status = OK, body = AuditPageResponse)),
    security(("session" = [])),
)]
async fn list(
    State(deps): State<Deps>,
    user: CurrentUser,
    QsQuery(query): QsQuery<AuditListQuery>,
) -> error::Result<impl IntoResponse> {
    let query = query.validated()?;
    let entries = deps.audit.list(&user.into(), &query.into()).await?;

    Ok(Json(AuditPageResponse {
        items: entries.into_iter().map(Into::into).collect(),
    }))
}

newtype! {
    pub Order: String -> AuditOrder {
        NewestFirst => "newest_first",
        OldestFirst => "oldest_first",
    }
}

newtype! {
    pub EntityRoleParam: String -> EntityRole {
        ActorOrTarget => "actor_or_target",
        Actor => "actor",
        Target => "target",
    }
}

newtype! {
    pub EntityType: String -> EntityKind {
        User => "user",
        Machine => "machine",
    }
}

newtype! {
    pub EntryType: String -> AuditEntryType {
        schema: ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(
                AuditEntryType::ALL.iter().map(|entry_type| entry_type.as_str()),
            )),
        deserialize: |raw: String| raw.parse::<AuditEntryType>(),
        serialize: |entry_type: &AuditEntryType| Ok(entry_type.as_str()),
    }
}

#[validate]
#[derive(Debug, Deserialize, ToSchema)]
pub struct EntityParam {
    #[serde(rename = "type")]
    #[schema(inline)]
    entity_type: EntityType,
    id: uuid::Uuid,
    #[validate(default = EntityRole::ActorOrTarget)]
    #[schema(inline)]
    role: EntityRoleParam,
}

impl From<EntityParamParsed> for EntityFilter {
    fn from(entity: EntityParamParsed) -> Self {
        Self {
            entity: EntityRef {
                kind: entity.entity_type,
                id: entity.id,
            },
            role: entity.role,
        }
    }
}

#[validate]
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AuditListQuery {
    cursor: Option<uuid::Uuid>,
    from: Option<jiff::Timestamp>,
    to: Option<jiff::Timestamp>,
    #[param(style = DeepObject, explode)]
    entity: Option<EntityParam>,
    #[serde(rename = "type")]
    #[schema(inline)]
    entry_type: Option<EntryType>,
    #[validate(default)]
    limit: Limit,
    #[validate(default = AuditOrder::NewestFirst)]
    order: Order,
}

impl From<AuditListQueryParsed> for AuditQuery {
    fn from(query: AuditListQueryParsed) -> Self {
        Self {
            from: query.from,
            to: query.to,
            entity: query.entity.map(Into::into),
            entry_type: query.entry_type,
            order: query.order,
            cursor: query.cursor,
            limit: query.limit,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct AuditPageResponse {
    pub items: Vec<AuditEntryResponse>,
}

#[derive(Serialize, ToSchema)]
pub struct AuditEntryResponse {
    pub id: uuid::Uuid,
    pub created_at: jiff::Timestamp,
    pub actor: AuditActorResponse,
    pub target: AuditEntityResponse,
    pub event: AuditEventResponse,
}

impl From<AuditEntry<Result<AuditKind, UnknownAuditKind>>> for AuditEntryResponse {
    fn from(entry: AuditEntry<Result<AuditKind, UnknownAuditKind>>) -> Self {
        Self {
            id: entry.id,
            created_at: entry.created_at,
            actor: entry.actor.into(),
            target: entry.target.into(),
            event: entry.kind.into(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditActorResponse {
    System,
    User { id: uuid::Uuid, name: String },
    Machine { id: uuid::Uuid, name: String },
}

impl From<Actor> for AuditActorResponse {
    fn from(actor: Actor) -> Self {
        match actor {
            Actor::System => Self::System,
            Actor::Entity(Entity { reference, name }) => match reference.kind {
                EntityKind::User => Self::User {
                    id: reference.id,
                    name,
                },
                EntityKind::Machine => Self::Machine {
                    id: reference.id,
                    name,
                },
            },
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditEntityResponse {
    #[serde(rename = "type")]
    pub entity_type: EntityType,
    pub id: uuid::Uuid,
    pub name: String,
}

impl From<Entity> for AuditEntityResponse {
    fn from(entity: Entity) -> Self {
        Self {
            entity_type: entity.reference.kind.into(),
            id: entity.reference.id,
            name: entity.name,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(untagged)]
pub enum AuditEventResponse {
    Known(AuditPayload),
    Unknown(UnknownAuditEvent),
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "type", content = "payload")]
pub enum UnknownAuditEvent {
    #[serde(rename = "unknown")]
    Unknown { entry_type: String, payload: String },
}

impl From<Result<AuditKind, UnknownAuditKind>> for AuditEventResponse {
    fn from(kind: Result<AuditKind, UnknownAuditKind>) -> Self {
        match kind {
            Ok(kind) => Self::Known(AuditPayload::from(&kind)),
            Err(UnknownAuditKind {
                entry_type,
                payload,
            }) => Self::Unknown(UnknownAuditEvent::Unknown {
                entry_type,
                payload,
            }),
        }
    }
}

impl From<ListAuditEntriesError> for ApiError {
    fn from(error: ListAuditEntriesError) -> Self {
        match error {
            ListAuditEntriesError::Unexpected(cause) => cause.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use serde_json::json;

    use super::*;
    use crate::application::audit::MockAuditReader;
    use crate::infrastructure::http::router;
    use crate::testing::events;
    use crate::testing::http::{self, SESSION_COOKIE};

    #[tokio::test]
    async fn list_returns_the_entries() {
        let mut audit = MockAuditReader::new();
        audit
            .expect_list()
            .once()
            .withf(|actor, _| http::is_signed_in_user(actor))
            .returning(|_, _| {
                let entry = events::user_deleted_at(jiff::Timestamp::UNIX_EPOCH);

                Ok(vec![AuditEntry {
                    id: entry.id,
                    created_at: entry.created_at,
                    actor: entry.actor,
                    target: entry.target,
                    kind: Ok(entry.kind),
                }])
            });
        let api = router(Deps {
            audit: Arc::new(audit),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::get("/api/audit")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["items"][0]["event"],
            json!({ "type": "user.deleted", "payload": {} })
        );
        assert_eq!(body["items"][0]["actor"]["type"], "user");
        assert_eq!(body["items"][0]["target"]["name"], "giulia");
    }

    #[tokio::test]
    async fn list_by_entity_and_role_passes_the_entity_filter() {
        let user_id = uuid::Uuid::now_v7();
        let mut audit = MockAuditReader::new();
        audit
            .expect_list()
            .once()
            .withf(move |_, query| {
                query.entity
                    == Some(EntityFilter {
                        entity: EntityRef {
                            kind: EntityKind::User,
                            id: user_id,
                        },
                        role: EntityRole::Actor,
                    })
            })
            .returning(|_, _| Ok(Vec::new()));
        let api = router(Deps {
            audit: Arc::new(audit),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::get(format!(
                "/api/audit?entity[type]=user&entity[id]={user_id}&entity[role]=actor"
            ))
            .header(header::COOKIE, SESSION_COOKIE)
            .body(Body::empty())
            .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn list_by_type_passes_the_entry_type() {
        let mut audit = MockAuditReader::new();
        audit
            .expect_list()
            .once()
            .withf(|_, query| query.entry_type == Some(AuditEntryType::MachineTerminalOpened))
            .returning(|_, _| Ok(Vec::new()));
        let api = router(Deps {
            audit: Arc::new(audit),
            ..http::deps()
        });

        let (status, body) = http::send(
            &api,
            Request::get("/api/audit?type=machine.terminal_opened")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn list_with_percent_encoded_brackets_reads_the_entity() {
        let (status, body) = http::send(
            &router(http::deps()),
            Request::get("/api/audit?entity%5Btype%5D=robot")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(
            body["validation_errors"]["entity.type"].is_array(),
            "{body}"
        );
    }

    #[tokio::test]
    async fn list_with_entity_id_and_no_type_returns_unprocessable_entity() {
        let (status, body) = http::send(
            &router(http::deps()),
            Request::get(format!("/api/audit?entity[id]={}", uuid::Uuid::now_v7()))
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(
            body["validation_errors"],
            json!({ "entity.type": ["this field is required"] })
        );
    }

    #[tokio::test]
    async fn list_with_cursor_that_is_not_an_id_returns_unprocessable_entity() {
        let (status, body) = http::send(
            &router(http::deps()),
            Request::get("/api/audit?cursor=nope")
                .header(header::COOKIE, SESSION_COOKIE)
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(body["validation_errors"]["cursor"].is_array(), "{body}");
    }
}

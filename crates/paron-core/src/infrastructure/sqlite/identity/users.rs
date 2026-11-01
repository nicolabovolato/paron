use async_trait::async_trait;
use indoc::{formatdoc, indoc};

use super::UserRow;
use crate::application::identity::ports::{UserFilter, UserRepository};
use crate::application::repository::TxGuarantee::ReadModifyWrite;
use crate::application::repository::{
    ListQuery, Paginated, RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _,
};
use crate::domain::user::{User, Username};
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct SqliteUserRepository {
    pool: sqlx::SqlitePool,
    unit_of_work: std::sync::Arc<dyn UnitOfWork<Tx = SqliteTx>>,
}

impl SqliteRepository for SqliteUserRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl SqliteUserRepository {
    pub fn new(
        pool: sqlx::SqlitePool,
        unit_of_work: std::sync::Arc<dyn UnitOfWork<Tx = SqliteTx>>,
    ) -> Self {
        Self { pool, unit_of_work }
    }

    fn to_entity(row: UserRow) -> Result<User, RepositoryError> {
        let id = row.id;

        row.try_into_entity().map_err(|error| {
            RepositoryError::Unexpected(
                anyhow::Error::from_boxed(error).context(format!("error reading user {id}")),
            )
        })
    }

    async fn write_roles(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        row: &UserRow,
    ) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM user_roles WHERE user_id = ?1")
            .bind(row.id)
            .execute(self.executing(ctx))
            .await?;

        sqlx::query("INSERT INTO user_roles (user_id, role) SELECT ?1, value FROM json_each(?2)")
            .bind(row.id)
            .bind(&row.roles)
            .execute(self.executing(ctx))
            .await?;

        Ok(())
    }
}

#[async_trait]
impl UserRepository<SqliteTx> for SqliteUserRepository {
    async fn list(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        query: &ListQuery,
    ) -> Result<Paginated<User>, RepositoryError> {
        let search_filter = query.search.as_ref().map_or(
            "TRUE",
            |_| "instr(lower(username), lower(?1)) > 0 OR instr(lower(display_name), lower(?1)) > 0",
        );

        let total: u64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM users WHERE ({search_filter})"
        )))
        .bind(query.search.as_deref()) // ?1
        .fetch_one(self.executing(ctx))
        .await?;

        let rows: Vec<UserRow> = sqlx::query_as(sqlx::AssertSqlSafe(formatdoc! {"
            SELECT users.*,
                   (SELECT json_group_array(role) FROM user_roles
                    WHERE user_id = users.id) AS roles
            FROM users
            WHERE ({search_filter})
            ORDER BY users.username
            LIMIT ?2 OFFSET ?3
        "}))
        .bind(query.search.as_deref()) // ?1
        .bind(query.page.limit) // ?2
        .bind(query.page.offset) // ?3
        .fetch_all(self.executing(ctx))
        .await?;

        Ok(Paginated {
            items: rows
                .into_iter()
                .map(Self::to_entity)
                .collect::<Result<_, _>>()?,
            total,
        })
    }

    async fn get_by_id(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        id: uuid::Uuid,
    ) -> Result<User, RepositoryError> {
        let row: UserRow = sqlx::query_as(indoc! {"
            SELECT users.*,
                   (SELECT json_group_array(role) FROM user_roles
                    WHERE user_id = users.id) AS roles
            FROM users
            WHERE users.id = ?
        "})
        .bind(id)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn get_by_username(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        username: &Username,
    ) -> Result<User, RepositoryError> {
        let row: UserRow = sqlx::query_as(indoc! {"
            SELECT users.*,
                   (SELECT json_group_array(role) FROM user_roles
                    WHERE user_id = users.id) AS roles
            FROM users
            WHERE users.username = ?
        "})
        .bind(&**username)
        .fetch_one(self.executing(ctx))
        .await?;

        Self::to_entity(row)
    }

    async fn create(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user: &User,
    ) -> Result<User, RepositoryError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let row = UserRow::from_entity(user);

                sqlx::query(indoc! {"
                    INSERT INTO users (id, username, display_name, status, created_at_unix_ms)
                    VALUES (?1, ?2, ?3, ?4, ?5)
                "})
                .bind(row.id)
                .bind(&row.username)
                .bind(&row.display_name)
                .bind(row.status)
                .bind(row.created_at_unix_ms)
                .execute(self.executing(ctx))
                .await?;

                self.write_roles(ctx, &row).await?;

                self.get_by_id(ctx, user.id).await
            })
            .await
    }

    async fn update(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        user: &User,
    ) -> Result<User, RepositoryError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let row = UserRow::from_entity(user);

                sqlx::query(indoc! {"
                    UPDATE users SET
                        username = ?2, display_name = ?3, status = ?4
                    WHERE id = ?1
                    RETURNING id
                "})
                .bind(row.id)
                .bind(&row.username)
                .bind(&row.display_name)
                .bind(row.status)
                .fetch_one(self.executing(ctx))
                .await?;

                self.write_roles(ctx, &row).await?;

                self.get_by_id(ctx, user.id).await
            })
            .await
    }

    async fn count(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        filter: &UserFilter,
    ) -> Result<u64, RepositoryError> {
        let counted: u64 = sqlx::query_scalar(indoc! {"
            SELECT count(*) FROM users
            WHERE (?1 IS NULL OR users.status = ?1)
              AND (?2 IS NULL OR users.id != ?2)
              AND (?3 IS NULL OR users.id IN (
                    SELECT user_id FROM user_roles
                    WHERE user_roles.role IN (SELECT value FROM json_each(?3))))
        "})
        .bind(
            filter
                .status
                .map(|status| super::UserStatusKind::from(&status)),
        )
        .bind(filter.excluding)
        .bind(filter.roles.as_ref().map(|roles| {
            serde_json::Value::from_iter(roles.iter().map(|role| role.as_str())).to_string()
        }))
        .fetch_one(self.executing(ctx))
        .await?;

        Ok(counted)
    }

    async fn delete(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        id: uuid::Uuid,
    ) -> Result<User, RepositoryError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let removed = self.get_by_id(ctx, id).await?;

                sqlx::query("DELETE FROM users WHERE id = ? RETURNING id")
                    .bind(id)
                    .fetch_one(self.executing(ctx))
                    .await?;

                Ok(removed)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::collections::BTreeSet;

    use super::*;
    use crate::application::repository::{Page, SearchText};
    use crate::domain::user::{DisplayName, Role, UserStatus};
    use crate::testing::registry::Registry;
    use crate::testing::{identity, pagination};

    const NO_FILTER: UserFilter = UserFilter {
        status: None,
        roles: None,
        excluding: None,
    };

    async fn list_usernames(registry: &Registry, query: ListQuery) -> (Vec<String>, u64) {
        let users = registry
            .users
            .list(&mut TxContext::none(), &query)
            .await
            .unwrap();

        (
            users
                .items
                .iter()
                .map(|user| user.username.to_string())
                .collect(),
            users.total,
        )
    }

    #[tokio::test]
    async fn round_trip_preserves_every_field() {
        let registry = Registry::in_memory().await;
        let user = identity::user_builder()
            .roles(&[Role::Administrator, Role::Operator])
            .status(UserStatus::Suspended)
            .build();
        registry.create_user(&user).await;

        let stored = registry
            .users
            .get_by_id(&mut TxContext::none(), user.id)
            .await;

        assert_eq!(stored.unwrap(), user);
    }

    #[tokio::test]
    async fn round_trip_without_roles_preserves_the_empty_role_set() {
        let registry = Registry::in_memory().await;
        let user = identity::user_builder().roles(&[]).build();
        registry.create_user(&user).await;

        let stored = registry
            .users
            .get_by_id(&mut TxContext::none(), user.id)
            .await;

        assert_eq!(stored.unwrap(), user);
    }

    #[tokio::test]
    async fn create_with_a_taken_username_returns_already_exists() {
        let registry = Registry::in_memory().await;
        registry
            .create_user(&identity::user_builder().build())
            .await;

        let created = registry
            .users
            .create(&mut TxContext::none(), &identity::user_builder().build())
            .await;

        assert_matches!(created, Err(RepositoryError::AlreadyExists));
    }

    #[tokio::test]
    async fn get_by_username_returns_the_stored_user() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;

        let stored = registry
            .users
            .get_by_username(&mut TxContext::none(), &identity::username())
            .await;

        assert_eq!(stored.unwrap(), user);
    }

    #[tokio::test]
    async fn get_by_username_unknown_returns_not_found() {
        let registry = Registry::in_memory().await;

        let stored = registry
            .users
            .get_by_username(&mut TxContext::none(), &identity::username())
            .await;

        assert_matches!(stored, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn update_stores_the_new_fields() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;
        let updated_user = User {
            username: Username::parse("renamed").unwrap(),
            display_name: DisplayName::parse("Renamed").unwrap(),
            roles: BTreeSet::from([Role::Operator]),
            status: UserStatus::Suspended,
            ..user
        };

        registry
            .users
            .update(&mut TxContext::none(), &updated_user)
            .await
            .unwrap();

        let stored = registry
            .users
            .get_by_id(&mut TxContext::none(), updated_user.id)
            .await;
        assert_eq!(stored.unwrap(), updated_user);
    }

    #[tokio::test]
    async fn update_unknown_user_returns_not_found() {
        let registry = Registry::in_memory().await;

        let updated = registry
            .users
            .update(&mut TxContext::none(), &identity::user_builder().build())
            .await;

        assert_matches!(updated, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_removes_the_user() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;

        registry
            .users
            .delete(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        let stored = registry
            .users
            .get_by_id(&mut TxContext::none(), user.id)
            .await;
        assert_matches!(stored, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_removes_the_roles_of_the_user() {
        let registry = Registry::in_memory().await;
        let user = registry
            .create_user(&identity::user_builder().build())
            .await;

        registry
            .users
            .delete(&mut TxContext::none(), user.id)
            .await
            .unwrap();

        let user_role_count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_roles")
            .fetch_one(&registry.pool)
            .await
            .unwrap();
        assert_eq!(user_role_count, 0);
    }

    #[tokio::test]
    async fn delete_unknown_user_returns_not_found() {
        let registry = Registry::in_memory().await;

        let deleted = registry
            .users
            .delete(&mut TxContext::none(), uuid::Uuid::now_v7())
            .await;

        assert_matches!(deleted, Err(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn list_returns_the_users_ordered_by_username() {
        let registry = Registry::in_memory().await;
        registry
            .create_user(&identity::user_builder().username("zoe").build())
            .await;
        registry
            .create_user(&identity::user_builder().username("adam").build())
            .await;

        let (usernames, _) = list_usernames(&registry, pagination::everything()).await;

        assert_eq!(usernames, ["adam", "zoe"]);
    }

    #[tokio::test]
    async fn list_with_a_search_returns_the_users_whose_username_or_display_name_contains_it() {
        let registry = Registry::in_memory().await;
        for (username, display_name) in [
            ("adam", "Adam Rossi"),
            ("beth", "Beth"),
            ("car_l", "Carl"),
            ("carxl", "Carl"),
        ] {
            registry
                .create_user(
                    &identity::user_builder()
                        .username(username)
                        .display_name(display_name)
                        .build(),
                )
                .await;
        }

        for (search, expected_usernames) in [
            ("ROSS", &["adam"][..]),
            ("BETH", &["beth"]),
            ("r_l", &["car_l"]),
        ] {
            let (usernames, total) = list_usernames(
                &registry,
                ListQuery {
                    search: Some(SearchText::parse(search).unwrap()),
                    ..pagination::everything()
                },
            )
            .await;

            assert_eq!(usernames, expected_usernames, "{search}");
            assert_eq!(total, expected_usernames.len() as u64, "{search}");
        }
    }

    #[tokio::test]
    async fn list_returns_the_page_and_the_full_total() {
        let registry = Registry::in_memory().await;
        for username in ["adam", "beth", "carl"] {
            registry
                .create_user(&identity::user_builder().username(username).build())
                .await;
        }

        for (limit, offset, expected_usernames) in [
            (1, 1, &["beth"][..]),
            (2, 0, &["adam", "beth"]),
            (10, 10, &[]),
        ] {
            let (usernames, total) = list_usernames(
                &registry,
                ListQuery {
                    search: None,
                    page: Page { limit, offset },
                },
            )
            .await;

            assert_eq!(
                usernames, expected_usernames,
                "limit {limit}, offset {offset}"
            );
            assert_eq!(total, 3, "limit {limit}, offset {offset}");
        }
    }

    #[tokio::test]
    async fn count_applies_each_part_of_the_filter() {
        let registry = Registry::in_memory().await;
        let administrator = registry
            .create_user(&identity::user_builder().build())
            .await;
        registry
            .create_user(
                &identity::user_builder()
                    .username("operator")
                    .roles(&[Role::Operator])
                    .build(),
            )
            .await;
        registry
            .create_user(
                &identity::user_builder()
                    .username("suspended")
                    .status(UserStatus::Suspended)
                    .build(),
            )
            .await;

        for (part, filter, expected_count) in [
            ("no filter", NO_FILTER, 3),
            (
                "status",
                UserFilter {
                    status: Some(UserStatus::Active),
                    ..NO_FILTER
                },
                2,
            ),
            (
                "role",
                UserFilter {
                    roles: Some(BTreeSet::from([Role::Operator])),
                    ..NO_FILTER
                },
                1,
            ),
            (
                "empty role set",
                UserFilter {
                    roles: Some(BTreeSet::new()),
                    ..NO_FILTER
                },
                0,
            ),
            (
                "excluding",
                UserFilter {
                    excluding: Some(administrator.id),
                    ..NO_FILTER
                },
                2,
            ),
            (
                "status, role and excluding",
                UserFilter {
                    status: Some(UserStatus::Active),
                    roles: Some(BTreeSet::from([Role::Administrator])),
                    excluding: Some(administrator.id),
                },
                0,
            ),
        ] {
            let user_count = registry
                .users
                .count(&mut TxContext::none(), &filter)
                .await
                .unwrap();

            assert_eq!(user_count, expected_count, "{part}");
        }
    }
}

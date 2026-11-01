use std::sync::Arc;

use async_trait::async_trait;
use indoc::indoc;

use super::MachineRow;
use crate::application::machine::Repository;
use crate::application::repository::TxGuarantee::{ConsistentReads, ReadModifyWrite};
use crate::application::repository::{
    ListQuery, Paginated, RepositoryError, TxContext, UnitOfWork, UnitOfWorkExt as _,
};
use crate::domain::machine::Machine;
use crate::infrastructure::crypto::cipher::AeadCipher;
use crate::infrastructure::sqlite::{SqliteRepository, SqliteTx};

pub struct EncryptedSqliteMachineRepository {
    pool: sqlx::SqlitePool,
    unit_of_work: Arc<dyn UnitOfWork<Tx = SqliteTx>>,
    cipher: Arc<dyn AeadCipher>,
}

impl SqliteRepository for EncryptedSqliteMachineRepository {
    fn pool(&self) -> &sqlx::SqlitePool {
        &self.pool
    }
}

impl EncryptedSqliteMachineRepository {
    const SELECT: &str = indoc! {"
        SELECT machines.*,
               (SELECT json_object(
                   'machine_id', hex(machine_id),
                   'user', user,
                   'port', port,
                   'host_key', hex(host_key),
                   'authentication_method', authentication_method,
                   'password_encrypted',
                       iif(password_encrypted IS NULL, NULL, hex(password_encrypted)),
                   'keypair_private_key_encrypted',
                       iif(keypair_private_key_encrypted IS NULL, NULL, hex(keypair_private_key_encrypted)),
                   'keypair_source', keypair_source,
                   'connect_timeout_seconds', connect_timeout_seconds
               ) FROM machine_protocol_ssh WHERE machine_id = machines.id) AS protocol_ssh
        FROM machines
    "};

    pub fn new(
        pool: sqlx::SqlitePool,
        unit_of_work: Arc<dyn UnitOfWork<Tx = SqliteTx>>,
        cipher: Arc<dyn AeadCipher>,
    ) -> Self {
        Self {
            pool,
            unit_of_work,
            cipher,
        }
    }

    fn to_entity(&self, row: MachineRow) -> Result<Machine, RepositoryError> {
        use anyhow::Context as _;

        let id = row.id;

        row.try_into_entity(self.cipher.as_ref())
            .map_err(anyhow::Error::from_boxed)
            .with_context(|| format!("error reading machine {id}"))
            .map_err(RepositoryError::Unexpected)
    }

    async fn replace_protocols(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        row: &MachineRow,
    ) -> Result<(), RepositoryError> {
        match &row.protocol_ssh {
            Some(ssh) => sqlx::query(indoc! {"
                INSERT INTO machine_protocol_ssh (
                    machine_id,
                    user, port, host_key, authentication_method,
                    password_encrypted, keypair_private_key_encrypted, keypair_source,
                    connect_timeout_seconds
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT (machine_id) DO UPDATE SET
                    user = excluded.user,
                    port = excluded.port,
                    host_key = excluded.host_key,
                    authentication_method = excluded.authentication_method,
                    password_encrypted = excluded.password_encrypted,
                    keypair_private_key_encrypted = excluded.keypair_private_key_encrypted,
                    keypair_source = excluded.keypair_source,
                    connect_timeout_seconds = excluded.connect_timeout_seconds
            "})
            .bind(ssh.machine_id)
            .bind(&ssh.user)
            .bind(ssh.port)
            .bind(&ssh.host_key)
            .bind(ssh.authentication_method)
            .bind(&ssh.password_encrypted)
            .bind(&ssh.keypair_private_key_encrypted)
            .bind(ssh.keypair_source)
            .bind(ssh.connect_timeout_seconds),
            None => {
                sqlx::query("DELETE FROM machine_protocol_ssh WHERE machine_id = ?1").bind(row.id)
            }
        }
        .execute(self.executing(ctx))
        .await?;

        Ok(())
    }
}

#[async_trait]
impl Repository<SqliteTx> for EncryptedSqliteMachineRepository {
    async fn list(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        query: &ListQuery,
    ) -> Result<Paginated<Machine>, RepositoryError> {
        let search_filter = query
            .search
            .as_ref()
            .map_or("TRUE", |_| "instr(lower(name), lower(?1)) > 0");

        self.unit_of_work
            .transaction(ctx, ConsistentReads, async |ctx| {
                let total: u64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM machines WHERE ({search_filter})"
                )))
                .bind(query.search.as_deref()) // ?1
                .fetch_one(self.executing(ctx))
                .await?;

                let rows: Vec<MachineRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                    "{} WHERE ({search_filter}) ORDER BY id LIMIT ?2 OFFSET ?3",
                    Self::SELECT
                )))
                .bind(query.search.as_deref()) // ?1
                .bind(query.page.limit) // ?2
                .bind(query.page.offset) // ?3
                .fetch_all(self.executing(ctx))
                .await?;

                Ok(Paginated {
                    items: rows
                        .into_iter()
                        .map(|row| self.to_entity(row))
                        .collect::<Result<_, _>>()?,
                    total,
                })
            })
            .await
    }

    async fn get_by_id(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        id: uuid::Uuid,
    ) -> Result<Machine, RepositoryError> {
        let row: MachineRow = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{} WHERE id = ?",
            Self::SELECT
        )))
        .bind(id)
        .fetch_one(self.executing(ctx))
        .await?;

        self.to_entity(row)
    }

    async fn create(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        machine: &Machine,
    ) -> Result<Machine, RepositoryError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let row = MachineRow::from_entity(machine, self.cipher.as_ref());

                sqlx::query(indoc! {"
                    INSERT INTO machines (
                        id, name,
                        connectivity, direct_host,
                        tunnel_enrollment, tunnel_token_mac, tunnel_token_expires_at_unix_ms,
                        tunnel_client_public_key
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "})
                .bind(row.id)
                .bind(&row.name)
                .bind(row.connectivity)
                .bind(&row.direct_host)
                .bind(row.tunnel_enrollment)
                .bind(&row.tunnel_token_mac)
                .bind(row.tunnel_token_expires_at_unix_ms)
                .bind(&row.tunnel_client_public_key)
                .execute(self.executing(ctx))
                .await?;

                self.replace_protocols(ctx, &row).await?;

                self.get_by_id(ctx, machine.id).await
            })
            .await
    }

    async fn update(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        machine: &Machine,
    ) -> Result<Machine, RepositoryError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let row = MachineRow::from_entity(machine, self.cipher.as_ref());

                sqlx::query(indoc! {"
                    UPDATE machines SET
                        name = ?2,
                        connectivity = ?3, direct_host = ?4,
                        tunnel_enrollment = ?5, tunnel_token_mac = ?6,
                        tunnel_token_expires_at_unix_ms = ?7, tunnel_client_public_key = ?8
                    WHERE id = ?1
                    RETURNING id
                "})
                .bind(row.id)
                .bind(&row.name)
                .bind(row.connectivity)
                .bind(&row.direct_host)
                .bind(row.tunnel_enrollment)
                .bind(&row.tunnel_token_mac)
                .bind(row.tunnel_token_expires_at_unix_ms)
                .bind(&row.tunnel_client_public_key)
                .fetch_one(self.executing(ctx))
                .await?;

                self.replace_protocols(ctx, &row).await?;

                self.get_by_id(ctx, machine.id).await
            })
            .await
    }

    async fn delete(
        &self,
        ctx: &mut TxContext<SqliteTx>,
        id: uuid::Uuid,
    ) -> Result<Machine, RepositoryError> {
        self.unit_of_work
            .transaction(ctx, ReadModifyWrite, async |ctx| {
                let removed = self.get_by_id(ctx, id).await?;

                sqlx::query("DELETE FROM machines WHERE id = ? RETURNING id")
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
    use std::time::Duration;

    use secrecy::ExposeSecret;

    use super::*;
    use crate::application::machine::event::{
        ConnectivitySnapshot, EnrollmentSnapshot, MachineSnapshot,
    };
    use crate::application::repository::{Page, SearchText};
    use crate::domain::machine::{Connectivity, ssh, tunnel};
    use crate::infrastructure::crypto::cipher::XChaCha20Poly1305Cipher;
    use crate::testing::keys::{SSH_PRIVATE_KEY, TUNNEL_PUBLIC_KEY};
    use crate::testing::machine::ssh_access;
    use crate::testing::registry::Registry;
    use crate::testing::{enrollment, keys, machine, pagination};

    const OTHER_CIPHER_KEY: [u8; 32] = [8; 32];

    async fn create_machines_with_ascending_ids(registry: &Registry, names: &[&str]) {
        for (position, name) in (1..).zip(names) {
            registry
                .create_machine(&Machine {
                    id: uuid::Uuid::from_u128(position),
                    ..machine::machine_builder().name(name).build()
                })
                .await;
        }
    }

    async fn list_names(registry: &Registry, query: ListQuery) -> (Vec<String>, u64) {
        let machines = registry
            .machines
            .list(&mut TxContext::none(), &query)
            .await
            .unwrap();

        (
            machines
                .items
                .iter()
                .map(|machine| machine.name.to_string())
                .collect(),
            machines.total,
        )
    }

    async fn execute_for_machine(registry: &Registry, statement: &'static str, id: uuid::Uuid) {
        sqlx::query(statement)
            .bind(id)
            .execute(&registry.pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn round_trip_direct_machine_with_password_preserves_every_field() {
        let registry = Registry::in_memory().await;
        let machine = machine::direct_machine();
        registry.create_machine(&machine).await;

        let stored = registry.get_machine(machine.id).await;

        assert_eq!(
            MachineSnapshot::from(&stored),
            MachineSnapshot::from(&machine)
        );
        assert_eq!(stored.id, machine.id);
        assert_eq!(
            *ssh_access(&stored).connect_timeout,
            machine::CONNECT_TIMEOUT
        );
        let ssh::AuthMethod::Password(password) = ssh_access(&stored).authentication.method()
        else {
            panic!("expected a password");
        };
        assert_eq!(password.expose_secret(), machine::SSH_PASSWORD);
    }

    #[tokio::test]
    async fn round_trip_machine_without_protocols_preserves_no_protocols() {
        let registry = Registry::in_memory().await;
        let machine = machine::machine_builder().without_ssh().build();
        registry.create_machine(&machine).await;

        let stored = registry.get_machine(machine.id).await;

        assert_eq!(
            MachineSnapshot::from(&stored),
            MachineSnapshot::from(&machine)
        );
    }

    #[tokio::test]
    async fn round_trip_imported_keypair_preserves_the_private_key() {
        let registry = Registry::in_memory().await;
        let machine = machine::machine_builder()
            .authentication(machine::imported_keypair_method(keys::ssh_private_key()))
            .build();
        registry.create_machine(&machine).await;

        let stored = registry.get_machine(machine.id).await;

        assert_eq!(
            MachineSnapshot::from(&stored),
            MachineSnapshot::from(&machine)
        );
        let ssh::AuthMethod::Keypair { private_key, .. } =
            ssh_access(&stored).authentication.method()
        else {
            panic!("expected a keypair");
        };
        assert_eq!(private_key.expose_secret(), SSH_PRIVATE_KEY);
    }

    #[tokio::test]
    async fn round_trip_invited_tunnel_preserves_the_outstanding_token() {
        let registry = Registry::in_memory().await;
        let machine = machine::invited_machine();
        registry.create_machine(&machine).await;

        let stored = registry.get_machine(machine.id).await;

        assert_eq!(
            MachineSnapshot::from(&stored).connectivity,
            ConnectivitySnapshot::Tunneled {
                enrollment: EnrollmentSnapshot::Invited {
                    expires_at: enrollment::expires_at()
                }
            }
        );
        let Connectivity::Tunneled(tunnel::Enrollment::Invited { mac, .. }) = &stored.connectivity
        else {
            panic!("expected an invited tunnel");
        };
        assert_eq!(**mac, enrollment::TOKEN_MAC);
    }

    #[tokio::test]
    async fn round_trip_unenrolled_tunnel_preserves_the_enrollment_state() {
        let registry = Registry::in_memory().await;
        let machine = machine::unenrolled_machine();
        registry.create_machine(&machine).await;

        let stored = registry.get_machine(machine.id).await;

        assert_eq!(
            MachineSnapshot::from(&stored).connectivity,
            ConnectivitySnapshot::Tunneled {
                enrollment: EnrollmentSnapshot::NotEnrolled
            }
        );
    }

    #[tokio::test]
    async fn round_trip_enrolled_tunnel_preserves_the_client_public_key() {
        let registry = Registry::in_memory().await;
        let machine = machine::enrolled_machine();
        registry.create_machine(&machine).await;

        let stored = registry.get_machine(machine.id).await;

        let Connectivity::Tunneled(tunnel::Enrollment::Enrolled { client_public_key }) =
            &stored.connectivity
        else {
            panic!("expected an enrolled tunnel");
        };
        assert_eq!(&**client_public_key, TUNNEL_PUBLIC_KEY);
    }

    #[tokio::test]
    async fn create_with_a_taken_id_returns_already_exists() {
        let registry = Registry::in_memory().await;
        let machine = machine::direct_machine();
        registry.create_machine(&machine).await;

        let created = registry
            .machines
            .create(&mut TxContext::none(), &machine)
            .await;

        assert_matches!(created.err(), Some(RepositoryError::AlreadyExists));
    }

    #[tokio::test]
    async fn create_stores_the_id_as_a_sixteen_byte_blob() {
        let registry = Registry::in_memory().await;
        registry.create_machine(&machine::direct_machine()).await;

        let (column_type, length): (String, i64) =
            sqlx::query_as("SELECT typeof(id), length(id) FROM machines")
                .fetch_one(&registry.pool)
                .await
                .unwrap();

        assert_eq!((column_type.as_str(), length), ("blob", 16));
    }

    #[tokio::test]
    async fn create_stores_password_as_ciphertext() {
        let registry = Registry::in_memory().await;
        let machine = registry.create_machine(&machine::direct_machine()).await;

        let stored_password_bytes: Vec<u8> = sqlx::query_scalar(
            "SELECT password_encrypted FROM machine_protocol_ssh WHERE machine_id = ?",
        )
        .bind(machine.id)
        .fetch_one(&registry.pool)
        .await
        .unwrap();

        let password = machine::SSH_PASSWORD.as_bytes();
        assert!(
            !stored_password_bytes
                .windows(password.len())
                .any(|window| window == password)
        );
    }

    #[tokio::test]
    async fn get_by_id_unknown_returns_not_found() {
        let registry = Registry::in_memory().await;

        let stored = registry
            .machines
            .get_by_id(&mut TxContext::none(), uuid::Uuid::now_v7())
            .await;

        assert_matches!(stored.err(), Some(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn get_by_id_with_a_column_it_cannot_decode_returns_error() {
        let registry = Registry::in_memory().await;
        let corruptions = [
            (
                machine::direct_machine as fn() -> Machine,
                "UPDATE machine_protocol_ssh SET host_key = X'00' WHERE machine_id = ?",
            ),
            (
                machine::direct_machine,
                "UPDATE machine_protocol_ssh SET password_encrypted = X'00' WHERE machine_id = ?",
            ),
            (
                machine::direct_machine,
                "UPDATE machines SET direct_host = '[' WHERE id = ?",
            ),
            // The column is `STRICT INTEGER`, so only a number out of range can be wrong.
            (
                machine::invited_machine,
                "UPDATE machines SET tunnel_token_expires_at_unix_ms = 9223372036854775807 WHERE id = ?",
            ),
            (
                machine::invited_machine,
                "UPDATE machines SET tunnel_token_mac = NULL WHERE id = ?",
            ),
            (
                machine::enrolled_machine,
                "UPDATE machines SET tunnel_client_public_key = X'00' WHERE id = ?",
            ),
        ];

        for (build_machine, corruption) in corruptions {
            let machine = registry.create_machine(&build_machine()).await;
            execute_for_machine(&registry, corruption, machine.id).await;

            let stored = registry
                .machines
                .get_by_id(&mut TxContext::none(), machine.id)
                .await;

            assert_matches!(
                stored.err(),
                Some(RepositoryError::Unexpected(_)),
                "{corruption}"
            );
        }
    }

    #[tokio::test]
    async fn get_by_id_under_another_cipher_key_returns_error() {
        let registry = Registry::in_memory().await;
        let machine = registry.create_machine(&machine::direct_machine()).await;
        let repository = EncryptedSqliteMachineRepository::new(
            registry.pool.clone(),
            Arc::clone(&registry.transactions) as Arc<_>,
            Arc::new(XChaCha20Poly1305Cipher::new(&keys::symmetric_key(
                OTHER_CIPHER_KEY,
            ))),
        );

        let stored = repository
            .get_by_id(&mut TxContext::none(), machine.id)
            .await;

        assert_matches!(stored.err(), Some(RepositoryError::Unexpected(_)));
    }

    #[tokio::test]
    async fn get_by_id_with_password_moved_from_another_machine_returns_error() {
        let registry = Registry::in_memory().await;
        let source_machine = registry
            .create_machine(&machine::machine_builder().name("web-1").build())
            .await;
        let target_machine = registry
            .create_machine(&machine::machine_builder().name("web-2").build())
            .await;
        sqlx::query(indoc! {"
            UPDATE machine_protocol_ssh
            SET password_encrypted =
                (SELECT password_encrypted FROM machine_protocol_ssh WHERE machine_id = ?1)
            WHERE machine_id = ?2
        "})
        .bind(source_machine.id)
        .bind(target_machine.id)
        .execute(&registry.pool)
        .await
        .unwrap();

        let stored = registry
            .machines
            .get_by_id(&mut TxContext::none(), target_machine.id)
            .await;

        assert_matches!(stored.err(), Some(RepositoryError::Unexpected(_)));
    }

    #[tokio::test]
    async fn get_by_id_with_private_key_moved_to_the_password_returns_error() {
        let registry = Registry::in_memory().await;
        let machine = registry
            .create_machine(
                &machine::machine_builder()
                    .authentication(machine::imported_keypair_method(keys::ssh_private_key()))
                    .build(),
            )
            .await;
        execute_for_machine(
            &registry,
            indoc! {"
                UPDATE machine_protocol_ssh
                SET authentication_method = 'password',
                    password_encrypted = keypair_private_key_encrypted,
                    keypair_private_key_encrypted = NULL,
                    keypair_source = NULL
                WHERE machine_id = ?
            "},
            machine.id,
        )
        .await;

        let stored = registry
            .machines
            .get_by_id(&mut TxContext::none(), machine.id)
            .await;

        assert_matches!(stored.err(), Some(RepositoryError::Unexpected(_)));
    }

    #[tokio::test]
    async fn list_returns_the_machines_ordered_by_id() {
        let registry = Registry::in_memory().await;
        registry
            .create_machine(&Machine {
                id: uuid::Uuid::from_u128(2),
                ..machine::machine_builder().name("b").build()
            })
            .await;
        registry
            .create_machine(&Machine {
                id: uuid::Uuid::from_u128(1),
                ..machine::machine_builder().name("a").build()
            })
            .await;

        let (names, _) = list_names(&registry, pagination::everything()).await;

        assert_eq!(names, ["a", "b"]);
    }

    #[tokio::test]
    async fn list_with_and_without_ssh_returns_each_machine_with_its_own_protocols() {
        let registry = Registry::in_memory().await;
        let with_ssh = Machine {
            id: uuid::Uuid::from_u128(1),
            ..machine::machine_builder().build()
        };
        let without_ssh = Machine {
            id: uuid::Uuid::from_u128(2),
            ..machine::machine_builder().without_ssh().build()
        };
        registry.create_machine(&with_ssh).await;
        registry.create_machine(&without_ssh).await;

        let machines = registry
            .machines
            .list(&mut TxContext::none(), &pagination::everything())
            .await
            .unwrap();

        let snapshots: Vec<_> = machines.items.iter().map(MachineSnapshot::from).collect();
        assert_eq!(
            snapshots,
            [
                MachineSnapshot::from(&with_ssh),
                MachineSnapshot::from(&without_ssh)
            ]
        );
    }

    #[tokio::test]
    async fn list_with_a_search_returns_the_machines_whose_name_contains_it() {
        let registry = Registry::in_memory().await;
        create_machines_with_ascending_ids(&registry, &["web-01", "WEB-02", "db_1", "dbx1"]).await;

        for (search, expected_names) in [("web", &["web-01", "WEB-02"][..]), ("b_1", &["db_1"])] {
            let (names, total) = list_names(
                &registry,
                ListQuery {
                    search: Some(SearchText::parse(search).unwrap()),
                    ..pagination::everything()
                },
            )
            .await;

            assert_eq!(names, expected_names, "{search}");
            assert_eq!(total, expected_names.len() as u64, "{search}");
        }
    }

    #[tokio::test]
    async fn list_returns_the_page_and_the_full_total() {
        let registry = Registry::in_memory().await;
        create_machines_with_ascending_ids(&registry, &["a", "b", "c"]).await;

        for (limit, offset, expected_names) in
            [(1, 1, &["b"][..]), (2, 0, &["a", "b"]), (10, 10, &[])]
        {
            let (names, total) = list_names(
                &registry,
                ListQuery {
                    search: None,
                    page: Page { limit, offset },
                },
            )
            .await;

            assert_eq!(names, expected_names, "limit {limit}, offset {offset}");
            assert_eq!(total, 3, "limit {limit}, offset {offset}");
        }
    }

    #[tokio::test]
    async fn list_with_a_row_that_lacks_a_required_column_returns_error() {
        let registry = Registry::in_memory().await;
        let machine = registry.create_machine(&machine::direct_machine()).await;
        execute_for_machine(
            &registry,
            "UPDATE machines SET direct_host = NULL WHERE id = ?",
            machine.id,
        )
        .await;

        let listed = registry
            .machines
            .list(&mut TxContext::none(), &pagination::everything())
            .await;

        assert_matches!(listed.err(), Some(RepositoryError::Unexpected(_)));
    }

    #[tokio::test]
    async fn update_replaces_every_field() {
        let registry = Registry::in_memory().await;
        let machine = registry.create_machine(&machine::direct_machine()).await;
        let replacement = Machine {
            id: machine.id,
            ..machine::machine_builder()
                .name("renamed")
                .tunneled(tunnel::Enrollment::NotEnrolled)
                .authentication(machine::imported_keypair_method(keys::ssh_private_key()))
                .connect_timeout(Duration::from_secs(45))
                .build()
        };

        registry
            .machines
            .update(&mut TxContext::none(), &replacement)
            .await
            .unwrap();

        let stored = registry.get_machine(machine.id).await;
        assert_eq!(
            MachineSnapshot::from(&stored),
            MachineSnapshot::from(&replacement)
        );
        assert_eq!(
            *ssh_access(&stored).connect_timeout,
            Duration::from_secs(45)
        );
    }

    #[tokio::test]
    async fn update_clears_the_columns_that_its_connectivity_no_longer_uses() {
        let registry = Registry::in_memory().await;
        let machine = registry.create_machine(&machine::direct_machine()).await;

        registry
            .machines
            .update(
                &mut TxContext::none(),
                &Machine {
                    id: machine.id,
                    ..machine::enrolled_machine()
                },
            )
            .await
            .unwrap();

        let direct_host: Option<String> =
            sqlx::query_scalar("SELECT direct_host FROM machines WHERE id = ?")
                .bind(machine.id)
                .fetch_one(&registry.pool)
                .await
                .unwrap();
        assert_eq!(direct_host, None);
    }

    #[tokio::test]
    async fn update_without_ssh_removes_the_ssh_row() {
        let registry = Registry::in_memory().await;
        let machine = registry.create_machine(&machine::direct_machine()).await;

        registry
            .machines
            .update(
                &mut TxContext::none(),
                &Machine {
                    id: machine.id,
                    ..machine::machine_builder().without_ssh().build()
                },
            )
            .await
            .unwrap();

        let ssh_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM machine_protocol_ssh WHERE machine_id = ?")
                .bind(machine.id)
                .fetch_one(&registry.pool)
                .await
                .unwrap();
        assert_eq!(ssh_rows, 0);
    }

    #[tokio::test]
    async fn update_unknown_machine_returns_not_found() {
        let registry = Registry::in_memory().await;

        let updated = registry
            .machines
            .update(&mut TxContext::none(), &machine::direct_machine())
            .await;

        assert_matches!(updated.err(), Some(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_removes_the_machine() {
        let registry = Registry::in_memory().await;
        let machine = registry.create_machine(&machine::direct_machine()).await;

        registry
            .machines
            .delete(&mut TxContext::none(), machine.id)
            .await
            .unwrap();

        let stored = registry
            .machines
            .get_by_id(&mut TxContext::none(), machine.id)
            .await;
        assert_matches!(stored.err(), Some(RepositoryError::NotFound));
    }

    #[tokio::test]
    async fn delete_removes_the_protocols_of_the_machine() {
        let registry = Registry::in_memory().await;
        let machine = registry.create_machine(&machine::direct_machine()).await;

        registry
            .machines
            .delete(&mut TxContext::none(), machine.id)
            .await
            .unwrap();

        let ssh_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM machine_protocol_ssh")
            .fetch_one(&registry.pool)
            .await
            .unwrap();
        assert_eq!(ssh_rows, 0);
    }

    #[tokio::test]
    async fn delete_unknown_machine_returns_not_found() {
        let registry = Registry::in_memory().await;

        let deleted = registry
            .machines
            .delete(&mut TxContext::none(), uuid::Uuid::now_v7())
            .await;

        assert_matches!(deleted.err(), Some(RepositoryError::NotFound));
    }
}

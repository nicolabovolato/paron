CREATE TABLE users (
    id                 BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    username           TEXT    NOT NULL UNIQUE,
    display_name       TEXT    NOT NULL,
    status             TEXT    NOT NULL CHECK (status IN ('active', 'suspended')),
    created_at_unix_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE user_credential_password (
    user_id            BLOB    NOT NULL PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    phc_encrypted      BLOB    NOT NULL,
    updated_at_unix_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE user_credential_totp (
    user_id            BLOB    NOT NULL PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    secret_encrypted   BLOB    NOT NULL,
    last_used_step     INTEGER NOT NULL CHECK (last_used_step >= 0),
    created_at_unix_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE user_credential_passkey (
    id                   BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    user_id              BLOB    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    credential_id        BLOB    NOT NULL UNIQUE,
    public_key           BLOB    NOT NULL,
    name                 TEXT    NOT NULL,
    sign_count           INTEGER NOT NULL CHECK (sign_count BETWEEN 0 AND 4294967295),
    backup_eligible      INTEGER NOT NULL CHECK (backup_eligible IN (0, 1)),
    backup_state         INTEGER NOT NULL CHECK (backup_state IN (0, 1)),
    created_at_unix_ms   INTEGER NOT NULL,
    last_used_at_unix_ms INTEGER
) STRICT, WITHOUT ROWID;

CREATE INDEX user_credential_passkey_user_id_idx ON user_credential_passkey (user_id);

CREATE TABLE user_credential_recovery_codes (
    user_id              BLOB    NOT NULL PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    codes                TEXT    NOT NULL CHECK (json_valid(codes)),
    generated_at_unix_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE user_pending_account_recovery (
    user_id            BLOB    NOT NULL PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    token_mac          BLOB    NOT NULL UNIQUE,
    expires_at_unix_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE TABLE user_roles (
    user_id BLOB NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role    TEXT NOT NULL CHECK (role IN ('administrator', 'operator')),

    PRIMARY KEY (user_id, role)
) STRICT, WITHOUT ROWID;

CREATE TABLE user_sessions (
    id                                   BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    user_id                              BLOB    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_mac                            BLOB    NOT NULL UNIQUE,
    issued_at_unix_ms                    INTEGER NOT NULL,
    recently_authenticated_until_unix_ms INTEGER NOT NULL,
    last_used_at_unix_ms                 INTEGER NOT NULL,
    expires_at_unix_ms                   INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX user_sessions_user_id_issued_at_unix_ms_idx ON user_sessions (user_id, issued_at_unix_ms);
CREATE INDEX user_sessions_expires_at_unix_ms_idx ON user_sessions (expires_at_unix_ms);
CREATE INDEX user_sessions_last_used_at_unix_ms_idx ON user_sessions (last_used_at_unix_ms);

CREATE TABLE user_pending_sign_ins (
    token_mac          BLOB    NOT NULL PRIMARY KEY,
    origin             TEXT    NOT NULL,
    renewed_session_id BLOB    REFERENCES user_sessions (id) ON DELETE CASCADE,
    user_id            BLOB    REFERENCES users (id) ON DELETE CASCADE,
    verified_methods   TEXT    NOT NULL CHECK (json_valid(verified_methods)),
    accepted_methods   TEXT    NOT NULL CHECK (json_valid(accepted_methods)),
    challenge          BLOB,
    expires_at_unix_ms INTEGER NOT NULL
) STRICT, WITHOUT ROWID;

CREATE INDEX user_pending_sign_ins_expires_at_unix_ms_idx ON user_pending_sign_ins (expires_at_unix_ms);

CREATE TABLE user_failed_sign_in_attempts (
    method                  TEXT    NOT NULL CHECK (method IN ('password', 'totp', 'recovery_code')),
    user_id                 BLOB    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    device_id               BLOB    CHECK (device_id IS NULL OR length(device_id) = 16),
    attempts                INTEGER NOT NULL CHECK (attempts >= 1),
    next_attempt_at_unix_ms INTEGER NOT NULL,
    expires_at_unix_ms      INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX user_failed_sign_in_attempts_method_user_id_idx ON user_failed_sign_in_attempts (method, user_id) WHERE device_id IS NULL;
CREATE UNIQUE INDEX user_failed_sign_in_attempts_method_device_id_idx ON user_failed_sign_in_attempts (method, device_id) WHERE device_id IS NOT NULL;
CREATE INDEX user_failed_sign_in_attempts_expires_at_unix_ms_idx ON user_failed_sign_in_attempts (expires_at_unix_ms);

CREATE TABLE machines (
    id   BLOB NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    name TEXT NOT NULL,

    connectivity TEXT NOT NULL CHECK (connectivity IN ('direct', 'tunneled')),

    -- Connectivity::Direct
    direct_host TEXT,

    -- Connectivity::Tunneled
    tunnel_enrollment TEXT CHECK (tunnel_enrollment IS NULL OR tunnel_enrollment IN ('not_enrolled', 'invited', 'enrolled')),

    -- tunnel::Enrollment::Invited
    tunnel_token_mac                BLOB,
    tunnel_token_expires_at_unix_ms INTEGER,

    -- tunnel::Enrollment::Enrolled
    tunnel_client_public_key BLOB
) STRICT, WITHOUT ROWID;

CREATE TABLE machine_protocol_ssh (
    machine_id BLOB NOT NULL PRIMARY KEY REFERENCES machines (id) ON DELETE CASCADE,

    user                    TEXT    NOT NULL,
    port                    INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    connect_timeout_seconds INTEGER NOT NULL CHECK (connect_timeout_seconds BETWEEN 1 AND 60),

    -- ssh::Authentication
    host_key              BLOB NOT NULL,
    authentication_method TEXT NOT NULL CHECK (authentication_method IN ('password', 'keypair')),

    -- ssh::AuthMethod::Password
    password_encrypted BLOB,

    -- ssh::AuthMethod::Keypair
    keypair_private_key_encrypted BLOB,
    keypair_source                TEXT CHECK (keypair_source IS NULL OR keypair_source IN ('managed', 'imported'))
) STRICT, WITHOUT ROWID;

CREATE TABLE audit_log (
    id                  BLOB    NOT NULL PRIMARY KEY CHECK (length(id) = 16),
    created_at_unix_ms  INTEGER NOT NULL,
    actor_type          TEXT    NOT NULL,
    actor_id            BLOB    CHECK (actor_id IS NULL OR length(actor_id) = 16),
    actor_name          TEXT,
    target_type         TEXT    NOT NULL,
    target_id           BLOB    NOT NULL CHECK (length(target_id) = 16),
    target_name         TEXT    NOT NULL,
    type                TEXT    NOT NULL,
    payload             TEXT    NOT NULL CHECK (json_valid(payload))
) STRICT, WITHOUT ROWID;

CREATE INDEX audit_log_actor_type_actor_id_idx ON audit_log (actor_type, actor_id);
CREATE INDEX audit_log_target_id_idx ON audit_log (target_id);

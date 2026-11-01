use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ciborium::Value;
use ring::digest;
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use webauthn_rs_proto::{PublicKeyCredential, RegisterPublicKeyCredential};

use crate::application::audit::AuditService;
use crate::application::clock::Clock;
use crate::application::identity::authentication::credentials::CredentialService;
use crate::application::identity::authentication::methods::passkey::{
    Challenge, CredentialId, PasskeyBackup, PasskeyCredential, PasskeyName, PasskeyRegistration,
    PasskeyService,
};
use crate::application::identity::authentication::methods::password::PasswordService;
use crate::application::identity::authentication::methods::password::{
    Password, PasswordCredential,
};
use crate::application::identity::authentication::methods::ports::{
    MockPasswordHasher, PasskeyCredentialRepository as _, PasswordCredentialRepository as _,
    RecoveryCodeRepository as _, TotpCredentialRepository as _,
};
use crate::application::identity::authentication::methods::recovery_code::RecoveryCodeService;
use crate::application::identity::authentication::methods::recovery_code::{
    RecoveryCode, RecoveryCodeCredential,
};
use crate::application::identity::authentication::methods::totp::{
    TotpCode, TotpCredential, TotpSecret, TotpStep,
};
use crate::application::identity::authentication::methods::totp::{TotpRegistration, TotpService};
use crate::application::identity::authentication::sessions::SessionService;
use crate::application::identity::users::UserService;
use crate::application::repository::TxContext;
use crate::application::tokens::Tokens as _;
use crate::domain::user::{Role, User};
use crate::infrastructure::crypto::tokens::Hmac;
use crate::infrastructure::in_memory::credential_registrations::InMemoryRegistrationRepository;
use crate::infrastructure::sqlite::SqliteTx;
use crate::infrastructure::sqlite::transaction::SqliteUnitOfWork;
use crate::testing::clock::{clock_at, timestamp};
use crate::testing::events::{self, RecordedAudit};
use crate::testing::identity;
use crate::testing::keys::symmetric_key;
use crate::testing::registry::Registry;

pub const NOW: i64 = 1_800_000_000;
pub const TOTP_SECRET: [u8; 20] = [7; 20];
pub const CREDENTIAL_ID: [u8; 16] = [9; 16];
pub const PASSKEY_CEREMONY_LIFETIME: Duration = Duration::from_mins(5);
pub const TOTP_REGISTRATION_LIFETIME: Duration = Duration::from_mins(5);
pub const RECOVERY_CODE: &str = "abcdefghjk";

/// A database and the ceremonies in memory. Services made at different times share them, so a token
/// from one works with the next.
pub struct Installation {
    pub registry: Registry,
    pub audit: Arc<RecordedAudit<SqliteTx>>,
    pub passkey_registrations: Arc<InMemoryRegistrationRepository<PasskeyRegistration>>,
    pub totp_registrations: Arc<InMemoryRegistrationRepository<TotpRegistration>>,
}

/// The services that the identity use cases are made of, at one time.
pub struct Services {
    pub users: Arc<UserService<SqliteTx>>,
    pub sessions: Arc<SessionService<SqliteTx>>,
    pub credentials: Arc<CredentialService<SqliteTx>>,
    pub unit_of_work: Arc<SqliteUnitOfWork>,
    pub audit: Arc<AuditService<SqliteTx>>,
    pub clock: Arc<dyn Clock>,
}

impl Installation {
    pub async fn new() -> Self {
        let registry = Registry::in_memory().await;
        let audit = Arc::new(RecordedAudit::new(Arc::clone(&registry.audit) as Arc<_>));

        Self {
            registry,
            audit,
            passkey_registrations: Arc::new(InMemoryRegistrationRepository::new(clock_at(
                timestamp(NOW),
            ))),
            totp_registrations: Arc::new(InMemoryRegistrationRepository::new(clock_at(timestamp(
                NOW,
            )))),
        }
    }

    pub fn services(&self, seconds: i64) -> Services {
        let clock = clock_at(timestamp(seconds));
        let unit_of_work = Arc::clone(&self.registry.transactions);
        let audit = Arc::new(AuditService::new(
            Arc::clone(&self.audit) as Arc<_>,
            Arc::clone(&clock),
        ));
        let events = events::event_service();
        let sessions = Arc::new(SessionService::new(
            Arc::clone(&self.registry.sessions) as Arc<_>,
            Arc::new(Hmac::new(symmetric_key([1; 32]))),
            Arc::clone(&clock),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&events),
            identity::session_policy(),
        ));
        let users = Arc::new(UserService::new(
            Arc::clone(&self.registry.users) as Arc<_>,
            Arc::clone(&sessions),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&audit),
            Arc::clone(&events),
            Arc::clone(&clock),
        ));
        let mut hasher = MockPasswordHasher::new();
        hasher
            .expect_verify()
            .returning(|password, _| password.expose_secret() == identity::PASSWORD);
        hasher.expect_hash().returning(|_| identity::password_phc());
        let password = Arc::new(PasswordService::new(
            Arc::clone(&self.registry.passwords) as Arc<_>,
            Arc::new(hasher),
            Arc::clone(&audit),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&clock),
        ));
        let totp = Arc::new(TotpService::new(
            Arc::clone(&self.registry.totps) as Arc<_>,
            Arc::clone(&self.totp_registrations) as Arc<_>,
            Arc::clone(&audit),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&clock),
            TOTP_REGISTRATION_LIFETIME,
            url::Host::Domain("paron.example".to_owned()),
        ));
        let passkeys = Arc::new(PasskeyService::new(
            Arc::clone(&self.registry.passkeys) as Arc<_>,
            Arc::clone(&self.passkey_registrations) as Arc<_>,
            Arc::clone(&audit),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&clock),
            PASSKEY_CEREMONY_LIFETIME,
        ));
        let recovery_codes = Arc::new(RecoveryCodeService::new(
            Arc::clone(&self.registry.recovery_codes) as Arc<_>,
            Arc::new(recovery_code_tokens()),
            Arc::clone(&audit),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&clock),
            4,
        ));

        let credentials = Arc::new(CredentialService::new(
            password,
            totp,
            passkeys,
            recovery_codes,
            Arc::clone(&users),
            Arc::clone(&sessions),
            Arc::clone(&unit_of_work) as Arc<_>,
            Arc::clone(&clock),
        ));

        Services {
            users,
            sessions,
            credentials,
            unit_of_work,
            audit,
            clock,
        }
    }

    /// An administrator with the password [`identity::PASSWORD`].
    pub async fn user_with_password(&self, username: &str) -> User {
        let user = self
            .registry
            .create_user(
                &identity::user_builder()
                    .username(username)
                    .roles(&[Role::Administrator])
                    .build(),
            )
            .await;
        self.registry
            .passwords
            .upsert(
                &mut TxContext::none(),
                &PasswordCredential {
                    user_id: user.id,
                    phc: identity::password_phc(),
                    updated_at: timestamp(0),
                },
            )
            .await
            .unwrap();

        user
    }

    /// With [`TOTP_SECRET`]. No code is used yet.
    pub async fn add_totp(&self, user: &User) {
        self.registry
            .totps
            .create(
                &mut TxContext::none(),
                &TotpCredential {
                    user_id: user.id,
                    secret: TotpSecret::from(SecretSlice::from(TOTP_SECRET.to_vec())),
                    last_used_step: TotpStep(0),
                    created_at: timestamp(0),
                },
            )
            .await
            .unwrap();
    }

    /// "Laptop", with [`CREDENTIAL_ID`] and the key of [`passkey_assertion`].
    pub async fn add_passkey(&self, user: &User) -> PasskeyCredential {
        self.registry
            .passkeys
            .create(
                &mut TxContext::none(),
                &PasskeyCredential {
                    id: uuid::Uuid::now_v7(),
                    user_id: user.id,
                    credential_id: CredentialId(CREDENTIAL_ID.to_vec()),
                    public_key: passkey_public_key(),
                    name: PasskeyName::parse("Laptop").unwrap(),
                    sign_count: 0,
                    backup: PasskeyBackup::default(),
                    created_at: timestamp(0),
                    last_used_at: None,
                },
            )
            .await
            .unwrap()
    }

    /// The one code is [`RECOVERY_CODE`].
    pub async fn add_recovery_code(&self, user: &User) {
        self.registry
            .recovery_codes
            .upsert(
                &mut TxContext::none(),
                &RecoveryCodeCredential {
                    user_id: user.id,
                    code_macs: vec![
                        recovery_code_tokens().mac(&RecoveryCode::parse(RECOVERY_CODE).unwrap()),
                    ],
                    generated_at: timestamp(0),
                },
            )
            .await
            .unwrap();
    }
}

pub fn recovery_code_tokens() -> Hmac {
    Hmac::new(symmetric_key([2; 32]))
}

pub fn code_at(seconds: i64) -> TotpCode {
    let code = totp_rs::Builder::new()
        .with_secret(TOTP_SECRET.to_vec())
        .build()
        .unwrap()
        .generate(u64::try_from(seconds).unwrap());

    TotpCode::parse(&code.to_string()).unwrap()
}

pub fn wrong_password() -> Password {
    Password::parse(SecretString::from("not the right password")).unwrap()
}

pub fn origin() -> url::Origin {
    url::Url::parse("https://paron.example").unwrap().origin()
}

const USER_PRESENT: u8 = 0x01;
const USER_VERIFIED: u8 = 0x04;
const ATTESTED_CREDENTIAL_DATA: u8 = 0x40;

fn passkey_key() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[3; 32]).unwrap()
}

/// The public key of the test passkey, as a COSE key (RFC 9053): OKP, EdDSA, Ed25519.
pub fn passkey_public_key() -> Vec<u8> {
    let mut encoded = Vec::new();
    ciborium::ser::into_writer(
        &Value::Map(vec![
            (Value::from(1), Value::from(1)),
            (Value::from(3), Value::from(-8)),
            (Value::from(-1), Value::from(6)),
            (
                Value::from(-2),
                Value::Bytes(passkey_key().public_key().as_ref().to_vec()),
            ),
        ]),
        &mut encoded,
    )
    .unwrap();

    encoded
}

/// The authenticator data and the client data of a ceremony at `origin` (WebAuthn §6.1, §5.8.1).
fn ceremony_data(
    kind: &str,
    origin: &url::Origin,
    challenge: &Challenge,
    flags: u8,
    attested_credential: &[u8],
) -> (Vec<u8>, Vec<u8>) {
    let url::Origin::Tuple(_, host, _) = origin else {
        panic!("the origin has no host");
    };
    let rp_id_hash = digest::digest(&digest::SHA256, host.to_string().as_bytes());
    let authenticator_data = [rp_id_hash.as_ref(), &[flags], &[0; 4], attested_credential].concat();
    let client_data = serde_json::json!({
        "type": kind,
        "challenge": URL_SAFE_NO_PAD.encode(challenge.0),
        "origin": origin.ascii_serialization(),
    })
    .to_string()
    .into_bytes();

    (authenticator_data, client_data)
}

/// What a browser sends to register the test passkey with [`CREDENTIAL_ID`], with a `none`
/// statement.
pub fn passkey_registration(
    origin: &url::Origin,
    challenge: &Challenge,
) -> RegisterPublicKeyCredential {
    let attested_credential = [
        &[0; 16][..],
        &u16::try_from(CREDENTIAL_ID.len()).unwrap().to_be_bytes(),
        &CREDENTIAL_ID,
        &passkey_public_key(),
    ]
    .concat();
    let (authenticator_data, client_data) = ceremony_data(
        "webauthn.create",
        origin,
        challenge,
        USER_PRESENT | USER_VERIFIED | ATTESTED_CREDENTIAL_DATA,
        &attested_credential,
    );
    let mut attestation_object = Vec::new();
    ciborium::ser::into_writer(
        &Value::Map(vec![
            (Value::from("fmt"), Value::from("none")),
            (Value::from("attStmt"), Value::Map(Vec::new())),
            (Value::from("authData"), Value::Bytes(authenticator_data)),
        ]),
        &mut attestation_object,
    )
    .unwrap();

    serde_json::from_value(serde_json::json!({
        "id": URL_SAFE_NO_PAD.encode(CREDENTIAL_ID),
        "rawId": URL_SAFE_NO_PAD.encode(CREDENTIAL_ID),
        "response": {
            "attestationObject": URL_SAFE_NO_PAD.encode(attestation_object),
            "clientDataJSON": URL_SAFE_NO_PAD.encode(client_data),
        },
        "type": "public-key",
    }))
    .unwrap()
}

/// What a browser sends for the test passkey of `user`, signed over `challenge`.
pub fn passkey_assertion(
    user: &User,
    origin: &url::Origin,
    challenge: &Challenge,
    user_verified: bool,
) -> PublicKeyCredential {
    let flags = if user_verified {
        USER_PRESENT | USER_VERIFIED
    } else {
        USER_PRESENT
    };
    let (authenticator_data, client_data) =
        ceremony_data("webauthn.get", origin, challenge, flags, &[]);
    let client_data_hash = digest::digest(&digest::SHA256, &client_data);
    let signature =
        passkey_key().sign(&[&authenticator_data[..], client_data_hash.as_ref()].concat());

    serde_json::from_value(serde_json::json!({
        "id": URL_SAFE_NO_PAD.encode(CREDENTIAL_ID),
        "rawId": URL_SAFE_NO_PAD.encode(CREDENTIAL_ID),
        "response": {
            "authenticatorData": URL_SAFE_NO_PAD.encode(authenticator_data),
            "clientDataJSON": URL_SAFE_NO_PAD.encode(client_data),
            "signature": URL_SAFE_NO_PAD.encode(signature.as_ref()),
            "userHandle": URL_SAFE_NO_PAD.encode(user.id),
        },
        "type": "public-key",
    }))
    .unwrap()
}

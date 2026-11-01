//! The relying party of WebAuthn Level 3: §7.1 for a registration and §7.2 for an authentication.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD_INDIFFERENT;
use ciborium::Value;
use serde::Deserialize;
use subtle::ConstantTimeEq as _;
use webauthn_rs_proto::{
    AllowCredentials, AttestationConveyancePreference, AuthenticatorSelectionCriteria, CredProtect,
    CredentialProtectionPolicy, PubKeyCredParams, PublicKeyCredential,
    PublicKeyCredentialCreationOptions, PublicKeyCredentialDescriptor,
    PublicKeyCredentialRequestOptions, RegisterPublicKeyCredential, RequestRegistrationExtensions,
    ResidentKeyRequirement, UserVerificationPolicy,
};

use super::authenticator_data::AuthenticatorData;
use super::cbor;
use super::cose::{self, CoseKey};
use super::{Challenge, CredentialId, PasskeyBackup, PasskeyCredential};
use crate::domain::user::User;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyUserVerification {
    Verified,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPasskey {
    pub credential_id: CredentialId,
    pub public_key: Vec<u8>,
    pub user_verification: PasskeyUserVerification,
    pub sign_count: u32,
    pub backup: PasskeyBackup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasskeyAssertion {
    pub user_verification: PasskeyUserVerification,
    pub sign_count: u32,
    pub backup: PasskeyBackup,
}

pub struct StartedPasskeyRegistration {
    pub options: PublicKeyCredentialCreationOptions,
    pub challenge: Challenge,
}

pub struct StartedPasskeyAuthentication {
    pub options: PublicKeyCredentialRequestOptions,
    pub challenge: Challenge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InvalidPasskey {
    #[error("the answer of the authenticator is not well formed")]
    Malformed,

    #[error("the answer is for another credential")]
    OtherCredential,

    #[error("the answer is for another ceremony")]
    OtherCeremony,

    #[error("the challenge is not the challenge of this ceremony")]
    WrongChallenge,

    #[error("the origin of the page is not the origin of the ceremony")]
    WrongOrigin,

    #[error("the authenticator signed for another relying party")]
    WrongRelyingParty,

    #[error("the authenticator did not check that the user is present")]
    UserNotPresent,

    #[error("the authenticator says that the passkey is backed up, but cannot be")]
    InconsistentBackupFlags,

    #[error("the public key of the passkey is not supported")]
    UnsupportedKey,

    #[error("the attestation statement of the passkey is not supported")]
    UnsupportedAttestation,

    #[error("the signature is not correct")]
    WrongSignature,

    #[error("the signature counter did not increase, so the authenticator can be a copy")]
    CounterNotIncreased,

    #[error("the backup eligibility of the passkey changed")]
    BackupEligibilityChanged,
}

const RP_NAME: &str = "Paron";
const PUBLIC_KEY: &str = "public-key";
const MAX_ATTESTATION_OBJECT_LENGTH: usize = 32 * 1024;

struct RelyingParty {
    id: String,
    origin: String,
}

impl RelyingParty {
    /// A browser accepts only a domain name as the `rpId`, so it refuses a passkey at an IP address.
    /// Paron leaves that check to the browser.
    fn for_origin(origin: &url::Origin) -> Self {
        let id = match origin {
            url::Origin::Tuple(_, url::Host::Domain(domain), _) => {
                psl::domain_str(domain).unwrap_or(domain).to_owned()
            }
            url::Origin::Tuple(_, host, _) => host.to_string(),
            url::Origin::Opaque(_) => origin.ascii_serialization(),
        };

        Self {
            id,
            origin: origin.ascii_serialization(),
        }
    }
}

#[derive(Deserialize)]
struct CollectedClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin", default)]
    cross_origin: bool,
    #[serde(rename = "topOrigin")]
    top_origin: Option<String>,
}

fn decoded(text: &str) -> Result<Vec<u8>, InvalidPasskey> {
    URL_SAFE_NO_PAD_INDIFFERENT
        .decode(text)
        .map_err(|_| InvalidPasskey::Malformed)
}

fn credential_id(id: &str, raw_id: &[u8], kind: &str) -> Result<CredentialId, InvalidPasskey> {
    if kind != PUBLIC_KEY || decoded(id)? != raw_id {
        return Err(InvalidPasskey::Malformed);
    }

    Ok(CredentialId(raw_id.to_vec()))
}

/// §7.1 steps 7 to 11 and §7.2 steps 10 to 14.
fn check_client_data(
    client_data_json: &[u8],
    kind: &str,
    expected_challenge: &Challenge,
    relying_party: &RelyingParty,
) -> Result<(), InvalidPasskey> {
    let client_data: CollectedClientData =
        serde_json::from_slice(client_data_json).map_err(|_| InvalidPasskey::Malformed)?;
    let challenge = decoded(&client_data.challenge)?;

    if client_data.kind != kind {
        return Err(InvalidPasskey::OtherCeremony);
    }
    if !bool::from(challenge.ct_eq(expected_challenge.0.as_slice())) {
        return Err(InvalidPasskey::WrongChallenge);
    }
    if client_data.origin != relying_party.origin
        || client_data.cross_origin
        || client_data.top_origin.is_some()
    {
        return Err(InvalidPasskey::WrongOrigin);
    }

    Ok(())
}

/// §7.1 steps 14 to 17 and §7.2 steps 15 to 18, without user verification: the caller decides
/// if it needs it.
fn check_authenticator_data(
    data: &AuthenticatorData,
    relying_party: &RelyingParty,
) -> Result<(), InvalidPasskey> {
    let rp_id_hash = ring::digest::digest(&ring::digest::SHA256, relying_party.id.as_bytes());

    if !bool::from(data.rp_id_hash.ct_eq(rp_id_hash.as_ref())) {
        return Err(InvalidPasskey::WrongRelyingParty);
    }
    if !data.user_present() {
        return Err(InvalidPasskey::UserNotPresent);
    }
    if data.backup_state() && !data.backup_eligible() {
        return Err(InvalidPasskey::InconsistentBackupFlags);
    }

    Ok(())
}

fn user_verification(data: &AuthenticatorData) -> PasskeyUserVerification {
    if data.user_verified() {
        PasskeyUserVerification::Verified
    } else {
        PasskeyUserVerification::Unverified
    }
}

fn backup_of(data: &AuthenticatorData) -> PasskeyBackup {
    PasskeyBackup {
        eligible: data.backup_eligible(),
        state: data.backup_state(),
    }
}

/// The attestation statement formats that Paron verifies. Paron asks for the conveyance `none`, so
/// a browser replaces each statement with `none`, except a self attestation (§5.4.7).
enum AttestationStatement {
    /// §8.7.
    None,
    /// §8.2, without a certificate: the credential private key signed it.
    PackedSelf { algorithm: i64, signature: Vec<u8> },
}

/// The statement and the authenticator data of an attestation object (§6.5.4). A canonical map has
/// its keys in this order. A format that Paron cannot verify fails the registration (§7.1).
fn attestation_of(
    attestation_object: &[u8],
) -> Result<(AttestationStatement, Vec<u8>), InvalidPasskey> {
    if attestation_object.len() > MAX_ATTESTATION_OBJECT_LENGTH {
        return Err(InvalidPasskey::Malformed);
    }

    let Some(Value::Map(entries)) = cbor::read_all(attestation_object) else {
        return Err(InvalidPasskey::Malformed);
    };
    let [
        (format_key, format),
        (statement_key, statement),
        (data_key, data),
    ] = entries.as_slice()
    else {
        return Err(InvalidPasskey::Malformed);
    };
    let (Some("fmt"), Some("attStmt"), Some("authData")) = (
        format_key.as_text(),
        statement_key.as_text(),
        data_key.as_text(),
    ) else {
        return Err(InvalidPasskey::Malformed);
    };
    let (Some(format), Some(statement), Some(data)) =
        (format.as_text(), statement.as_map(), data.as_bytes())
    else {
        return Err(InvalidPasskey::Malformed);
    };

    let statement = match (format, statement.as_slice()) {
        ("none", []) => AttestationStatement::None,
        ("packed", [(algorithm_key, algorithm), (signature_key, signature)])
            if algorithm_key.as_text() == Some("alg") && signature_key.as_text() == Some("sig") =>
        {
            let (Some(algorithm), Some(signature)) = (
                algorithm
                    .as_integer()
                    .and_then(|algorithm| i64::try_from(algorithm).ok()),
                signature.as_bytes(),
            ) else {
                return Err(InvalidPasskey::Malformed);
            };

            AttestationStatement::PackedSelf {
                algorithm,
                signature: signature.clone(),
            }
        }
        // A certificate needs a trust anchor, and Paron has none.
        ("packed", [_, _, (certificates_key, _)]) if certificates_key.as_text() == Some("x5c") => {
            return Err(InvalidPasskey::UnsupportedAttestation);
        }
        ("none" | "packed", _) => return Err(InvalidPasskey::Malformed),
        _ => return Err(InvalidPasskey::UnsupportedAttestation),
    };

    Ok((statement, data.clone()))
}

pub fn start_registration(
    origin: &url::Origin,
    user: &User,
    exclude_credentials: &[CredentialId],
    timeout: Duration,
) -> StartedPasskeyRegistration {
    let relying_party = RelyingParty::for_origin(origin);
    let challenge = Challenge::generate();

    let options = PublicKeyCredentialCreationOptions {
        rp: webauthn_rs_proto::RelyingParty {
            name: RP_NAME.to_owned(),
            id: relying_party.id,
        },
        user: webauthn_rs_proto::User {
            id: user.id.as_bytes().to_vec(),
            name: user.username.to_string(),
            display_name: user.display_name.to_string(),
        },
        challenge: challenge.0.to_vec(),
        pub_key_cred_params: cose::ALGORITHMS
            .iter()
            .map(|&alg| PubKeyCredParams {
                type_: PUBLIC_KEY.to_owned(),
                alg,
            })
            .collect(),
        timeout: u32::try_from(timeout.as_millis()).ok(),
        exclude_credentials: Some(
            exclude_credentials
                .iter()
                .map(|credential_id| PublicKeyCredentialDescriptor {
                    type_: PUBLIC_KEY.to_owned(),
                    id: credential_id.0.clone(),
                    transports: None,
                })
                .collect(),
        ),
        authenticator_selection: Some(AuthenticatorSelectionCriteria {
            authenticator_attachment: None,
            resident_key: Some(ResidentKeyRequirement::Required),
            require_resident_key: true,
            user_verification: UserVerificationPolicy::Required,
        }),
        hints: None,
        attestation: Some(AttestationConveyancePreference::None),
        attestation_formats: None,
        extensions: Some(RequestRegistrationExtensions {
            // Asked for, not enforced: a browser refuses every authenticator without the
            // extension when the request enforces it.
            cred_protect: Some(CredProtect {
                credential_protection_policy: CredentialProtectionPolicy::UserVerificationRequired,
                enforce_credential_protection_policy: Some(false),
            }),
            uvm: None,
            cred_props: Some(true),
            min_pin_length: None,
            hmac_create_secret: None,
        }),
    };

    StartedPasskeyRegistration { options, challenge }
}

pub fn finish_registration(
    origin: &url::Origin,
    challenge: &Challenge,
    response: &RegisterPublicKeyCredential,
) -> Result<NewPasskey, InvalidPasskey> {
    let relying_party = RelyingParty::for_origin(origin);
    let raw_id = credential_id(&response.id, &response.raw_id, &response.type_)?;

    check_client_data(
        &response.response.client_data_json,
        "webauthn.create",
        challenge,
        &relying_party,
    )?;
    let (statement, authenticator_data) = attestation_of(&response.response.attestation_object)?;
    let data = AuthenticatorData::parse(&authenticator_data).ok_or(InvalidPasskey::Malformed)?;
    check_authenticator_data(&data, &relying_party)?;

    let verification = user_verification(&data);
    let sign_count = data.sign_count;
    let backup = backup_of(&data);
    let attested = data.attested_credential.ok_or(InvalidPasskey::Malformed)?;
    if attested.credential_id != raw_id.0 {
        return Err(InvalidPasskey::OtherCredential);
    }
    let key = CoseKey::parse(&attested.public_key).ok_or(InvalidPasskey::UnsupportedKey)?;

    // §8.2: a self attestation is signed with the algorithm and the private key of the
    // credential, over the authenticator data and the hash of the client data.
    if let AttestationStatement::PackedSelf {
        algorithm,
        signature,
    } = statement
    {
        let client_data_hash =
            ring::digest::digest(&ring::digest::SHA256, &response.response.client_data_json);
        let signed = [&authenticator_data[..], client_data_hash.as_ref()].concat();
        if algorithm != key.algorithm() || !key.verify(&signed, &signature) {
            return Err(InvalidPasskey::WrongSignature);
        }
    }

    Ok(NewPasskey {
        credential_id: raw_id,
        public_key: attested.public_key,
        user_verification: verification,
        sign_count,
        backup,
    })
}

pub fn start_authentication(
    origin: &url::Origin,
    allow_credentials: &[CredentialId],
    timeout: Duration,
) -> StartedPasskeyAuthentication {
    let relying_party = RelyingParty::for_origin(origin);
    let challenge = Challenge::generate();

    let options = PublicKeyCredentialRequestOptions {
        challenge: challenge.0.to_vec(),
        timeout: u32::try_from(timeout.as_millis()).ok(),
        rp_id: relying_party.id,
        allow_credentials: allow_credentials
            .iter()
            .map(|credential_id| AllowCredentials {
                type_: PUBLIC_KEY.to_owned(),
                id: credential_id.0.clone(),
                transports: None,
            })
            .collect(),
        user_verification: UserVerificationPolicy::Required,
        hints: None,
        extensions: None,
    };

    StartedPasskeyAuthentication { options, challenge }
}

pub fn finish_authentication(
    origin: &url::Origin,
    challenge: &Challenge,
    response: &PublicKeyCredential,
    passkey: &PasskeyCredential,
) -> Result<PasskeyAssertion, InvalidPasskey> {
    let relying_party = RelyingParty::for_origin(origin);
    let raw_id = credential_id(&response.id, &response.raw_id, &response.type_)?;
    if raw_id != passkey.credential_id {
        return Err(InvalidPasskey::OtherCredential);
    }

    let client_data_json = &response.response.client_data_json;
    check_client_data(client_data_json, "webauthn.get", challenge, &relying_party)?;
    let authenticator_data = &response.response.authenticator_data;
    let data = AuthenticatorData::parse(authenticator_data).ok_or(InvalidPasskey::Malformed)?;
    check_authenticator_data(&data, &relying_party)?;

    if data.attested_credential.is_some() {
        return Err(InvalidPasskey::Malformed);
    }

    // §7.2 steps 20 and 21: the signature over the authenticator data and the hash of the
    // client data.
    let client_data_hash = ring::digest::digest(&ring::digest::SHA256, client_data_json);
    let signed = [&authenticator_data[..], client_data_hash.as_ref()].concat();
    let key = CoseKey::parse(&passkey.public_key).ok_or(InvalidPasskey::UnsupportedKey)?;
    if !key.verify(&signed, &response.response.signature) {
        return Err(InvalidPasskey::WrongSignature);
    }

    // §7.2 step 22: a counter that does not grow shows that the authenticator may be a copy. A
    // synced passkey keeps the counter at zero.
    if (data.sign_count != 0 || passkey.sign_count != 0) && data.sign_count <= passkey.sign_count {
        return Err(InvalidPasskey::CounterNotIncreased);
    }

    // §7.2: the backup eligibility of a passkey never changes; the backup state can.
    let backup = backup_of(&data);
    if backup.eligible != passkey.backup.eligible {
        return Err(InvalidPasskey::BackupEligibilityChanged);
    }

    Ok(PasskeyAssertion {
        user_verification: user_verification(&data),
        sign_count: data.sign_count,
        backup,
    })
}

#[cfg(test)]
mod tests {
    use passkey::authenticator::{Authenticator, UiHint, UserCheck, UserValidationMethod};
    use passkey::client::{Client, DefaultClientData};
    use passkey::crypto::rust_crypto::RustCryptoBackend;
    use passkey::types::Passkey;
    use passkey::types::ctap2::{Aaguid, Ctap2Error};
    use passkey::types::webauthn::{CredentialCreationOptions, CredentialRequestOptions};
    use std::assert_matches;

    use super::*;
    use crate::application::identity::authentication::methods::passkey::PasskeyName;
    use crate::testing::identity;

    const ORIGIN: &str = "https://paron.example.com:8443";
    const TIMEOUT: Duration = Duration::from_mins(5);

    struct ApprovesEverything;

    #[async_trait::async_trait]
    impl UserValidationMethod for ApprovesEverything {
        type PasskeyItem = Passkey;

        async fn check_user<'a>(
            &self,
            _hint: UiHint<'a, Self::PasskeyItem>,
            _presence: bool,
            _verification: bool,
        ) -> Result<UserCheck, Ctap2Error> {
            Ok(UserCheck {
                presence: true,
                verification: true,
            })
        }

        fn is_verification_enabled(&self) -> Option<bool> {
            Some(true)
        }

        fn is_presence_enabled(&self) -> bool {
            true
        }
    }

    fn origin() -> url::Origin {
        url::Url::parse(ORIGIN).unwrap().origin()
    }

    /// A new passkey from the test authenticator, which sends a `none` statement.
    async fn create(user: &User) -> (Option<Passkey>, Challenge, RegisterPublicKeyCredential) {
        let mut browser = Client::new(Authenticator::new(
            Aaguid::new_empty(),
            None,
            ApprovesEverything,
            RustCryptoBackend,
        ));
        let StartedPasskeyRegistration { options, challenge } =
            start_registration(&origin(), user, &[], TIMEOUT);
        let created = browser
            .register(
                &url::Url::parse(ORIGIN).unwrap(),
                CredentialCreationOptions {
                    public_key: serde_json::from_value(serde_json::to_value(options).unwrap())
                        .unwrap(),
                },
                DefaultClientData,
            )
            .await
            .unwrap();

        (
            browser.authenticator().store().clone(),
            challenge,
            serde_json::from_value(serde_json::to_value(created).unwrap()).unwrap(),
        )
    }

    /// The response with another attestation statement in its attestation object.
    fn with_statement(
        mut response: RegisterPublicKeyCredential,
        format: &str,
        statement: Vec<(Value, Value)>,
    ) -> RegisterPublicKeyCredential {
        let Some(Value::Map(mut entries)) =
            cbor::read_all(response.response.attestation_object.as_ref())
        else {
            panic!("the attestation object is not a map");
        };
        entries[0].1 = Value::Text(format.to_owned());
        entries[1].1 = Value::Map(statement);
        let mut attestation_object = Vec::new();
        ciborium::ser::into_writer(&Value::Map(entries), &mut attestation_object).unwrap();
        response.response.attestation_object = attestation_object;

        response
    }

    async fn register(user: &User) -> (Option<Passkey>, PasskeyCredential) {
        let (key, challenge, response) = create(user).await;
        let registered = finish_registration(&origin(), &challenge, &response).unwrap();

        let passkey = PasskeyCredential {
            id: uuid::Uuid::now_v7(),
            user_id: user.id,
            credential_id: registered.credential_id,
            public_key: registered.public_key,
            name: PasskeyName::parse("Test key").unwrap(),
            sign_count: 0,
            backup: registered.backup,
            created_at: jiff::Timestamp::UNIX_EPOCH,
            last_used_at: None,
        };

        (key, passkey)
    }

    async fn sign(
        key: Option<&Passkey>,
        options: PublicKeyCredentialRequestOptions,
        origin: &str,
    ) -> PublicKeyCredential {
        let assertion = Client::new(Authenticator::new(
            Aaguid::new_empty(),
            key.cloned(),
            ApprovesEverything,
            RustCryptoBackend,
        ))
        .authenticate(
            &url::Url::parse(origin).unwrap(),
            CredentialRequestOptions {
                public_key: serde_json::from_value(serde_json::to_value(options).unwrap()).unwrap(),
            },
            DefaultClientData,
        )
        .await
        .unwrap();

        serde_json::from_value(serde_json::to_value(assertion).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn finish_authentication_with_the_registered_passkey_returns_verified() {
        let user = identity::user_builder().build();
        let (key, passkey) = register(&user).await;
        let StartedPasskeyAuthentication { options, challenge } =
            start_authentication(&origin(), &[], TIMEOUT);
        let response = sign(key.as_ref(), options, ORIGIN).await;

        let finished = finish_authentication(&origin(), &challenge, &response, &passkey);

        assert_matches!(
            finished,
            Ok(PasskeyAssertion {
                user_verification: PasskeyUserVerification::Verified,
                ..
            })
        );
    }

    #[tokio::test]
    async fn finish_authentication_with_another_backup_eligibility_returns_backup_eligibility_changed()
     {
        let user = identity::user_builder().build();
        let (key, passkey) = register(&user).await;
        let StartedPasskeyAuthentication { options, challenge } =
            start_authentication(&origin(), &[], TIMEOUT);
        let response = sign(key.as_ref(), options, ORIGIN).await;
        let passkey = PasskeyCredential {
            backup: PasskeyBackup {
                eligible: !passkey.backup.eligible,
                ..passkey.backup
            },
            ..passkey
        };

        let finished = finish_authentication(&origin(), &challenge, &response, &passkey);

        assert_matches!(finished, Err(InvalidPasskey::BackupEligibilityChanged));
    }

    #[tokio::test]
    async fn finish_authentication_with_a_counter_that_did_not_grow_returns_counter_not_increased()
    {
        let user = identity::user_builder().build();
        let (key, passkey) = register(&user).await;
        let StartedPasskeyAuthentication { options, challenge } =
            start_authentication(&origin(), &[], TIMEOUT);
        let response = sign(key.as_ref(), options, ORIGIN).await;
        let passkey = PasskeyCredential {
            sign_count: u32::MAX,
            ..passkey
        };

        let finished = finish_authentication(&origin(), &challenge, &response, &passkey);

        assert_matches!(finished, Err(InvalidPasskey::CounterNotIncreased));
    }

    #[tokio::test]
    async fn finish_registration_for_another_site_returns_wrong_origin() {
        let user = identity::user_builder().build();
        let (_, challenge, response) = create(&user).await;
        let other_origin = url::Url::parse("https://other.example").unwrap().origin();

        let registered = finish_registration(&other_origin, &challenge, &response);

        assert_matches!(registered, Err(InvalidPasskey::WrongOrigin));
    }

    #[tokio::test]
    async fn finish_registration_with_a_packed_self_attestation_returns_the_passkey() {
        let user = identity::user_builder().build();
        let (key, challenge, response) = create(&user).await;
        let Some(Value::Map(entries)) =
            cbor::read_all(response.response.attestation_object.as_ref())
        else {
            panic!("the attestation object is not a map");
        };
        let authenticator_data = entries[2].1.as_bytes().unwrap();
        let client_data_hash =
            ring::digest::digest(&ring::digest::SHA256, &response.response.client_data_json);
        let private_key = key
            .unwrap()
            .key
            .params
            .into_iter()
            .find_map(|(label, value)| (label == coset::Label::Int(-4)).then_some(value))
            .and_then(|value| value.into_bytes().ok())
            .unwrap();
        let attested = AuthenticatorData::parse(authenticator_data)
            .unwrap()
            .attested_credential
            .unwrap();
        let Some(CoseKey::Ed25519 { x }) = CoseKey::parse(&attested.public_key) else {
            panic!("the passkey is not Ed25519");
        };
        let signature = ring::signature::Ed25519KeyPair::from_seed_and_public_key(&private_key, &x)
            .unwrap()
            .sign(&[&authenticator_data[..], client_data_hash.as_ref()].concat());
        let response = with_statement(
            response,
            "packed",
            vec![
                (Value::Text("alg".to_owned()), Value::from(-8)),
                (
                    Value::Text("sig".to_owned()),
                    Value::Bytes(signature.as_ref().to_vec()),
                ),
            ],
        );

        let registered = finish_registration(&origin(), &challenge, &response);

        assert_matches!(registered, Ok(_));
    }

    #[tokio::test]
    async fn finish_registration_with_a_wrong_packed_signature_returns_wrong_signature() {
        let user = identity::user_builder().build();
        let (_, challenge, response) = create(&user).await;
        let response = with_statement(
            response,
            "packed",
            vec![
                (Value::Text("alg".to_owned()), Value::from(-8)),
                (Value::Text("sig".to_owned()), Value::Bytes(vec![1; 64])),
            ],
        );

        let registered = finish_registration(&origin(), &challenge, &response);

        assert_matches!(registered, Err(InvalidPasskey::WrongSignature));
    }

    #[tokio::test]
    async fn finish_registration_with_an_attestation_object_longer_than_the_limit_returns_malformed()
     {
        let user = identity::user_builder().build();
        let (_, challenge, response) = create(&user).await;
        let response = with_statement(
            response,
            "packed",
            vec![
                (Value::Text("alg".to_owned()), Value::from(-8)),
                (
                    Value::Text("sig".to_owned()),
                    Value::Bytes(vec![1; MAX_ATTESTATION_OBJECT_LENGTH]),
                ),
            ],
        );

        let registered = finish_registration(&origin(), &challenge, &response);

        assert_matches!(registered, Err(InvalidPasskey::Malformed));
    }

    #[tokio::test]
    async fn finish_registration_with_a_certificate_or_another_format_returns_unsupported_attestation()
     {
        let user = identity::user_builder().build();
        let (_, challenge, response) = create(&user).await;
        let with_certificate = with_statement(
            response.clone(),
            "packed",
            vec![
                (Value::Text("alg".to_owned()), Value::from(-7)),
                (Value::Text("sig".to_owned()), Value::Bytes(vec![1; 64])),
                (
                    Value::Text("x5c".to_owned()),
                    Value::Array(vec![Value::Bytes(vec![1; 64])]),
                ),
            ],
        );
        let other_format = with_statement(response, "tpm", Vec::new());

        assert_matches!(
            finish_registration(&origin(), &challenge, &with_certificate),
            Err(InvalidPasskey::UnsupportedAttestation)
        );
        assert_matches!(
            finish_registration(&origin(), &challenge, &other_format),
            Err(InvalidPasskey::UnsupportedAttestation)
        );
    }

    #[tokio::test]
    async fn finish_authentication_from_another_origin_returns_wrong_origin() {
        let user = identity::user_builder().build();
        let (key, passkey) = register(&user).await;
        let StartedPasskeyAuthentication { options, challenge } =
            start_authentication(&origin(), &[], TIMEOUT);
        let response = sign(key.as_ref(), options, "https://paron.example.com:9443").await;

        let assertion = finish_authentication(&origin(), &challenge, &response, &passkey);

        assert_matches!(assertion, Err(InvalidPasskey::WrongOrigin));
    }

    #[tokio::test]
    async fn finish_authentication_under_another_name_of_the_domain_returns_verified() {
        let user = identity::user_builder().build();
        let (key, passkey) = register(&user).await;
        let other_name = url::Url::parse("https://lan.example.com").unwrap().origin();
        let StartedPasskeyAuthentication { options, challenge } =
            start_authentication(&other_name, &[], TIMEOUT);
        let response = sign(key.as_ref(), options, "https://lan.example.com").await;

        let finished = finish_authentication(&other_name, &challenge, &response, &passkey);

        assert_matches!(finished, Ok(_));
    }

    #[test]
    fn start_authentication_returns_the_registrable_domain_as_the_relying_party() {
        for (origin, rp_id) in [
            ("https://paron.example.com:8443", "example.com"),
            ("https://a.b.example.co.uk", "example.co.uk"),
            ("https://paron.local:8443", "paron.local"),
            ("https://localhost:8443", "localhost"),
        ] {
            let started =
                start_authentication(&url::Url::parse(origin).unwrap().origin(), &[], TIMEOUT);

            assert_eq!(started.options.rp_id, rp_id, "{origin}");
        }
    }

    #[tokio::test]
    async fn finish_authentication_with_another_challenge_returns_wrong_challenge() {
        let user = identity::user_builder().build();
        let (key, passkey) = register(&user).await;
        let StartedPasskeyAuthentication { options, .. } =
            start_authentication(&origin(), &[], TIMEOUT);
        let StartedPasskeyAuthentication {
            challenge: other_challenge,
            ..
        } = start_authentication(&origin(), &[], TIMEOUT);
        let response = sign(key.as_ref(), options, ORIGIN).await;

        let assertion = finish_authentication(&origin(), &other_challenge, &response, &passkey);

        assert_matches!(assertion, Err(InvalidPasskey::WrongChallenge));
    }
}

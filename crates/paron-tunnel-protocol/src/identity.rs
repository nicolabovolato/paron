//! Identities and their encodings (RFC §2.1, §2.2).

use secrecy::{ExposeSecret as _, SecretSlice};
use x509_parser::prelude::FromDer as _;

use crate::error::TunnelError;

/// Who an agent claims to be: the 16-octet binary form of RFC 9562 on the wire (§2.1).
///
/// Public by definition — it appears in operator records and logs, and SEC-1 does not depend on
/// it being secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgentId(uuid::Uuid);

impl AgentId {
    pub fn new(id: uuid::Uuid) -> Self {
        Self(id)
    }

    /// The string representation of RFC 9562 in any case, and only that representation: braces,
    /// a `urn:uuid` prefix and the unhyphenated form are refused, since an identifier that
    /// round-trips two ways is two identifiers to whoever greps the logs.
    pub fn parse(text: &str) -> Option<Self> {
        let parsed = uuid::Uuid::try_parse(text).ok()?;
        let mut buffer = uuid::Uuid::encode_buffer();

        text.eq_ignore_ascii_case(parsed.hyphenated().encode_lower(&mut buffer))
            .then_some(Self(parsed))
    }

    pub fn uuid(self) -> uuid::Uuid {
        self.0
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A public key in the canonical encoding of §2.2: a DER-encoded `SubjectPublicKeyInfo`.
///
/// Comparison is byte equality of the DER, which is all POL-1 does with it. Validated on the way
/// in, so anything that exists is a key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublicKey(Vec<u8>);

impl PublicKey {
    pub fn parse(bytes: impl Into<Vec<u8>>) -> Result<Self, NotAKey> {
        let bytes = bytes.into();
        let (rest, _) =
            x509_parser::x509::SubjectPublicKeyInfo::from_der(&bytes).map_err(|_| NotAKey)?;

        if !rest.is_empty() {
            return Err(NotAKey);
        }

        Ok(Self(bytes))
    }

    /// The `SubjectPublicKeyInfo` of a certificate, which §2.2 makes the whole of the identity:
    /// every other field is ignored (TRN-2, TRN-3).
    pub fn of_certificate(certificate_der: &[u8]) -> Result<Self, NotAKey> {
        let (_, parsed) =
            x509_parser::parse_x509_certificate(certificate_der).map_err(|_| NotAKey)?;

        Self::parse(parsed.tbs_certificate.subject_pki.raw.to_vec())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// What a token pins this key by.
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint {
            hash: Fingerprint::WRITTEN,
            digest: Fingerprint::WRITTEN.digest(&self.0),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("expected a DER-encoded SubjectPublicKeyInfo")]
pub struct NotAKey;

/// One party's key material: a private key and the self-signed certificate carrying its public
/// half into the handshake (§2.2).
///
/// The RFC's "agent identity" pairs a key with a name; this is the key material alone, the name
/// travelling in `CLAIM`. The private half never leaves the host (POL-9).
pub struct Identity {
    private_key_der: SecretSlice<u8>,
    certificate_der: Vec<u8>,
    public_key: PublicKey,
}

impl Identity {
    /// Mint a fresh keypair and its self-signed certificate.
    pub fn generate() -> Result<Self, NotAKeypair> {
        let key = rcgen::KeyPair::generate().map_err(|_| NotAKeypair)?;

        Self::assemble(&key)
    }

    /// Rebuild from a stored PKCS#8 DER private key. The certificate is re-minted rather than
    /// stored: peers compare the `SubjectPublicKeyInfo` alone (§2.2), so one that differs
    /// between runs still names the same identity.
    pub fn from_pkcs8_der(private_key_der: &SecretSlice<u8>) -> Result<Self, NotAKeypair> {
        let key =
            rcgen::KeyPair::try_from(private_key_der.expose_secret()).map_err(|_| NotAKeypair)?;

        Self::assemble(&key)
    }

    fn assemble(key: &rcgen::KeyPair) -> Result<Self, NotAKeypair> {
        let params =
            rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|_| NotAKeypair)?;
        let certificate = params.self_signed(key).map_err(|_| NotAKeypair)?;
        let certificate_der = certificate.der().to_vec();
        let public_key = PublicKey::of_certificate(&certificate_der).map_err(|_| NotAKeypair)?;

        Ok(Self {
            private_key_der: SecretSlice::from(key.serialize_der()),
            certificate_der,
            public_key,
        })
    }

    /// The public half (§2.2): what a broker publishes for agents to pin, and what an agent's
    /// claim is judged against (POL-1).
    pub fn public_key(&self) -> PublicKey {
        self.public_key.clone()
    }

    /// The private key, for persisting. Written restricted, never logged (POL-9).
    pub fn private_key_der(&self) -> &SecretSlice<u8> {
        &self.private_key_der
    }

    /// The container §2.2 asks for: TLS has no way to present a bare key, so the key travels in
    /// a certificate whose other fields the peer discards.
    pub(crate) fn certificate_der(&self) -> &[u8] {
        &self.certificate_der
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("expected a PKCS#8 DER private key this crate can sign with")]
pub struct NotAKeypair;

/// What an enrollment token pins: a hash of the broker's root `SubjectPublicKeyInfo`, with the
/// hash named in front of it.
///
/// A token holds this rather than the key itself for one reason, and it is the reason the format
/// has to outlive every algorithm in it. A key grows with its algorithm — 44 octets for the
/// Ed25519 of today, a few thousand for a lattice signature — and a token an operator pastes into
/// a shell cannot grow with it. A hash does not: this is 32 octets whatever it names, now and
/// after the key algorithm has been replaced twice.
///
/// The name in front is what makes the hash itself replaceable. An agent that meets a hash it
/// does not know refuses and says which one it met, rather than comparing octets of two different
/// things and finding they differ.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    hash: Hash,
    digest: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Hash {
    Sha256,
}

impl Hash {
    const NAMES: [(&'static str, Self); 1] = [("sha256", Self::Sha256)];

    fn name(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
        }
    }

    fn digest(self, bytes: &[u8]) -> Vec<u8> {
        use sha2::Digest as _;

        match self {
            Self::Sha256 => sha2::Sha256::digest(bytes).to_vec(),
        }
    }
}

impl Fingerprint {
    /// The hash a broker minting a token today names. Reading accepts every hash this build
    /// knows, so the one written can move on without stranding the tokens already handed out.
    const WRITTEN: Hash = Hash::Sha256;

    pub fn parse(text: &str) -> Result<Self, NotAFingerprint> {
        let (name, digest) = text.split_once(':').ok_or(NotAFingerprint)?;

        let hash = Hash::NAMES
            .into_iter()
            .find_map(|(known, hash)| (known == name).then_some(hash))
            .ok_or(NotAFingerprint)?;

        let digest = base16ct::lower::decode_vec(digest).map_err(|_| NotAFingerprint)?;

        if digest.len() != hash.digest(b"").len() {
            return Err(NotAFingerprint);
        }

        Ok(Self { hash, digest })
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            out,
            "{}:{}",
            self.hash.name(),
            base16ct::lower::encode_string(&self.digest)
        )
    }
}

/// The pin appears in tokens, logs and operator records, so it reads the same in all of them.
impl std::fmt::Debug for Fingerprint {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{self}")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("expected a fingerprint of the form sha256:<hex>")]
pub struct NotAFingerprint;

const ROOT_SUBJECT: &str = "Paron broker root";
const SERVING_SUBJECT: &str = "Paron broker";

/// RFC 5280 §4.1.2.5: the value that says a certificate has no well-defined expiration date.
/// IEEE 802.1AR gives a device identity the same one, for the same reason paron does — nobody can
/// reach the machines that hold the pin, so an expiry date is an outage with a date on it and no
/// way to recover from it.
fn never() -> time::OffsetDateTime {
    time::macros::datetime!(9999-12-31 23:59:59 UTC)
}

/// Where an agent keeps the rollback index it has accepted (TRN-2).
///
/// A stolen serving key holds a certificate the root key signed, and that signature stays good
/// for ever. Minting a replacement takes nothing away from the thief, because the agent cannot
/// tell the two certificates apart. The index is what tells them apart: it only goes up, and an
/// agent that has met a higher one refuses the certificate the thief holds.
///
/// A date would do the same job and is what web PKI uses. It is not used here because the agent
/// would then need a working clock to connect, and a host with a dead clock is a host nobody can
/// reach any more. TUF, Uptane and Android Verified Boot count for the same reason.
pub trait RollbackIndex: std::fmt::Debug + Send + Sync {
    /// The highest index this agent has accepted, or zero if it has accepted none.
    ///
    /// An index that cannot be read is an error rather than a zero. Reading a lost record as
    /// "nothing accepted yet" is the rollback this exists to refuse.
    fn highest(&self) -> Result<u64, TunnelError>;

    /// Record an index higher than [`highest`](RollbackIndex::highest) returned, unless a higher
    /// one is recorded by now. The connection attempts of one race (§6.3) can pass TRN-2
    /// together, each having read the index before the other recorded, and the lower index must
    /// not overwrite the higher (POL-11).
    fn accepted(&self, index: u64) -> Result<(), TunnelError>;
}

/// A rollback index that lasts as long as the process, for an agent with nowhere to write: a
/// test, a benchmark, or an embedder in a browser or a unikernel.
///
/// It refuses an index below one this process has already accepted, and it starts again at zero
/// on every restart. An agent that can write keeps [`RollbackIndex`] on disk instead.
#[derive(Debug, Default)]
pub struct InMemoryRollbackIndex(std::sync::atomic::AtomicU64);

impl RollbackIndex for InMemoryRollbackIndex {
    fn highest(&self) -> Result<u64, TunnelError> {
        Ok(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn accepted(&self, index: u64) -> Result<(), TunnelError> {
        self.0
            .fetch_max(index, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }
}

/// The broker's key material, in two halves that do different jobs.
///
/// The **root** key is what an agent pins at enrollment, and it is the one thing here that must
/// never change: an agent offline for a year comes back holding nothing else, and there is no way
/// to tell it otherwise (§12.4). It signs and it never serves, so it can one day live somewhere
/// the running broker cannot reach, and losing the host stops costing every machine.
///
/// The **serving** key is minted at every start, and carries a rollback index the root signs over.
/// The handshake proves possession of the serving key, and the index is what makes replacing it
/// mean something: an agent refuses an index below the one it has accepted (TRN-2,
/// [`RollbackIndex`]).
///
/// Neither certificate carries a usable expiry, and no agent reads a date from them. That is what
/// keeps a machine with a dead clock, or one restored from a two-year-old snapshot, able to
/// connect at all.
pub struct BrokerIdentity {
    root: PublicKey,
    root_der: Vec<u8>,
    serving: PublicKey,
    serving_key_der: SecretSlice<u8>,
    serving_der: Vec<u8>,
}

impl BrokerIdentity {
    /// A new root key. Only an installation does this: minting a second one is what §12.4 calls
    /// locking out every agent that pinned the first.
    pub fn generate_root() -> Result<SecretSlice<u8>, NotAKeypair> {
        Ok(SecretSlice::from(
            rcgen::KeyPair::generate()
                .map_err(|_| NotAKeypair)?
                .serialize_der(),
        ))
    }

    /// A root key and a certificate under it in one step, for a test or an embedder with nowhere
    /// to keep a root. An installation mints the root once with [`BrokerIdentity::generate_root`]
    /// and loads it on every start.
    pub fn generate() -> Result<Self, NotAKeypair> {
        Self::from_root_pkcs8_der(&Self::generate_root()?, 1)
    }

    /// Load the root key and mint the certificate this run serves under it.
    ///
    /// `rollback_index` must be higher than the one the last run served, and it must not come
    /// from a clock: a broker whose clock moved backwards would serve an index below one its
    /// agents have accepted, and lock itself out of its own fleet ([`RollbackIndex`]).
    pub fn from_root_pkcs8_der(
        root_key_der: &SecretSlice<u8>,
        rollback_index: u64,
    ) -> Result<Self, NotAKeypair> {
        let root_key =
            rcgen::KeyPair::try_from(root_key_der.expose_secret()).map_err(|_| NotAKeypair)?;

        let mut root_params =
            rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|_| NotAKeypair)?;

        root_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        root_params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
        root_params.not_after = never();
        root_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, ROOT_SUBJECT);

        let root_certificate = root_params
            .self_signed(&root_key)
            .map_err(|_| NotAKeypair)?;

        let serving_key = rcgen::KeyPair::generate().map_err(|_| NotAKeypair)?;
        let mut serving_params =
            rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|_| NotAKeypair)?;

        serving_params.not_after = never();
        serving_params.serial_number = Some(rcgen::SerialNumber::from(rollback_index));
        serving_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, SERVING_SUBJECT);

        let serving = serving_params
            .signed_by(
                &serving_key,
                &rcgen::Issuer::from_params(&root_params, &root_key),
            )
            .map_err(|_| NotAKeypair)?;

        Ok(Self {
            root: PublicKey::of_certificate(root_certificate.der()).map_err(|_| NotAKeypair)?,
            root_der: root_certificate.der().to_vec(),
            serving: PublicKey::of_certificate(serving.der()).map_err(|_| NotAKeypair)?,
            serving_key_der: SecretSlice::from(serving_key.serialize_der()),
            serving_der: serving.der().to_vec(),
        })
    }

    /// The key an agent pins, by [`PublicKey::fingerprint`].
    pub fn root_public_key(&self) -> PublicKey {
        self.root.clone()
    }

    /// The key this run's handshakes prove possession of. Different on every start, and nothing
    /// pins it.
    pub fn serving_public_key(&self) -> PublicKey {
        self.serving.clone()
    }

    /// The serving certificate first, then the one the root key travels in. An agent takes the
    /// root key out of the second and checks that it signed the first (TRN-2).
    pub(crate) fn chain(&self) -> Vec<Vec<u8>> {
        vec![self.serving_der.clone(), self.root_der.clone()]
    }

    pub(crate) fn serving_key_der(&self) -> &SecretSlice<u8> {
        &self.serving_key_der
    }
}

impl std::fmt::Debug for BrokerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerIdentity")
            .field("root", &self.root.fingerprint())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn of_certificate_of_identity_returns_its_public_key() {
        let identity = Identity::generate().unwrap();

        let public_key = PublicKey::of_certificate(identity.certificate_der()).unwrap();

        assert_eq!(public_key, identity.public_key());
    }

    #[test]
    fn certificate_der_excludes_private_key() {
        let identity = Identity::generate().unwrap();
        let private_key = identity.private_key_der().expose_secret();

        let certificate = identity.certificate_der();

        assert!(
            !certificate
                .windows(private_key.len())
                .any(|window| window == private_key)
        );
    }

    #[test]
    fn parse_public_key_of_arbitrary_bytes_returns_error() {
        let public_key = PublicKey::parse(b"not a key".to_vec());

        assert_matches!(public_key, Err(NotAKey));
    }

    #[test]
    fn from_pkcs8_der_of_private_key_returns_same_public_key() {
        let identity = Identity::generate().unwrap();

        let reloaded_identity = Identity::from_pkcs8_der(identity.private_key_der()).unwrap();

        assert_eq!(reloaded_identity.public_key(), identity.public_key());
    }

    #[test]
    fn parse_agent_id_hyphenated_in_any_case_returns_id() {
        let id = uuid::Uuid::now_v7();
        let lowercase_uuid = id.hyphenated().to_string();

        let parsed = [lowercase_uuid.clone(), lowercase_uuid.to_uppercase()]
            .map(|text| AgentId::parse(&text));

        assert_eq!(parsed, [Some(AgentId::new(id)); 2]);
    }

    #[test]
    fn parse_agent_id_other_uuid_forms_returns_none() {
        let lowercase_uuid = uuid::Uuid::now_v7().hyphenated().to_string();

        let parsed = [
            format!("urn:uuid:{lowercase_uuid}"),
            format!("{{{lowercase_uuid}}}"),
            lowercase_uuid.replace('-', ""),
        ]
        .map(|text| AgentId::parse(&text));

        assert_eq!(parsed, [None; 3]);
    }
}

//! The TLS layer both transport mappings share (TRN-2, TRN-3, TRN-4, TRN-12).
//!
//! One identity layer for both mappings is what lets one configured broker key authenticate the
//! broker on either. Certificates are containers for a `SubjectPublicKeyInfo` (§2.2): the broker
//! completes the handshake whatever client certificate arrives, judgement being POL-1's (SEC-7),
//! and the agent compares the broker's `SubjectPublicKeyInfo` with the key it was configured with
//! and nothing else.
//!
//! Certificates rather than RFC 7250 raw public keys, although the model is raw keys: every TLS
//! stack can mint and present a self-signed certificate, while raw-public-key support is rare
//! (Go's crypto/tls has none), and §11 wants conformance runnable against an implementation in
//! another language.

use std::sync::{Arc, LazyLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};
use secrecy::ExposeSecret as _;

use crate::error::TunnelError;
use crate::identity::{BrokerIdentity, Fingerprint, Identity, PublicKey, RollbackIndex};
use x509_parser::prelude::FromDer as _;

/// The ALPN identifier of PTP/1 (TRN-12): a connection that does not negotiate it is not a PTP
/// connection.
pub const ALPN: &[u8] = b"ptp1";

/// What the binary installed at startup, which is where paron says which crypto it speaks.
///
/// A test binary installs nothing, and neither does an embedder that left the choice to us. Both
/// get ring, which carries its own crypto with software fallbacks.
pub(crate) fn crypto_provider() -> Arc<CryptoProvider> {
    static FALLBACK: LazyLock<Arc<CryptoProvider>> =
        LazyLock::new(|| Arc::new(rustls::crypto::ring::default_provider()));

    CryptoProvider::get_default().map_or_else(|| Arc::clone(&FALLBACK), Arc::clone)
}

/// The broker's TLS configuration: TLS 1.3 only, client certificate required and never judged
/// at this layer (TRN-3), no session tickets and no early data (TRN-4).
pub(crate) fn broker_config(
    identity: &BrokerIdentity,
) -> Result<Arc<rustls::ServerConfig>, TunnelError> {
    let crypto_provider = crypto_provider();
    let verifier = Arc::new(ClientKeyVerifier {
        crypto_provider: Arc::clone(&crypto_provider),
    });

    let mut config = rustls::ServerConfig::builder_with_provider(crypto_provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(TunnelError::unexpected)?
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            identity
                .chain()
                .into_iter()
                .map(CertificateDer::from)
                .collect(),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                identity.serving_key_der().expose_secret().to_vec(),
            )),
        )
        .map_err(TunnelError::unexpected)?;

    // TRN-4: a resumed handshake carries no CertificateVerify, so nothing resumable is issued and
    // nothing early is accepted. Both are rustls' defaults; TRN-9 says relying on a library's
    // defaults does not discharge a requirement, and a default is one dependency bump from being
    // something else.
    config.send_tls13_tickets = 0;
    config.max_early_data_size = 0;
    config.alpn_protocols = vec![ALPN.to_vec()];

    Ok(Arc::new(config))
}

/// The agent's TLS configuration: the broker's public key checked during the handshake, before
/// the agent's own certificate is transmitted (TRN-2), and possession of the agent key proven
/// by the handshake (TRN-3).
pub(crate) fn agent_config(
    identity: &Identity,
    broker_root: &Fingerprint,
    rollback: Arc<dyn RollbackIndex>,
) -> Result<Arc<rustls::ClientConfig>, TunnelError> {
    let crypto_provider = crypto_provider();
    let verifier = Arc::new(ServerKeyVerifier {
        root: broker_root.clone(),
        rollback,
        crypto_provider: Arc::clone(&crypto_provider),
    });

    let mut config = rustls::ClientConfig::builder_with_provider(crypto_provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(TunnelError::unexpected)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_client_auth_cert(
            vec![CertificateDer::from(identity.certificate_der().to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                identity.private_key_der().expose_secret().to_vec(),
            )),
        )
        .map_err(TunnelError::unexpected)?;

    config.resumption = rustls::client::Resumption::disabled();
    config.enable_early_data = false;
    config.alpn_protocols = vec![ALPN.to_vec()];

    Ok(Arc::new(config))
}

/// The name rustls demands for the peer. It travels as SNI when it is a DNS name and is omitted
/// for an IP literal; it is never verified against the certificate, §2.2 ignoring every field but
/// the key. Syntax the API wants, not trust.
pub(crate) fn server_name(host: &str) -> Result<ServerName<'static>, TunnelError> {
    ServerName::try_from(host.to_owned()).map_err(TunnelError::unexpected)
}

/// TRN-3: the handshake completes whatever certificate is presented, and a certificate is
/// required. Rejecting at the TLS layer would distinguish an unknown key from a known one by
/// the layer that answers (SEC-7); the only check here is that a `SubjectPublicKeyInfo` can be
/// extracted at all, without which there is nothing to judge later.
#[derive(Debug)]
struct ClientKeyVerifier {
    crypto_provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for ClientKeyVerifier {
    /// TRN-3: the certificate is asked for, and a handshake presenting none fails. Both are
    /// rustls' defaults and both are stated, because a requirement discharged by a default is one
    /// nobody reading this can see, and TRN-9 makes the same argument about transport capabilities.
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    /// No issuer is hinted, because none is wanted: the certificate is a container for a key
    /// (§2.2) and who signed it decides nothing.
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        PublicKey::of_certificate(end_entity).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;

        Ok(ClientCertVerified::assertion())
    }

    /// Unreachable — both configurations select TLS 1.3 — and failing closed in case a change
    /// ever reaches it (TRN-4).
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("PTP/1 is TLS 1.3 only".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.crypto_provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.crypto_provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Whether `issuer` signed `certificate`.
///
/// Every algorithm the provider supports is tried, and none is named here. That is the whole
/// reason this is written out rather than handed to a path builder: the set of algorithms paron
/// accepts becomes a property of the crypto provider, so a broker whose root key is one this
/// build has never heard of works the day the provider learns it.
///
/// Nothing but the signature is looked at. No dates, no basic constraints, no name — §2.2 makes
/// the key the whole of the identity, and a bounded depth of one is what TRN-2 allows.
fn signed_by(
    certificate: &CertificateDer<'_>,
    issuer: &PublicKey,
    crypto_provider: &CryptoProvider,
) -> bool {
    let (Ok((_, certificate)), Ok((_, issuer))) = (
        x509_parser::parse_x509_certificate(certificate),
        x509_parser::x509::SubjectPublicKeyInfo::from_der(issuer.as_bytes()),
    ) else {
        return false;
    };

    let key = issuer.subject_public_key.as_ref();
    let message = certificate.tbs_certificate.as_ref();
    let signature = certificate.signature_value.as_ref();

    crypto_provider
        .signature_verification_algorithms
        .all
        .iter()
        .any(|algorithm| algorithm.verify_signature(key, message, signature).is_ok())
}

/// The rollback index of a serving certificate: its X.509 serial number (TRN-2, TRN-13).
///
/// `None` is a certificate no broker of this protocol minted, and the caller refuses it. Eight
/// octets is the whole of the range TRN-13 allows, so a wider one is not a number this comparison
/// is defined over.
fn rollback_index(certificate: &CertificateDer<'_>) -> Option<u64> {
    let (_, certificate) = x509_parser::parse_x509_certificate(certificate).ok()?;
    let octets = certificate.tbs_certificate.raw_serial();
    let leading_zeros = octets
        .iter()
        .position(|octet| *octet != 0)
        .unwrap_or(octets.len());
    let significant = &octets[leading_zeros..];

    (significant.len() <= 8).then(|| {
        significant
            .iter()
            .fold(0_u64, |index, octet| (index << 8) | u64::from(*octet))
    })
}

/// Why an agent refused a certificate the pinned root did sign. It reaches the agent's own log and
/// no further: a TLS alert carries no text, and the broker learns only that the handshake failed.
#[derive(Debug, thiserror::Error)]
enum RolledBack {
    #[error("the broker served rollback index {served}, below the {accepted} this agent accepted")]
    Lower { served: u64, accepted: u64 },

    #[error("the accepted rollback index could not be read")]
    Unreadable(#[source] TunnelError),

    #[error("rollback index {0} could not be recorded")]
    Unrecorded(u64, #[source] TunnelError),
}

fn rolled_back(refusal: RolledBack) -> rustls::Error {
    rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
        Arc::new(refusal),
    )))
}

/// TRN-2: the server is accepted only if the root key the agent pinned signed the certificate
/// the server presented, and the certificate is no older than the last one this agent accepted.
///
/// Three things are checked and nothing else. The chain's second certificate carries a key whose
/// fingerprint must equal the pin, that key must have signed the first, and the first must carry a
/// rollback index at or above the one already accepted. Validity, names, basic constraints and
/// every other field are ignored (§2.2), so no agent is ever locked out by a date — not by a root
/// that aged out, and not by its own clock being wrong. The handshake's `CertificateVerify`,
/// checked by [`ServerCertVerifier::verify_tls13_signature`], is what proves the server holds the
/// serving key.
///
/// No algorithm is named here. The signature is checked against every algorithm the crypto
/// provider supports, so the day a broker's root key becomes a lattice key is the day the provider
/// grows one, and this code does not change.
#[derive(Debug)]
struct ServerKeyVerifier {
    root: Fingerprint,
    rollback: Arc<dyn RollbackIndex>,
    crypto_provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for ServerKeyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let bad_encoding =
            || rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding);
        // Aborting the handshake here is CNF-5: nothing of the protocol, the agent identifier
        // included, has been transmitted yet.
        let unknown_issuer =
            || rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);

        // Depth is bounded at one link rather than walked. A longer chain is refused rather
        // than searched: nothing in it could be accepted that the pin did not already authorise,
        // and a path builder is where certificate policies and name constraints get in.
        let [carrier] = intermediates else {
            return Err(unknown_issuer());
        };

        let root = PublicKey::of_certificate(carrier).map_err(|_| bad_encoding())?;

        if root.fingerprint() != self.root {
            return Err(unknown_issuer());
        }

        if !signed_by(end_entity, &root, &self.crypto_provider) {
            return Err(unknown_issuer());
        }

        // Read after the signature, never before: only the pinned root can set this field, so a
        // certificate nobody signed has to fail as an unknown issuer rather than as a rollback.
        let served = rollback_index(end_entity).ok_or_else(bad_encoding)?;
        let accepted = self
            .rollback
            .highest()
            .map_err(|reason| rolled_back(RolledBack::Unreadable(reason)))?;

        if served < accepted {
            return Err(rolled_back(RolledBack::Lower { served, accepted }));
        }

        // An agent that cannot record the index would accept the certificate a thief holds on its
        // next connection, so a record that cannot be written ends the handshake rather than
        // being dropped.
        if served > accepted {
            self.rollback
                .accepted(served)
                .map_err(|reason| rolled_back(RolledBack::Unrecorded(served, reason)))?;
        }

        Ok(ServerCertVerified::assertion())
    }

    /// Unreachable and failing closed, as on [`ClientKeyVerifier`].
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("PTP/1 is TLS 1.3 only".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.crypto_provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.crypto_provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use rustls::CertificateError;
    use secrecy::SecretSlice;

    use super::*;
    use crate::identity::InMemoryRollbackIndex;

    fn client_verifier() -> ClientKeyVerifier {
        ClientKeyVerifier {
            crypto_provider: crypto_provider(),
        }
    }

    fn server_verifier(root: &PublicKey) -> ServerKeyVerifier {
        ServerKeyVerifier {
            root: root.fingerprint(),
            rollback: Arc::new(InMemoryRollbackIndex::default()),
            crypto_provider: crypto_provider(),
        }
    }

    // rustls has no public constructor, so this decodes the wire form.
    fn digitally_signed_struct() -> DigitallySignedStruct {
        use rustls::internal::msgs::codec::{Codec as _, Reader};

        let mut signature_wire_bytes = vec![0x04, 0x03, 0x00, 0x04];
        signature_wire_bytes.extend_from_slice(&[0xAA; 4]);

        DigitallySignedStruct::read(&mut Reader::init(&signature_wire_bytes)).unwrap()
    }

    fn verify_chain(
        verifier: &ServerKeyVerifier,
        chain: Vec<Vec<u8>>,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let chain: Vec<_> = chain.into_iter().map(CertificateDer::from).collect();

        verifier.verify_server_cert(
            &chain[0],
            &chain[1..],
            &ServerName::try_from("broker.invalid").unwrap(),
            &[],
            UnixTime::now(),
        )
    }

    fn broker_identity_at_index(root: &SecretSlice<u8>, index: u64) -> BrokerIdentity {
        BrokerIdentity::from_root_pkcs8_der(root, index).unwrap()
    }

    #[test]
    fn verify_tls12_signature_by_broker_returns_error() {
        let identity = Identity::generate().unwrap();
        let certificate = CertificateDer::from(identity.certificate_der().to_vec());

        let verdict = client_verifier().verify_tls12_signature(
            b"transcript",
            &certificate,
            &digitally_signed_struct(),
        );

        assert_matches!(verdict, Err(rustls::Error::General(_)));
    }

    #[test]
    fn verify_tls12_signature_by_agent_returns_error() {
        let identity = Identity::generate().unwrap();
        let certificate = CertificateDer::from(identity.certificate_der().to_vec());

        let verdict = server_verifier(&identity.public_key()).verify_tls12_signature(
            b"transcript",
            &certificate,
            &digitally_signed_struct(),
        );

        assert_matches!(verdict, Err(rustls::Error::General(_)));
    }

    #[test]
    fn verify_client_cert_not_a_certificate_returns_error() {
        let certificate = CertificateDer::from(b"not a certificate".to_vec());

        let verdict = client_verifier().verify_client_cert(&certificate, &[], UnixTime::now());

        assert_matches!(
            verdict,
            Err(rustls::Error::InvalidCertificate(
                CertificateError::BadEncoding
            ))
        );
    }

    #[test]
    fn verify_client_cert_unknown_key_returns_ok() {
        let unknown_identity = Identity::generate().unwrap();
        let certificate = CertificateDer::from(unknown_identity.certificate_der().to_vec());

        let verdict = client_verifier().verify_client_cert(&certificate, &[], UnixTime::now());

        assert_matches!(verdict, Ok(_));
    }

    #[test]
    fn verify_server_cert_signed_by_pinned_root_returns_ok() {
        let broker = BrokerIdentity::generate().unwrap();
        let verifier = server_verifier(&broker.root_public_key());

        let verdict = verify_chain(&verifier, broker.chain());

        assert_matches!(verdict, Ok(_));
    }

    // CNF-5.
    #[test]
    fn verify_server_cert_of_other_broker_returns_error() {
        let broker = BrokerIdentity::generate().unwrap();
        let other_broker = BrokerIdentity::generate().unwrap();
        let verifier = server_verifier(&broker.root_public_key());

        let verdict = verify_chain(&verifier, other_broker.chain());

        assert_matches!(
            verdict,
            Err(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer
            ))
        );
    }

    #[test]
    fn verify_server_cert_not_signed_by_pinned_root_returns_error() {
        let broker = BrokerIdentity::generate().unwrap();
        let other_broker = BrokerIdentity::generate().unwrap();
        let verifier = server_verifier(&broker.root_public_key());
        let chain = vec![other_broker.chain()[0].clone(), broker.chain()[1].clone()];

        let verdict = verify_chain(&verifier, chain);

        assert_matches!(
            verdict,
            Err(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer
            ))
        );
    }

    #[test]
    fn verify_server_cert_without_root_returns_error() {
        let broker = BrokerIdentity::generate().unwrap();
        let verifier = server_verifier(&broker.root_public_key());
        let chain = vec![broker.chain()[0].clone()];

        let verdict = verify_chain(&verifier, chain);

        assert_matches!(
            verdict,
            Err(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer
            ))
        );
    }

    #[test]
    fn verify_server_cert_with_extra_root_returns_error() {
        let broker = BrokerIdentity::generate().unwrap();
        let other_broker = BrokerIdentity::generate().unwrap();
        let verifier = server_verifier(&broker.root_public_key());
        let mut chain = broker.chain();
        chain.push(other_broker.chain()[1].clone());

        let verdict = verify_chain(&verifier, chain);

        assert_matches!(
            verdict,
            Err(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer
            ))
        );
    }

    #[test]
    fn verify_server_cert_with_root_not_a_certificate_returns_error() {
        let broker = BrokerIdentity::generate().unwrap();
        let verifier = server_verifier(&broker.root_public_key());
        let chain = vec![broker.chain()[0].clone(), b"not a certificate".to_vec()];

        let verdict = verify_chain(&verifier, chain);

        assert_matches!(
            verdict,
            Err(rustls::Error::InvalidCertificate(
                CertificateError::BadEncoding
            ))
        );
    }

    // The certificate a thief holds must not outlive the serving key that replaced it.
    #[test]
    fn verify_server_cert_below_accepted_index_returns_error() {
        let root = BrokerIdentity::generate_root().unwrap();
        let lower_index_broker = broker_identity_at_index(&root, 4);
        let higher_index_broker = broker_identity_at_index(&root, 5);
        let verifier = server_verifier(&lower_index_broker.root_public_key());
        verify_chain(&verifier, higher_index_broker.chain()).unwrap();

        let verdict = verify_chain(&verifier, lower_index_broker.chain());

        assert_matches!(
            verdict,
            Err(rustls::Error::InvalidCertificate(CertificateError::Other(
                _
            )))
        );
    }

    #[test]
    fn verify_server_cert_at_accepted_index_returns_ok() {
        let root = BrokerIdentity::generate_root().unwrap();
        let broker = broker_identity_at_index(&root, 9);
        let verifier = server_verifier(&broker.root_public_key());
        verify_chain(&verifier, broker.chain()).unwrap();

        let verdict = verify_chain(&verifier, broker.chain());

        assert_matches!(verdict, Ok(_));
    }

    #[test]
    fn verify_server_cert_with_rollback_index_that_cannot_record_returns_error() {
        #[derive(Debug)]
        struct RollbackIndexThatCannotRecord;

        impl RollbackIndex for RollbackIndexThatCannotRecord {
            fn highest(&self) -> Result<u64, TunnelError> {
                Ok(0)
            }

            fn accepted(&self, _index: u64) -> Result<(), TunnelError> {
                Err(TunnelError::fault("nowhere to write"))
            }
        }

        let broker = BrokerIdentity::generate().unwrap();
        let verifier = ServerKeyVerifier {
            root: broker.root_public_key().fingerprint(),
            rollback: Arc::new(RollbackIndexThatCannotRecord),
            crypto_provider: crypto_provider(),
        };

        let verdict = verify_chain(&verifier, broker.chain());

        assert_matches!(
            verdict,
            Err(rustls::Error::InvalidCertificate(CertificateError::Other(
                _
            )))
        );
    }

    #[test]
    fn rollback_index_of_serial_wider_than_eight_octets_returns_none() {
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        // With no serial set, rcgen writes a 20-octet hash.
        params.serial_number = None;
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = CertificateDer::from(params.self_signed(&key).unwrap().der().to_vec());

        let index = rollback_index(&certificate);

        assert_eq!(index, None);
    }
}

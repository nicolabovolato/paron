pub mod acme;
pub mod self_signed;
pub mod x509;

use std::sync::Arc;

use rustls::crypto::CryptoProvider;

use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

use crate::Result;
use self_signed::SelfSignedCertificate;
use x509::Identifier;

#[expect(
    clippy::expect_used,
    reason = "main installs the provider before any command runs"
)]
pub fn provider() -> &'static Arc<CryptoProvider> {
    CryptoProvider::get_default().expect("no crypto provider is installed")
}

pub struct Tls {
    pub config: Arc<rustls::ServerConfig>,
    pub acme_challenges: Vec<acme::Certificate>,
}

pub fn configure(
    data_dir: &std::path::Path,
    public_urls: &[crate::server::cli::PublicUrl],
    certificate: &crate::server::cli::CertificateArgs,
    background: &mut crate::server::run::background_tasks::BackgroundTasks,
) -> Result<(Tls, SelfSignedCertificate)> {
    let names = public_urls
        .iter()
        .map(|url| Identifier::try_from(url.host()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let self_signed = self_signed::configure(data_dir, &names, background)?;
    let acme_challenges = acme::configure(data_dir, &names, certificate, background)?;

    if acme_challenges.is_empty() && names.iter().any(acme::can_be_ordered) {
        use crate::server::cli::ServerRunArgs;

        tracing::warn!(
            "browsers will not trust Paron, ACME was not configured. Fix this with {} or {}",
            ServerRunArgs::NAMES.certificate.letsencrypt.flag,
            ServerRunArgs::NAMES.certificate.acme_directory.flag
        );
    }

    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(SplitCertResolver {
            acme: acme_challenges.clone(),
            self_signed: self_signed.clone(),
        }));

    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    Ok((
        Tls {
            config: Arc::new(tls),
            acme_challenges,
        },
        self_signed,
    ))
}

/// Resolves a client handshake using ACME issued certificates, falling back to a self signed certificate for failed or not requested names.
#[derive(Debug)]
struct SplitCertResolver {
    acme: Vec<acme::Certificate>,
    self_signed: SelfSignedCertificate,
}

impl ResolvesServerCert for SplitCertResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        // Normalize server name into our own type. this is validated by rustls before us.
        let identifier = hello
            .server_name()
            .and_then(|server_name| server_name.parse::<Identifier>().ok());

        // Is ACME configured to answer for this identifier?
        let ordered = identifier.and_then(|identifier| {
            self.acme
                .iter()
                .find(|certificate| *certificate.identifier() == identifier)
        });

        // Should the ACME resolver fail, we offer the self signed cert.
        Some(
            ordered
                .and_then(|certificate| certificate.resolve(hello))
                .unwrap_or_else(|| self.self_signed.keypair()),
        )
    }
}

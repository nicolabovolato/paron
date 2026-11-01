use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rustls::server::ClientHello;
use rustls::sign::CertifiedKey;
use rustls_acme::caches::DirCache;
use rustls_acme::{AcmeConfig, AcmeState, EventOk, ResolvesServerCertAcme, UseChallenge};
use tokio::net::TcpStream;
use tokio_rustls::server::StartHandshake;

use super::x509::Identifier;
use crate::Result;
use crate::server::cli::{LetsEncrypt, ServerRunArgs};

const CACHE_DIR: &str = ".cache/acme";

pub fn configure(
    data_dir: &Path,
    names: &[Identifier],
    certificate: &crate::server::cli::CertificateArgs,
    background: &mut crate::server::run::background_tasks::BackgroundTasks,
) -> Result<Vec<Certificate>> {
    let directory = match (certificate.acme_directory.as_ref(), certificate.letsencrypt) {
        (Some(url), _) => url.as_str(),
        (None, Some(LetsEncrypt::Production)) => {
            rustls_acme::acme::LETS_ENCRYPT_PRODUCTION_DIRECTORY
        }
        (None, Some(LetsEncrypt::Staging)) => rustls_acme::acme::LETS_ENCRYPT_STAGING_DIRECTORY,
        (None, None) => return Ok(Vec::new()),
    };

    let (identifiers, skipped_identifiers): (Vec<Identifier>, Vec<Identifier>) =
        names.iter().cloned().partition(can_be_ordered);

    if identifiers.is_empty() {
        return Err(format!(
            "no domain in {} can be ordered through ACME",
            ServerRunArgs::NAMES.public.urls.flag,
        )
        .into());
    }

    if !skipped_identifiers.is_empty() {
        tracing::warn!("{skipped_identifiers:?} cannot be ordered through ACME");
    }

    tracing::info!("ordering certificates from {directory} for {identifiers:?}");
    if certificate.letsencrypt == Some(LetsEncrypt::Staging) {
        tracing::warn!("these certificates are not publicly trusted and browsers will reject them");
    }

    Ok(identifiers
        .into_iter()
        .map(|identifier| {
            let state = AcmeConfig::new_with_provider(
                [identifier.to_string()],
                Arc::clone(super::provider()),
            )
            .contact(
                certificate
                    .acme_contact
                    .as_deref()
                    .map(|contact| format!("mailto:{contact}")),
            )
            .cache(DirCache::new(data_dir.join(CACHE_DIR)))
            .directory(directory)
            .challenge_type(UseChallenge::TlsAlpn01)
            .state();

            let certificate = Certificate {
                identifier: identifier.clone(),
                resolver: state.resolver().clone(),
                challenge: state
                    .challenge_rustls_config_with_provider(Arc::clone(super::provider())),
            };

            background.add(renewal(identifier, state));

            certificate
        })
        .collect())
}

/// Whether an authority can be asked for this name at all.
// TODO: is_ip_address() only used here because rustls_acme does not support it. tls-alpn-01 does support it. fix.
pub fn can_be_ordered(name: &Identifier) -> bool {
    !name.is_local() && !name.is_mdns() && !name.is_ip_address()
}

pub async fn handshake(
    start_tls_handshake: StartHandshake<TcpStream>,
    peer: SocketAddr,
    challenges: &[Certificate],
) {
    let identifier = start_tls_handshake
        .client_hello()
        .server_name()
        .and_then(|asked_for| asked_for.parse::<Identifier>().ok());

    let certificate = identifier.as_ref().and_then(|identifier| {
        challenges
            .iter()
            .find(|certificate| certificate.identifier() == identifier)
    });

    let Some(certificate) = certificate else {
        tracing::debug!(%peer, ?identifier, "received ACME challenge for an unknown identifier");

        return;
    };

    let identifier = certificate.identifier();

    match start_tls_handshake
        .into_stream(certificate.challenge())
        .await
    {
        Ok(_) => tracing::debug!(%peer, %identifier, "ACME challenge passed"),
        Err(error) => {
            tracing::debug!(%peer, %identifier, "ACME challenge failed: {error}");
        }
    }
}

#[derive(Clone, Debug)]
pub struct Certificate {
    identifier: Identifier,
    resolver: Arc<ResolvesServerCertAcme>,
    challenge: Arc<rustls::ServerConfig>,
}

impl Certificate {
    pub fn identifier(&self) -> &Identifier {
        &self.identifier
    }

    pub fn challenge(&self) -> Arc<rustls::ServerConfig> {
        Arc::clone(&self.challenge)
    }

    pub fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        use rustls::server::ResolvesServerCert as _;

        self.resolver.resolve(hello)
    }
}

/// `rustls-acme` already has a backoff strategy for failed order but it does not trigger when
/// an order succeeds and the certificate fails to parse, and that resets the backoff time. so,
/// without this, we could be polling a future indefinitely, without delay.
const MIN_INTERVAL: Duration = Duration::from_secs(5);
const MAX_INTERVAL: Duration = Duration::from_mins(30);

#[tracing::instrument(name = "acme", skip_all, fields(identifier = %identifier))]
async fn renewal(identifier: Identifier, mut state: AcmeState<std::io::Error, std::io::Error>) {
    use futures_util::StreamExt as _;

    let mut interval = MIN_INTERVAL;
    let mut attempted = tokio::time::Instant::now();

    while let Some(event) = state.next().await {
        match event {
            Ok(progress) => {
                interval = MIN_INTERVAL;

                match progress {
                    EventOk::DeployedNewCert => {
                        tracing::info!("new certificate issued and in use");
                    }
                    EventOk::DeployedCachedCert => {
                        tracing::info!("using the cached certificate");
                    }
                    EventOk::CertCacheStore => {
                        tracing::debug!("certificate written to the cache");
                    }
                    EventOk::AccountCacheStore => {
                        tracing::debug!("account key written to the cache");
                    }
                }
            }
            Err(error) => {
                tracing::warn!("could not obtain certificate: {error}");

                tokio::time::sleep_until(attempted + interval).await;

                interval = (interval * 2).min(MAX_INTERVAL);
            }
        }

        attempted = tokio::time::Instant::now();
    }

    tracing::error!("renewal ended, stopping so a restart can begin it again");
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::server::cli::CertificateArgs;
    use crate::server::run::background_tasks::BackgroundTasks;
    use crate::testing::{TempDataDir, parse_each};

    fn staging_certificate_args() -> CertificateArgs {
        CertificateArgs {
            letsencrypt: Some(LetsEncrypt::Staging),
            ..Default::default()
        }
    }

    #[test]
    fn configure_without_a_domain_returns_error() {
        let data_dir = TempDataDir::new("acme-no-domain");

        let configured = configure(
            data_dir.path(),
            &parse_each(&["localhost", "127.0.0.1"]),
            &staging_certificate_args(),
            &mut BackgroundTasks::new(),
        );

        assert_matches!(configured, Err(_));
    }

    #[test]
    fn configure_orders_every_name_that_is_not_local_mdns_or_an_address() {
        let data_dir = TempDataDir::new("acme-left-out");
        let names = [
            "paron.example.com",
            "under_score.com",
            "paron.internal",
            "paron",
            "localhost",
            "127.0.0.1",
            "::1",
            "10.0.0.5",
            "paron.local",
            "a.b.local",
        ];

        let certificates = configure(
            data_dir.path(),
            &parse_each(&names),
            &staging_certificate_args(),
            &mut BackgroundTasks::new(),
        )
        .unwrap();

        let ordered: Vec<_> = certificates
            .iter()
            .map(|certificate| certificate.identifier().clone())
            .collect();
        assert_eq!(
            ordered,
            parse_each::<Identifier>(&[
                "paron.example.com",
                "under_score.com",
                "paron.internal",
                "paron"
            ])
        );
    }

    #[test]
    fn configure_answers_each_name_with_its_own_resolver() {
        let data_dir = TempDataDir::new("acme-resolvers");

        let certificates = configure(
            data_dir.path(),
            &parse_each(&["a.example.com", "b.example.com"]),
            &staging_certificate_args(),
            &mut BackgroundTasks::new(),
        )
        .unwrap();

        assert_eq!(certificates.len(), 2);
        assert!(
            !Arc::ptr_eq(&certificates[0].resolver, &certificates[1].resolver),
            "both names are answered by the same resolver"
        );
    }
}

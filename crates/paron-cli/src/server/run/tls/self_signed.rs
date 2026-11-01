use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::{fs, io};

use parking_lot::RwLock;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::sign::CertifiedKey;

use super::x509::Identifier;
use crate::Result;
use crate::platform::files;
use crate::server::data_dir::self_signed_ca::{self, CertificateAuthority};

const RENEW_INTERVAL: Duration = Duration::from_hours(12);
const NOT_BEFORE_CLOCK_SKEW: time::Duration = time::Duration::hours(1);

const FILENAME: &str = "self-signed-certificate.pem";
const CERT_SUBJECT: &str = "Paron Server";
const CERT_LIFETIME: time::Duration = time::Duration::days(90);
const CERT_RENEW_WITHIN: time::Duration = time::Duration::days(30);

pub fn configure(
    data_dir: &Path,
    names: &[Identifier],
    background: &mut crate::server::run::background_tasks::BackgroundTasks,
) -> Result<SelfSignedCertificate> {
    let names = crate::server::local_hosts()
        .iter()
        .map(Identifier::try_from)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .chain(names.iter().cloned())
        .collect::<Vec<_>>();

    let path = data_dir.join(FILENAME);
    let authority = self_signed_ca::open(data_dir)?;
    let state = Arc::new(RwLock::new(State::open(&path, &authority, &names)?));

    background.add({
        let data_dir = data_dir.to_owned();
        let state = Arc::clone(&state);

        async move {
            use x509_parser::prelude::*;

            loop {
                tokio::time::sleep(RENEW_INTERVAL).await;

                let expires_in = state
                    .read()
                    .keypair
                    .end_entity_cert()
                    .ok()
                    .and_then(|serving| X509Certificate::from_der(serving).ok())
                    .and_then(|(_, serving)| serving.validity().time_to_expiration());

                let renewal = self_signed_ca::open(&data_dir).and_then(|authority| {
                    let due = authority.certificate() != state.read().ca_certificate
                        || expires_in.is_none_or(|left| left <= CERT_RENEW_WITHIN);

                    if !due {
                        return Ok(None);
                    }

                    State::open(&path, &authority, &names).map(Some)
                });

                match renewal {
                    Ok(None) => {}
                    Ok(Some(issued)) => *state.write() = issued,
                    Err(error) => {
                        if let Some(expires_in) = expires_in {
                            tracing::warn!(
                                "could not renew self-signed certificate, {} days left: {error}",
                                expires_in.whole_days()
                            );
                        } else {
                            tracing::error!(
                                "self-signed certificate expired, could not renew: {error}"
                            );

                            return;
                        }
                    }
                }
            }
        }
    });

    Ok(SelfSignedCertificate(state))
}

#[derive(Clone, Debug)]
pub struct SelfSignedCertificate(Arc<RwLock<State>>);

impl SelfSignedCertificate {
    pub fn keypair(&self) -> Arc<CertifiedKey> {
        Arc::clone(&self.0.read().keypair)
    }

    pub fn ca_certificate(&self) -> CertificateDer<'static> {
        self.0.read().ca_certificate.clone()
    }
}

#[derive(Debug)]
struct State {
    ca_certificate: CertificateDer<'static>,
    keypair: Arc<CertifiedKey>,
}

impl State {
    fn open(
        path: &Path,
        authority: &CertificateAuthority,
        subject_alt_names: &[Identifier],
    ) -> Result<Self> {
        use sha2::Digest as _;

        let stored = Self::read(path, authority, subject_alt_names).unwrap_or_else(|error| {
            tracing::warn!("self signed certificate must be issued again: {error}");
            None
        });
        let state = stored.map_or_else(|| Self::issue(path, authority, subject_alt_names), Ok)?;

        tracing::info!(
            "certificate issued under self signed CA SHA256:{}",
            base16ct::lower::encode_string(&sha2::Sha256::digest(&state.ca_certificate))
        );

        Ok(state)
    }

    fn issue(
        path: &Path,
        authority: &CertificateAuthority,
        subject_alt_names: &[Identifier],
    ) -> Result<Self> {
        let key = rcgen::KeyPair::generate()?;
        let mut params = rcgen::CertificateParams::new(
            subject_alt_names
                .iter()
                .map(Identifier::to_string)
                .collect::<Vec<_>>(),
        )?;

        params.insert_extended_key_usage(rcgen::ExtendedKeyUsagePurpose::ServerAuth);
        params.not_before = time::OffsetDateTime::now_utc() - NOT_BEFORE_CLOCK_SKEW;
        params.not_after = params.not_before + NOT_BEFORE_CLOCK_SKEW + CERT_LIFETIME;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, CERT_SUBJECT);

        let ca_certificate = authority.certificate();
        let chain = vec![authority.sign(&params, &key)?, ca_certificate.clone()];

        let chain_pem = chain
            .iter()
            .map(|certificate| {
                pem::encode_config(
                    &pem::Pem::new("CERTIFICATE", certificate.to_vec()),
                    pem::EncodeConfig::new().set_line_ending(pem::LineEnding::LF),
                )
            })
            .collect::<String>();
        let key_pem = zeroize::Zeroizing::new(key.serialize_pem());

        files::atomic_replace(
            path,
            zeroize::Zeroizing::new(chain_pem + &key_pem).as_bytes(),
        )
        .map_err(|reason| format!("{} could not be written: {reason}", path.display()))?;

        Ok(Self {
            ca_certificate,
            keypair: Arc::new(CertifiedKey::from_der(
                chain,
                PrivateKeyDer::try_from(key.serialize_der())?,
                super::provider(),
            )?),
        })
    }

    fn read(
        path: &Path,
        authority: &CertificateAuthority,
        subject_alt_names: &[Identifier],
    ) -> Result<Option<Self>> {
        use x509_parser::prelude::*;

        let contents = zeroize::Zeroizing::new(match fs::read(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            file => {
                file.map_err(|reason| format!("{} could not be read: {reason}", path.display()))?
            }
        });
        let chain = CertificateDer::pem_slice_iter(&contents).collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_slice(&contents)?;

        let [serving, ca_certificate] = chain.as_slice() else {
            return Err("the chain does not hold a certificate and its authority".into());
        };
        let ca_certificate = ca_certificate.clone();

        let (_, serving) = X509Certificate::from_der(serving)?;
        let stored_names = serving
            .subject_alternative_name()?
            .ok_or("missing subject alternative names")?
            .value
            .general_names
            .iter()
            .filter_map(|name| match name {
                GeneralName::DNSName(name) => name.parse().ok(),
                GeneralName::IPAddress(octets) => <[u8; 4]>::try_from(*octets)
                    .map(std::net::IpAddr::from)
                    .or_else(|_| <[u8; 16]>::try_from(*octets).map(std::net::IpAddr::from))
                    .ok()
                    .map(Identifier::Ip),
                _ => None,
            })
            .collect::<HashSet<_>>();

        if ca_certificate != authority.certificate()
            || serving
                .validity()
                .time_to_expiration()
                .is_none_or(|left| left <= CERT_RENEW_WITHIN)
            || stored_names != subject_alt_names.iter().cloned().collect()
        {
            return Ok(None);
        }

        Ok(Some(Self {
            ca_certificate,
            keypair: Arc::new(CertifiedKey::from_der(chain, key, super::provider())?),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use x509_parser::prelude::{FromDer as _, X509Certificate};

    use super::*;
    use crate::testing::{TempDataDir, parse_each};

    #[test]
    fn cert_lifetime_is_shorter_than_the_ca_renew_window() {
        assert!(self_signed_ca::CA_RENEW_WITHIN > CERT_LIFETIME);
    }

    // macOS and iOS refuse a server certificate that does not name serverAuth or that is valid
    // for more than 825 days.
    #[test]
    fn issue_returns_a_certificate_an_apple_platform_accepts() {
        let data_dir = TempDataDir::new("cert-apple");
        let authority = self_signed_ca::open(data_dir.path()).unwrap();

        let issued = State::issue(
            &data_dir.join(FILENAME),
            &authority,
            &["localhost".parse().unwrap()],
        )
        .unwrap();

        let (_, certificate) =
            X509Certificate::from_der(issued.keypair.end_entity_cert().unwrap()).unwrap();
        assert!(
            certificate
                .extended_key_usage()
                .unwrap()
                .unwrap()
                .value
                .server_auth,
            "the extended key usage has to name serverAuth"
        );
        let validity =
            (certificate.validity().not_after - certificate.validity().not_before).unwrap();
        assert!(
            validity <= ::time::Duration::days(825),
            "issued for {validity:?}"
        );
    }

    #[test]
    fn issue_puts_the_authority_in_the_chain() {
        let data_dir = TempDataDir::new("cert-chain");
        let authority = self_signed_ca::open(data_dir.path()).unwrap();

        let issued = State::issue(
            &data_dir.join(FILENAME),
            &authority,
            &["localhost".parse().unwrap()],
        )
        .unwrap();

        let chain = &issued.keypair.cert;
        assert_eq!(chain.len(), 2, "the authority is not in the chain");
        let (_, leaf_certificate) = X509Certificate::from_der(&chain[0]).unwrap();
        let (_, authority_certificate) = X509Certificate::from_der(&chain[1]).unwrap();
        assert_eq!(
            leaf_certificate.issuer().to_string(),
            authority_certificate.subject().to_string()
        );
    }

    #[test]
    fn issue_after_reopening_the_authority_keeps_it_and_returns_a_new_certificate() {
        let data_dir = TempDataDir::new("cert-kept");
        let first_issued = State::issue(
            &data_dir.join(FILENAME),
            &self_signed_ca::open(data_dir.path()).unwrap(),
            &["localhost".parse().unwrap()],
        )
        .unwrap();
        let reopened_authority = self_signed_ca::open(data_dir.path()).unwrap();

        let second_issued = State::issue(
            &data_dir.join(FILENAME),
            &reopened_authority,
            &["localhost".parse().unwrap()],
        )
        .unwrap();

        assert_eq!(
            second_issued.keypair.cert[1], first_issued.keypair.cert[1],
            "the authority was replaced"
        );
        assert_ne!(
            second_issued.keypair.cert[0], first_issued.keypair.cert[0],
            "the renewal task has nothing new to put behind the resolver"
        );
    }

    #[cfg(unix)]
    #[test]
    fn issue_writes_certificate_nobody_else_can_read() {
        use std::os::unix::fs::PermissionsExt as _;

        let data_dir = TempDataDir::new("cert-file-mode");

        State::issue(
            &data_dir.join(FILENAME),
            &self_signed_ca::open(data_dir.path()).unwrap(),
            &["localhost".parse().unwrap()],
        )
        .unwrap();

        let mode = fs::metadata(data_dir.join(FILENAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "found {:o}", mode & 0o777);
    }

    #[test]
    fn open_after_a_restart_returns_the_stored_certificate() {
        let data_dir = TempDataDir::new("cert-restart");
        let names = parse_each(&["localhost", "127.0.0.1", "::1", "paron.example.com"]);
        let first_opened = State::open(
            &data_dir.join(FILENAME),
            &self_signed_ca::open(data_dir.path()).unwrap(),
            &names,
        )
        .unwrap();

        let reopened = State::open(
            &data_dir.join(FILENAME),
            &self_signed_ca::open(data_dir.path()).unwrap(),
            &names.into_iter().rev().collect::<Vec<_>>(),
        )
        .unwrap();

        assert_eq!(reopened.keypair.cert, first_opened.keypair.cert);
    }

    #[test]
    fn read_with_another_name_returns_none() {
        let data_dir = TempDataDir::new("cert-new-name");
        let authority = self_signed_ca::open(data_dir.path()).unwrap();
        State::issue(
            &data_dir.join(FILENAME),
            &authority,
            &parse_each(&["localhost"]),
        )
        .unwrap();

        let stored = State::read(
            &data_dir.join(FILENAME),
            &authority,
            &parse_each(&["localhost", "10.0.0.5"]),
        );

        assert_matches!(stored, Ok(None));
    }

    #[test]
    fn read_under_a_new_authority_returns_none() {
        let data_dir = TempDataDir::new("cert-new-authority");
        State::issue(
            &data_dir.join(FILENAME),
            &self_signed_ca::open(data_dir.path()).unwrap(),
            &parse_each(&["localhost"]),
        )
        .unwrap();
        fs::remove_file(data_dir.join("self-signed-ca.pem")).unwrap();

        let stored = State::read(
            &data_dir.join(FILENAME),
            &self_signed_ca::open(data_dir.path()).unwrap(),
            &parse_each(&["localhost"]),
        );

        assert_matches!(stored, Ok(None));
    }

    #[test]
    fn open_corrupt_certificate_replaces_it() {
        let data_dir = TempDataDir::new("cert-file-corrupt");
        fs::write(data_dir.join(FILENAME), "this is not a certificate").unwrap();

        let opened = State::open(
            &data_dir.join(FILENAME),
            &self_signed_ca::open(data_dir.path()).unwrap(),
            &parse_each(&["localhost"]),
        );

        assert_matches!(opened, Ok(_));
    }
}

use std::io::Write as _;
use std::path::Path;
use std::{fs, io};

use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

use crate::Result;

const NOT_BEFORE_CLOCK_SKEW: time::Duration = time::Duration::hours(1);

const FILENAME: &str = "self-signed-ca.pem";

const CA_SUBJECT: &str = "Paron Root CA";
const CA_LIFETIME: time::Duration = time::Duration::days(365 * 10);
pub const CA_RENEW_WITHIN: time::Duration = time::Duration::days(120);

#[derive(Debug)]
pub struct CertificateAuthority {
    certificate: CertificateDer<'static>,
    issuer: rcgen::Issuer<'static, rcgen::KeyPair>,
}

impl CertificateAuthority {
    pub fn sign(
        &self,
        params: &rcgen::CertificateParams,
        key: &rcgen::KeyPair,
    ) -> Result<CertificateDer<'static>> {
        Ok(params.signed_by(key, &self.issuer)?.der().clone())
    }

    pub fn certificate(&self) -> CertificateDer<'static> {
        self.certificate.clone()
    }
}

pub fn open(data_dir: &Path) -> Result<CertificateAuthority> {
    let path = data_dir.join(FILENAME);

    let authority = match read(&path) {
        Ok(Some(authority)) => authority,
        Ok(None) => mint(&path)?,
        Err(err) => {
            tracing::warn!("self signed CA must be regenerated: {err}");

            mint(&path)?
        }
    };

    Ok(authority)
}

fn read(path: &Path) -> Result<Option<CertificateAuthority>> {
    use x509_parser::prelude::*;

    let pem = zeroize::Zeroizing::new(match fs::read(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        file => file.map_err(|reason| format!("{} could not be read: {reason}", path.display()))?,
    });

    let certificate = CertificateDer::from_pem_slice(&pem)?;
    let key = PrivateKeyDer::from_pem_slice(&pem)?;

    let (_, parsed) = X509Certificate::from_der(&certificate)?;

    match parsed.validity().time_to_expiration() {
        Some(left) if left > CA_RENEW_WITHIN => {}
        _ => return Err("renewal due".into()),
    }

    Ok(Some(CertificateAuthority {
        issuer: rcgen::Issuer::from_ca_cert_der(&certificate, rcgen::KeyPair::try_from(&key)?)?,
        certificate,
    }))
}

fn mint(path: &Path) -> Result<CertificateAuthority> {
    let key = rcgen::KeyPair::generate()?;
    let mut params = rcgen::CertificateParams::new(Vec::new())?;

    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    params.not_before = time::OffsetDateTime::now_utc() - NOT_BEFORE_CLOCK_SKEW;
    params.not_after = params.not_before + NOT_BEFORE_CLOCK_SKEW + CA_LIFETIME;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, CA_SUBJECT);

    let certificate = params.self_signed(&key)?;

    let key_pem = zeroize::Zeroizing::new(key.serialize_pem());
    fs::File::create(path)
        .and_then(|mut file| {
            file.write_all(certificate.pem().as_bytes())?;
            file.write_all(key_pem.as_bytes())
        })
        .map_err(|reason| format!("{} could not be written: {reason}", path.display()))?;

    let authority = CertificateAuthority {
        certificate: certificate.der().clone(),
        issuer: rcgen::Issuer::new(params, key),
    };

    tracing::info!("self signed CA generated");

    Ok(authority)
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::testing::TempDataDir;

    #[test]
    fn open_empty_directory_mints_a_ca_certificate() {
        use x509_parser::prelude::*;

        let data_dir = TempDataDir::new("cert-subject");

        let authority = open(data_dir.path()).unwrap();

        let (_, certificate) = X509Certificate::from_der(&authority.certificate).unwrap();
        assert_eq!(
            certificate.subject().to_string(),
            format!("CN={CA_SUBJECT}")
        );
        assert!(certificate.is_ca());
    }

    #[test]
    fn open_corrupt_authority_replaces_it() {
        let data_dir = TempDataDir::new("cert-corrupt");
        open(data_dir.path()).unwrap();
        fs::write(data_dir.join(FILENAME), "this is not an authority").unwrap();

        open(data_dir.path()).unwrap();

        assert_matches!(read(&data_dir.join(FILENAME)), Ok(Some(_)));
    }

    #[cfg(unix)]
    #[test]
    fn open_writes_authority_nobody_else_can_read() {
        use std::os::unix::fs::PermissionsExt as _;

        let data_dir = TempDataDir::new("cert-mode");

        open(data_dir.path()).unwrap();

        let mode = fs::metadata(data_dir.join(FILENAME))
            .unwrap()
            .permissions()
            .mode();

        assert_eq!(mode & 0o777, 0o600, "found {:o}", mode & 0o777);
    }
}

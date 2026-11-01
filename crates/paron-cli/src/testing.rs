use std::fmt::Debug;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Once;

pub const ROOT_FINGERPRINT: &str =
    "sha256:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

static PROCESS_SETUP: Once = Once::new();

pub struct TempDataDir(PathBuf);

impl TempDataDir {
    pub fn new(name: &str) -> Self {
        // `main` does this in a real run. Tests do not go through `main`.
        PROCESS_SETUP.call_once(|| {
            #[cfg(unix)]
            crate::platform::unix::files::restrict_new_files_to_owner();
            rustls::crypto::ring::default_provider()
                .install_default()
                .unwrap();
        });

        let path = std::env::temp_dir().join(format!("paron-cli-{}-{name}", std::process::id()));
        std::fs::create_dir(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));

        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0)
            .unwrap_or_else(|error| panic!("{}: {error}", self.0.display()));
    }
}

pub fn ip_address(text: &str) -> IpAddr {
    text.parse().unwrap()
}

pub fn parse_each<T>(texts: &[&str]) -> Vec<T>
where
    T: FromStr,
    T::Err: Debug,
{
    texts.iter().map(|text| text.parse().unwrap()).collect()
}

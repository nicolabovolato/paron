use std::path::Path;

use paron_tunnel_protocol::BrokerIdentity;
use secrecy::{ExposeSecret as _, SecretSlice};

use crate::Result;
use crate::platform::files;

pub const FILE: &str = "broker-ca.key";
pub const INDEX_FILE: &str = "broker-serving-index";

pub fn read(dir: &Path) -> Result<BrokerIdentity> {
    let path = dir.join(FILE);
    let stored = std::fs::read(&path)
        .map_err(|reason| format!("{} could not be read: {reason}", path.display()))?;

    Ok(BrokerIdentity::from_root_pkcs8_der(
        &SecretSlice::from(stored),
        advance(dir)?,
    )?)
}

pub fn mint(dir: &Path) -> Result<BrokerIdentity> {
    let root = BrokerIdentity::generate_root()?;
    let path = dir.join(FILE);
    std::fs::write(&path, root.expose_secret())
        .map_err(|reason| format!("{} could not be written: {reason}", path.display()))?;
    let index_path = dir.join(INDEX_FILE);
    files::atomic_replace(&index_path, b"0")
        .map_err(|reason| format!("{} could not be written: {reason}", index_path.display()))?;

    read(dir)
}

fn advance(dir: &Path) -> Result<u64> {
    let path = dir.join(INDEX_FILE);
    let held: u64 = std::fs::read_to_string(&path)
        .map_err(|reason| format!("{} could not be read: {reason}", path.display()))?
        .trim()
        .parse()
        .map_err(|_| format!("{} does not hold a serving index", path.display()))?;

    let next = held
        .checked_add(1)
        .ok_or_else(|| format!("{} is exhausted", path.display()))?;

    files::atomic_replace(&path, next.to_string().as_bytes())
        .map_err(|reason| format!("{} could not be written: {reason}", path.display()))?;

    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDataDir;

    #[test]
    fn read_after_mint_returns_the_minted_root() {
        let data_dir = TempDataDir::new("identity-stable");
        let identity_from_mint = mint(data_dir.path()).unwrap();

        let identity_from_read = read(data_dir.path()).unwrap();

        assert_eq!(
            identity_from_read.root_public_key(),
            identity_from_mint.root_public_key()
        );
    }

    #[test]
    fn read_after_mint_returns_a_new_serving_key() {
        let data_dir = TempDataDir::new("identity-serving");
        let identity_from_mint = mint(data_dir.path()).unwrap();

        let identity_from_read = read(data_dir.path()).unwrap();

        assert_ne!(
            identity_from_read.serving_public_key(),
            identity_from_mint.serving_public_key()
        );
    }

    #[test]
    fn advance_after_mint_and_read_returns_the_next_index() {
        let data_dir = TempDataDir::new("identity-index");
        mint(data_dir.path()).unwrap();

        let index = advance(data_dir.path()).unwrap();

        assert_eq!(index, 2);
    }

    #[test]
    fn read_without_an_index_file_returns_error() {
        let data_dir = TempDataDir::new("identity-index-lost");
        mint(data_dir.path()).unwrap();
        std::fs::remove_file(data_dir.join(INDEX_FILE)).unwrap();

        let Err(error) = read(data_dir.path()) else {
            panic!("the index file is missing");
        };

        assert!(error.to_string().contains(INDEX_FILE), "{error}");
    }
}

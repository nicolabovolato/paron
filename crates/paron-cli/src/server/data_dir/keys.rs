use std::path::Path;

use secrecy::{ExposeSecret as _, SecretBox};

use crate::Result;

pub const FILE: &str = "master.key";

pub struct MasterKey(SecretBox<[u8; 32]>);

impl MasterKey {
    #[expect(
        clippy::expect_used,
        reason = "a 32-byte key and a 32-byte output are inside the limits of HKDF-SHA256"
    )]
    pub fn derive(&self, label: &[u8]) -> SecretBox<[u8; 32]> {
        let mut derived = Box::new([0u8; 32]);

        hkdf::Hkdf::<sha2::Sha256>::from_prk(self.0.expose_secret())
            .expect("invalid master key")
            .expand(label, derived.as_mut_slice())
            .expect("could not derive key");

        SecretBox::new(derived)
    }
}

pub fn read(dir: &Path) -> Result<MasterKey> {
    let path = dir.join(FILE);
    let material = zeroize::Zeroizing::new(
        std::fs::read(&path)
            .map_err(|reason| format!("{} could not be read: {reason}", path.display()))?,
    );
    let material: &[u8; 32] = material
        .as_slice()
        .try_into()
        .map_err(|_| format!("{} is not a Paron key file", path.display()))?;

    Ok(MasterKey(SecretBox::init_with_mut(|key: &mut [u8; 32]| {
        key.copy_from_slice(material);
    })))
}

pub fn mint(dir: &Path) -> Result<MasterKey> {
    let key = SecretBox::init_with_mut(|key: &mut [u8; 32]| rand::fill(key));
    let path = dir.join(FILE);
    std::fs::write(&path, key.expose_secret())
        .map_err(|reason| format!("{} could not be written: {reason}", path.display()))?;

    read(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDataDir;

    #[test]
    fn read_after_mint_returns_the_minted_key() {
        let data_dir = TempDataDir::new("keys-stable");
        let key_from_mint = mint(data_dir.path()).unwrap();

        let key_from_read = read(data_dir.path()).unwrap();

        assert_eq!(
            key_from_read.derive(b"paron test v1").expose_secret(),
            key_from_mint.derive(b"paron test v1").expose_secret()
        );
    }

    #[test]
    fn read_file_of_wrong_length_returns_error() {
        let data_dir = TempDataDir::new("keys-short");
        std::fs::write(data_dir.join(FILE), [0u8; 64]).unwrap();

        let Err(error) = read(data_dir.path()) else {
            panic!("64 bytes is not a key file");
        };

        assert_eq!(
            error.to_string(),
            format!("{} is not a Paron key file", data_dir.join(FILE).display())
        );
    }
}

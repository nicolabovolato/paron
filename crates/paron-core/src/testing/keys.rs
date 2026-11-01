use secrecy::{SecretBox, SecretSlice};

use crate::domain::machine::{ssh, tunnel};

// `ssh-keygen -t ed25519` and `-t rsa -b 2048`, base64 body decoded to raw bytes.
pub const SSH_PUBLIC_KEY: &[u8] = include_bytes!("fixtures/ssh_ed25519_public.bin");
pub const SSH_PRIVATE_KEY: &[u8] = include_bytes!("fixtures/ssh_ed25519_private.bin");

pub const SSH_RSA_PRIVATE_KEY: &[u8] = include_bytes!("fixtures/ssh_rsa_private.bin");

/// Never added to any `authorized_keys`, so one server can host both the accepted and the rejected
/// case rather than needing a second container configured to refuse.
pub const SSH_UNAUTHORIZED_PRIVATE_KEY: &[u8] =
    include_bytes!("fixtures/ssh_ed25519_unauthorized_private.bin");

// `openssl genpkey -algorithm ed25519 | openssl pkey -pubout -outform DER`.
pub const TUNNEL_PUBLIC_KEY: &[u8] = include_bytes!("fixtures/tunnel_spki_ed25519.bin");
pub const OTHER_TUNNEL_PUBLIC_KEY: &[u8] = include_bytes!("fixtures/tunnel_spki_ed25519_other.bin");

pub fn ssh_public_key() -> ssh::PublicKey {
    ssh::PublicKey::parse(SSH_PUBLIC_KEY).unwrap()
}

pub fn ssh_private_key() -> ssh::PrivateKey {
    ssh::PrivateKey::parse(SecretSlice::from(SSH_PRIVATE_KEY.to_vec())).unwrap()
}

pub fn unauthorized_ssh_private_key() -> ssh::PrivateKey {
    ssh::PrivateKey::parse(SecretSlice::from(SSH_UNAUTHORIZED_PRIVATE_KEY.to_vec())).unwrap()
}

pub fn tunnel_public_key() -> tunnel::PublicKey {
    tunnel::PublicKey::parse(TUNNEL_PUBLIC_KEY).unwrap()
}

pub fn other_tunnel_public_key() -> tunnel::PublicKey {
    tunnel::PublicKey::parse(OTHER_TUNNEL_PUBLIC_KEY).unwrap()
}

pub fn symmetric_key(bytes: [u8; 32]) -> SecretBox<[u8; 32]> {
    SecretBox::new(Box::new(bytes))
}

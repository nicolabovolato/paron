use std::time::Duration;

use jiff::Timestamp;
use paron_tunnel_protocol::{BrokerAddress, Fingerprint};
use secrecy::SecretString;

use crate::application::machine::EnrollmentPolicy;

pub const TOKEN_MAC: [u8; 32] = [0xAB; 32];

pub fn enrollment_policy() -> EnrollmentPolicy {
    EnrollmentPolicy {
        invitation_lifetime: Duration::from_hours(12),
    }
}

/// With [`enrollment_policy`], an invitation issued at this instant expires at [`expires_at`].
pub fn issued_at() -> Timestamp {
    "2026-06-01T00:00:00Z".parse().unwrap()
}

pub fn expires_at() -> Timestamp {
    "2026-06-01T12:00:00Z".parse().unwrap()
}

pub fn after_expiry() -> Timestamp {
    "2026-06-02T00:00:00Z".parse().unwrap()
}

pub fn enrollment_token() -> SecretString {
    SecretString::from("a-token")
}

pub fn broker_address() -> BrokerAddress {
    "paron.example:8443".parse().unwrap()
}

pub fn broker_fingerprint() -> Fingerprint {
    Fingerprint::parse("sha256:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
        .unwrap()
}

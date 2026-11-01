use secrecy::{ExposeSecret as _, SecretString};

use crate::application::tokens::Tokens;
use crate::domain::token_mac::TokenMac;

pub struct DeviceToken {
    pub device_id: uuid::Uuid,
    mac: TokenMac,
}

impl DeviceToken {
    pub fn issue(tokens: &dyn Tokens, user_id: uuid::Uuid) -> Self {
        let device_id = uuid::Uuid::now_v7();

        Self {
            device_id,
            mac: tokens.mac(&Self::message(device_id, user_id)),
        }
    }

    pub fn verify(
        tokens: &dyn Tokens,
        encoded: &SecretString,
        user_id: uuid::Uuid,
    ) -> Option<Self> {
        let (device_id, mac) = encoded.expose_secret().split_once('.')?;
        let device_id = uuid::Uuid::try_parse(device_id).ok()?;
        let mac = TokenMac::from_hex(mac)?;

        (mac == tokens.mac(&Self::message(device_id, user_id))).then_some(Self { device_id, mac })
    }

    pub fn encode(&self) -> SecretString {
        SecretString::from(format!("{}.{}", self.device_id, self.mac.to_hex()))
    }

    fn message(device_id: uuid::Uuid, user_id: uuid::Uuid) -> SecretString {
        SecretString::from(format!("{device_id}.{user_id}"))
    }
}

use secrecy::SecretString;

use crate::domain::token_mac::TokenMac;

#[cfg_attr(test, mockall::automock)]
pub trait Tokens: Send + Sync {
    fn issue(&self) -> (SecretString, TokenMac);
    fn mac(&self, token: &SecretString) -> TokenMac;
}

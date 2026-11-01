pub mod passkey;
pub mod password;
pub mod ports;
pub mod recovery_code;
pub mod totp;

string_enum! {
    pub enum AuthenticationMethod {
        Password => "password",
        Totp => "totp",
        Passkey => "passkey",
        RecoveryCode => "recovery_code",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodRole {
    FirstFactor,
    SecondFactor,
    BothFactors,
}

impl AuthenticationMethod {
    pub fn role(self) -> MethodRole {
        match self {
            Self::Password => MethodRole::FirstFactor,
            Self::Totp | Self::RecoveryCode => MethodRole::SecondFactor,
            Self::Passkey => MethodRole::BothFactors,
        }
    }

    pub fn available_at(self, origin: &url::Origin) -> bool {
        match self {
            Self::Password | Self::Totp | Self::RecoveryCode => true,
            Self::Passkey => passkey::available_at(origin),
        }
    }
}

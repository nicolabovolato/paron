use derive_more::{Deref, From};

#[derive(Debug, Clone, Deref, From)]
#[deref(forward)]
pub struct TokenMac(Vec<u8>);

impl TokenMac {
    pub fn to_hex(&self) -> String {
        base16ct::lower::encode_string(&self.0)
    }

    pub fn from_hex(hex: &str) -> Option<Self> {
        base16ct::lower::decode_vec(hex).ok().map(Self)
    }
}

impl PartialEq for TokenMac {
    fn eq(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq as _;

        self.0.ct_eq(&other.0).into()
    }
}

impl Eq for TokenMac {}

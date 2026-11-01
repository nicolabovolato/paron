use ciborium::Value;

use super::cbor;

const USER_PRESENT: u8 = 0x01;
const USER_VERIFIED: u8 = 0x04;
const BACKUP_ELIGIBLE: u8 = 0x08;
const BACKUP_STATE: u8 = 0x10;
const ATTESTED_CREDENTIAL_DATA: u8 = 0x40;
const EXTENSION_DATA: u8 = 0x80;

const MAX_CREDENTIAL_ID_LENGTH: usize = 1023;
const MAX_LENGTH: usize = 16 * 1024;

/// The authenticator data of WebAuthn Level 3, §6.1.
#[derive(Debug, PartialEq, Eq)]
pub struct AuthenticatorData {
    pub rp_id_hash: [u8; 32],
    flags: u8,
    pub sign_count: u32,
    pub attested_credential: Option<AttestedCredential>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AttestedCredential {
    pub credential_id: Vec<u8>,
    /// The COSE key exactly as the authenticator encoded it.
    pub public_key: Vec<u8>,
}

impl AuthenticatorData {
    pub fn user_present(&self) -> bool {
        self.flags & USER_PRESENT != 0
    }

    pub fn user_verified(&self) -> bool {
        self.flags & USER_VERIFIED != 0
    }

    pub fn backup_eligible(&self) -> bool {
        self.flags & BACKUP_ELIGIBLE != 0
    }

    pub fn backup_state(&self) -> bool {
        self.flags & BACKUP_STATE != 0
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_LENGTH {
            return None;
        }

        let (rp_id_hash, rest) = bytes.split_first_chunk::<32>()?;
        let (&flags, rest) = rest.split_first()?;
        let (sign_count, mut rest) = rest.split_first_chunk::<4>()?;

        let attested_credential = if flags & ATTESTED_CREDENTIAL_DATA == 0 {
            None
        } else {
            let (_aaguid, after_aaguid) = rest.split_first_chunk::<16>()?;
            let (length, after_length) = after_aaguid.split_first_chunk::<2>()?;
            let length = usize::from(u16::from_be_bytes(*length));
            if length > MAX_CREDENTIAL_ID_LENGTH || after_length.len() < length {
                return None;
            }
            let (credential_id, after_id) = after_length.split_at(length);
            let (public_key, after_key) = split_cbor_item(after_id)?;
            rest = after_key;

            Some(AttestedCredential {
                credential_id: credential_id.to_vec(),
                public_key: public_key.to_vec(),
            })
        };

        if flags & EXTENSION_DATA != 0 {
            let (Value::Map(_), after_extensions) = cbor::read(rest)? else {
                return None;
            };
            rest = after_extensions;
        }

        if !rest.is_empty() {
            return None;
        }

        Some(Self {
            rp_id_hash: *rp_id_hash,
            flags,
            sign_count: u32::from_be_bytes(*sign_count),
            attested_credential,
        })
    }
}

/// The bytes of one canonical CBOR data item at the start of `bytes`, and what follows it.
fn split_cbor_item(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let (_, rest) = cbor::read(bytes)?;

    Some(bytes.split_at(bytes.len() - rest.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RP_ID_HASH: [u8; 32] = [7; 32];

    fn cbor(value: &Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(value, &mut bytes).unwrap();

        bytes
    }

    fn header(flags: u8) -> Vec<u8> {
        [&RP_ID_HASH[..], &[flags], &[0; 4]].concat()
    }

    #[test]
    fn parse_assertion_data_returns_the_flags() {
        let bytes = header(USER_PRESENT | USER_VERIFIED | BACKUP_ELIGIBLE);

        let parsed = AuthenticatorData::parse(&bytes);

        let parsed = parsed.unwrap();
        assert!(parsed.user_present());
        assert!(parsed.user_verified());
        assert!(parsed.backup_eligible());
        assert!(!parsed.backup_state());
        assert_eq!(parsed.attested_credential, None);
    }

    #[test]
    fn parse_attested_data_returns_the_credential_and_its_key() {
        let key = cbor(&Value::Map(vec![(Value::from(1), Value::from(2))]));
        let extensions = cbor(&Value::Map(vec![(
            Value::Text("credProtect".to_owned()),
            Value::from(3),
        )]));
        let bytes = [
            header(USER_PRESENT | ATTESTED_CREDENTIAL_DATA | EXTENSION_DATA),
            vec![0; 16],
            vec![0, 3],
            vec![9, 9, 9],
            key.clone(),
            extensions,
        ]
        .concat();

        let parsed = AuthenticatorData::parse(&bytes).unwrap();

        assert_eq!(
            parsed.attested_credential,
            Some(AttestedCredential {
                credential_id: vec![9, 9, 9],
                public_key: key,
            })
        );
    }

    #[test]
    fn parse_data_with_trailing_bytes_or_short_credential_id_returns_not_found() {
        let trailing = [header(USER_PRESENT), vec![0]].concat();
        let short_id = [
            header(USER_PRESENT | ATTESTED_CREDENTIAL_DATA),
            vec![0; 16],
            vec![0, 9],
            vec![1, 2],
        ]
        .concat();

        assert_eq!(AuthenticatorData::parse(&trailing), None);
        assert_eq!(AuthenticatorData::parse(&short_id), None);
        assert_eq!(AuthenticatorData::parse(&RP_ID_HASH), None);
    }

    #[test]
    fn parse_data_longer_than_the_limit_returns_not_found() {
        let extensions = cbor(&Value::Map(vec![(
            Value::Text("large".to_owned()),
            Value::Bytes(vec![0; MAX_LENGTH]),
        )]));
        let bytes = [header(USER_PRESENT | EXTENSION_DATA), extensions].concat();

        let parsed = AuthenticatorData::parse(&bytes);

        assert_eq!(parsed, None);
    }
}

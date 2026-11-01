use std::cmp::Ordering;

use ciborium::Value;

/// One CBOR data item at the start of `bytes`, and what follows it. WebAuthn §2.4: a decoder should
/// refuse CBOR that is not in the CTAP2 canonical form, and maps with duplicate keys.
pub fn read(bytes: &[u8]) -> Option<(Value, &[u8])> {
    let mut rest = bytes;
    let value = ciborium::de::from_reader::<Value, _>(&mut rest).ok()?;
    let item = &bytes[..bytes.len() - rest.len()];

    // The canonical form has the shortest heads and only definite lengths, so it is the encoding
    // that ciborium writes again.
    let mut written = Vec::new();
    ciborium::ser::into_writer(&value, &mut written).ok()?;
    (written == item && keys_in_order(&value)).then_some((value, rest))
}

/// `bytes` hold one CBOR data item and nothing after it.
pub fn read_all(bytes: &[u8]) -> Option<Value> {
    let (value, rest) = read(bytes)?;

    rest.is_empty().then_some(value)
}

/// CTAP 2.1 §8: the keys of a map grow by major type, then by length, then byte by byte. A
/// duplicate key is not greater than the key before it.
fn keys_in_order(value: &Value) -> bool {
    match value {
        Value::Map(entries) => {
            // The encoded keys are dropped before the recursion. Kept, each level of a key nested
            // in a key holds a copy of the input.
            let keys_ascend = entries
                .iter()
                .map(|(key, _)| {
                    let mut encoded = Vec::new();
                    ciborium::ser::into_writer(key, &mut encoded).ok()?;
                    Some(encoded)
                })
                .collect::<Option<Vec<_>>>()
                .is_some_and(|keys| {
                    keys.windows(2)
                        .all(|pair| canonical_order(&pair[0], &pair[1]) == Ordering::Less)
                });

            keys_ascend
                && entries
                    .iter()
                    .all(|(key, value)| keys_in_order(key) && keys_in_order(value))
        }
        Value::Array(items) => items.iter().all(keys_in_order),
        Value::Tag(_, inner) => keys_in_order(inner),
        _ => true,
    }
}

fn canonical_order(first: &[u8], second: &[u8]) -> Ordering {
    let major_type = |key: &[u8]| key.first().map(|head| head >> 5);

    major_type(first)
        .cmp(&major_type(second))
        .then(first.len().cmp(&second.len()))
        .then(first.cmp(second))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_canonical_item_returns_it_and_the_bytes_after_it() {
        // {1: 2, -1: h'01'} followed by one more byte.
        let bytes = [0xa2, 0x01, 0x02, 0x20, 0x41, 0x01, 0xff];

        let read = read(&bytes);

        assert_eq!(
            read,
            Some((
                Value::Map(vec![
                    (Value::from(1), Value::from(2)),
                    (Value::from(-1), Value::Bytes(vec![1])),
                ]),
                &[0xff][..],
            ))
        );
    }

    #[test]
    fn read_item_that_is_not_canonical_returns_not_found() {
        let keys_out_of_order = [0xa2, 0x20, 0x00, 0x01, 0x00];
        let duplicate_keys = [0xa2, 0x01, 0x00, 0x01, 0x00];
        let long_head = [0x18, 0x01];
        let indefinite_length = [0x5f, 0x41, 0x01, 0xff];

        assert_eq!(read(&keys_out_of_order), None);
        assert_eq!(read(&duplicate_keys), None);
        assert_eq!(read(&long_head), None);
        assert_eq!(read(&indefinite_length), None);
    }
}

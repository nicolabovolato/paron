#[derive(Debug, thiserror::Error)]
#[error("expected between {min} and {max} characters")]
pub struct InvalidText {
    min: usize,
    max: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, derive_more::Deref)]
#[deref(forward)]
pub struct LimitedText<const MIN: usize, const MAX: usize>(String);

impl<const MIN: usize, const MAX: usize> LimitedText<MIN, MAX> {
    pub const MIN: usize = MIN;
    pub const MAX: usize = MAX;

    pub fn parse(raw: impl Into<String>) -> Result<Self, InvalidText> {
        let raw = raw.into();

        if !(MIN..=MAX).contains(&raw.chars().count()) {
            return Err(InvalidText { min: MIN, max: MAX });
        }

        Ok(Self(raw))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    type Name = LimitedText<1, 4>;

    #[test]
    fn parse_within_the_limits_keeps_the_text_as_typed() {
        for valid_text in ["a", " ab ", "àéîõ"] {
            assert_eq!(&*Name::parse(valid_text).unwrap(), valid_text);
        }
    }

    #[test]
    fn parse_of_wrong_length_returns_error() {
        for invalid_text in ["", "abcde", "àéîõü"] {
            assert_matches!(
                Name::parse(invalid_text),
                Err(InvalidText { min: 1, max: 4 }),
                "{invalid_text:?}"
            );
        }
    }
}

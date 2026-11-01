/// Declares an enum whose variants have a stable external spelling, and derives the parsing,
/// the rendering and the exhaustive list from the declaration.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident { $($variant:ident => $wire:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis enum $name {
            $($variant),+
        }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = $crate::domain::UnknownStringEnumValue;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                match raw {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err($crate::domain::UnknownStringEnumValue {
                        kind: stringify!($name),
                        value: raw.to_owned(),
                    }),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

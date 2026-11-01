//! Reading a request body without giving up on the first bad field.
//!
//! A boundary type holds a `Result` instead of returning one, so `Deserialize` always succeeds and
//! serde reads the whole body. Failures are collected afterwards by walking the request type,
//! which is also where the field paths come from.
//!
//! Building the paths ourselves is what makes this work through tagged enums: a path like
//! `protocol.authentication.private_key` comes from the walk, and serde is never asked for it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

// The generated code names `::serde_utoipa_validate`, which has to resolve inside this crate too.
extern crate self as serde_utoipa_validate;

pub use serde_utoipa_validate_macros::validate;

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::{Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

/// Public because macro expansions in other crates name it; not API.
#[doc(hidden)]
pub const REQUIRED: &str = "this field is required";

/// Everything wrong with a body, keyed by the path of the field it was wrong at.
///
/// Paths are dotted for nesting and bracketed for lists — `protocol.authentication.user`,
/// `roles[1]` — so a form can match one against the input that produced it.
#[derive(Debug, Default)]
pub struct ValidationErrors {
    fields: BTreeMap<String, Vec<String>>,
}

impl ValidationErrors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub fn into_fields(self) -> BTreeMap<String, Vec<String>> {
        self.fields
    }

    pub fn field<W: Wire>(&mut self, name: &str, value: W) -> Option<W::Parsed> {
        let mut nested = Self::new();
        let parsed = value.parse(&mut nested);

        self.absorb(name, nested);

        parsed
    }

    fn absorb(&mut self, name: &str, nested: Self) {
        for (path, messages) in nested.fields {
            self.fields
                .entry(join(name, &path))
                .or_default()
                .extend(messages);
        }
    }

    pub fn required<W: Wire>(&mut self, name: &str, patch: Patch<W>) -> Option<W::Parsed> {
        match patch {
            Patch::Absent => self.reject_field(name, REQUIRED),
            Patch::Present(value) => self.field(name, value),
        }
    }

    /// A field marked `#[validate(default)]`, whose type says what an absent one becomes. The
    /// document calls it optional, and the caller never writes the default at the use site.
    pub fn defaulted<W: Wire + Absent>(
        &mut self,
        name: &str,
        patch: Patch<W>,
    ) -> Option<W::Parsed> {
        match patch {
            Patch::Absent => Some(W::absent()),
            Patch::Present(value) => self.field(name, value),
        }
    }

    /// A field marked `#[validate(default = value)]`: an absent one is this value, for this field
    /// only.
    pub fn defaulted_to<W: Wire>(
        &mut self,
        name: &str,
        patch: Patch<W>,
        default: impl FnOnce() -> W::Parsed,
    ) -> Option<W::Parsed> {
        match patch {
            Patch::Absent => Some(default()),
            Patch::Present(value) => self.field(name, value),
        }
    }

    /// A field written `Option<T>`, which keeps serde's meaning: absent and `null` both say
    /// nothing about the field.
    pub fn optional<W: Wire>(
        &mut self,
        name: &str,
        patch: Patch<Option<W>>,
    ) -> Option<Option<W::Parsed>> {
        match patch {
            Patch::Absent => Some(None),
            Patch::Present(value) => self.field(name, value),
        }
    }

    /// A field written `Patch<T>`. The outer `Option` says whether it passed, the inner whether it
    /// was sent; whether `null` is allowed is `T`'s business, not this method's.
    #[allow(clippy::option_option)]
    pub fn patch<W: Wire>(&mut self, name: &str, patch: Patch<W>) -> Option<Option<W::Parsed>> {
        match patch {
            Patch::Absent => Some(None),
            Patch::Present(value) => self.field(name, value).map(Some),
        }
    }

    /// Files a message against a named field. Returns `None` so it can be the whole of a judging
    /// method's failing arm.
    fn reject_field<T>(&mut self, name: &str, message: &str) -> Option<T> {
        self.fields
            .entry(name.to_owned())
            .or_default()
            .push(message.to_owned());

        None
    }

    /// Files a message against the value being judged rather than a field inside it — the other
    /// half of [`reject_field`](Self::reject_field). `field` joins the empty path to the name of
    /// whatever holds this value.
    #[doc(hidden)]
    pub fn reject(&mut self, message: String) {
        self.fields.entry(String::new()).or_default().push(message);
    }

    /// The parsed value, or everything that stopped it being one.
    ///
    /// `build` only runs when nothing was rejected, and returns `None` if a field it needs is
    /// missing. Both are checked: the compiler cannot see that one implies the other.
    pub fn build<T>(self, build: impl FnOnce() -> Option<T>) -> Result<T, Self> {
        if !self.is_empty() {
            return Err(self);
        }

        build().ok_or(self)
    }
}

/// `roles` + `[1]` is `roles[1]`; `protocol` + `authentication.user` is dotted. An empty tail is
/// the field itself rejecting, rather than something inside it.
fn join(name: &str, tail: &str) -> String {
    if tail.is_empty() {
        name.to_owned()
    } else if tail.starts_with('[') {
        format!("{name}{tail}")
    } else {
        format!("{name}.{tail}")
    }
}

/// Whether a field was sent at all.
///
/// `Option` cannot answer this: serde reads an absent field and an explicit `null` as the same
/// `None`. This reads only the presence of the key and leaves the rest to `W`, so nullability
/// composes rather than being a third state baked in here.
///
/// `Patch<Username>` may be omitted but not blanked — `null` fails `Username`'s own reading like
/// any other wrong type. `Patch<Option<Email>>` may be blanked, and `null` is a value it carries.
/// That is the merge patch of RFC 7396, spelled with the types that already exist.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Patch<W> {
    #[default]
    Absent,
    Present(W),
}

impl<'de, W: Deserialize<'de>> Deserialize<'de> for Patch<W> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // serde only calls this when the key is present, so absence is `Default` and never
        // reaches here.
        Ok(Self::Present(W::deserialize(deserializer)?))
    }
}

/// What a type becomes when the field holding it was not sent.
///
/// Deliberately without a blanket impl: a type that has no sensible default must not silently
/// acquire one, and a field marked `#[validate(default)]` whose type does not implement this is a
/// compile error rather than a document that lies about what may be omitted.
pub trait Absent: Wire {
    fn absent() -> Self::Parsed;
}

/// A type as it arrives, and what it becomes once judged.
pub trait Wire: Sized {
    type Parsed;

    /// Judges the value, filing anything wrong into `errors` under the paths it owns.
    ///
    /// `None` means it did not pass. Every field is judged before any is given up on, which is
    /// what lets one body report everything wrong with it at once.
    fn parse(self, errors: &mut ValidationErrors) -> Option<Self::Parsed>;
}

impl<W: Wire> Wire for Vec<W> {
    type Parsed = Vec<W::Parsed>;

    fn parse(self, errors: &mut ValidationErrors) -> Option<Self::Parsed> {
        let mut parsed = Vec::with_capacity(self.len());
        let mut rejected = false;

        // Every element is judged, not just up to the first bad one: a list of five roles with
        // two typos should name both. The index is only formatted for an element that failed —
        // going through `field` instead would allocate one per element, which measured a third of
        // the cost of reading a ten-element list.
        for (index, item) in self.into_iter().enumerate() {
            let mut nested = ValidationErrors::new();

            match item.parse(&mut nested) {
                Some(item) if nested.is_empty() => parsed.push(item),
                _ => {
                    rejected = true;
                    errors.absorb(&format!("[{index}]"), nested);
                }
            }
        }

        (!rejected).then_some(parsed)
    }
}

/// Absent is accepted; present but rejected is not, so a bad value is never mistaken for one the
/// caller chose not to send.
impl<W: Wire> Wire for Option<W> {
    type Parsed = Option<W::Parsed>;

    fn parse(self, errors: &mut ValidationErrors) -> Option<Self::Parsed> {
        match self {
            None => Some(None),
            Some(value) => value.parse(errors).map(Some),
        }
    }
}

/// A JSON scalar as it arrived, before anything decides whether it is the one that was wanted.
///
/// `deserialize_any` accepts whatever is there, so a value of the wrong type becomes a rejection
/// the walk can report rather than an error that ends the body. JSON is self-describing, which is
/// what makes that viable; a binary format would not be.
enum Scalar {
    Text(String),
    Signed(i64),
    Unsigned(u64),
    /// A bool, a null, a list or an object — nothing any reader here accepts.
    Other,
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ScalarVisitor)
    }
}

struct ScalarVisitor;

impl<'de> Visitor<'de> for ScalarVisitor {
    type Value = Scalar;

    // Never surfaces: every arm below succeeds, so serde has no error to word.
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_str<E>(self, raw: &str) -> Result<Scalar, E> {
        Ok(Scalar::Text(raw.to_owned()))
    }

    fn visit_i64<E>(self, raw: i64) -> Result<Scalar, E> {
        Ok(Scalar::Signed(raw))
    }

    fn visit_u64<E>(self, raw: u64) -> Result<Scalar, E> {
        Ok(Scalar::Unsigned(raw))
    }

    fn visit_f64<E>(self, _: f64) -> Result<Scalar, E> {
        Ok(Scalar::Other)
    }

    fn visit_bool<E>(self, _: bool) -> Result<Scalar, E> {
        Ok(Scalar::Other)
    }

    fn visit_unit<E>(self) -> Result<Scalar, E> {
        Ok(Scalar::Other)
    }

    fn visit_none<E>(self) -> Result<Scalar, E> {
        Ok(Scalar::Other)
    }

    // Drained rather than skipped: leaving a value half-read resumes the parser mid-value and
    // everything after it is garbage.
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Scalar, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}

        Ok(Scalar::Other)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Scalar, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}

        Ok(Scalar::Other)
    }
}

/// Declares a reader: how a scalar becomes the primitive it wanted, and what to say when it is not
/// that scalar.
macro_rules! raw {
    ($name:ident, $parsed:ty, $schema:expr, $read:expr) => {
        newtype!(@type
            /// A field written as a plain `String`, `u64`, `Uuid` or `Timestamp` is read through
            /// this, so a wrong value is a rejection rather than the end of the body. `#[validate]`
            /// puts it there.
            pub $name($parsed) { $schema }
        );
        newtype!(@carry $name($parsed));

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[allow(clippy::redundant_closure_call)]
                Ok(Self(($read)(Scalar::deserialize(deserializer)?)))
            }
        }

        impl $name {
            #[doc(hidden)]
            pub fn into_inner(self) -> Result<$parsed, String> {
                self.0
            }
        }
    };
}

raw!(
    RawString,
    String,
    utoipa::openapi::ObjectBuilder::new().schema_type(utoipa::openapi::schema::Type::String),
    |scalar| match scalar {
        Scalar::Text(text) => Ok(text),
        _ => Err("expected a string".to_owned()),
    }
);

raw!(
    RawNumber,
    u64,
    utoipa::openapi::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::Integer)
        .format(Some(utoipa::openapi::SchemaFormat::KnownFormat(
            utoipa::openapi::KnownFormat::Int64,
        ))),
    |scalar| match scalar {
        Scalar::Unsigned(number) => Ok(number),
        Scalar::Signed(number) =>
            u64::try_from(number).map_err(|_| "expected a positive whole number".to_owned()),
        // A query string carries every value as text, so the digits arrive spelled out. Reading
        // them here rather than in a second reader is what keeps a declaration from having to
        // know whether it will be used in a body or in a query.
        Scalar::Text(text) => text
            .trim()
            .parse::<u64>()
            .map_err(|_| "expected a whole number".to_owned()),
        Scalar::Other => Err("expected a whole number".to_owned()),
    }
);

#[cfg(feature = "uuid")]
raw!(
    RawUuid,
    uuid::Uuid,
    utoipa::openapi::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::String)
        .format(Some(utoipa::openapi::SchemaFormat::KnownFormat(
            utoipa::openapi::KnownFormat::Uuid,
        ))),
    |scalar| match scalar {
        Scalar::Text(text) => text.parse().map_err(|_| "expected a UUID".to_owned()),
        _ => Err("expected a UUID".to_owned()),
    }
);

#[cfg(feature = "jiff")]
raw!(
    RawTimestamp,
    jiff::Timestamp,
    utoipa::openapi::ObjectBuilder::new()
        .schema_type(utoipa::openapi::schema::Type::String)
        .format(Some(utoipa::openapi::SchemaFormat::KnownFormat(
            utoipa::openapi::KnownFormat::DateTime,
        ))),
    |scalar| match scalar {
        Scalar::Text(text) => text
            .parse()
            .map_err(|_| "expected an RFC 3339 date and time".to_owned()),
        _ => Err("expected an RFC 3339 date and time".to_owned()),
    }
);

/// Declares a type the API speaks in: what it looks like in JSON, what it becomes, and how it is
/// described in the document.
///
/// The generated type carries its failure instead of returning it, so one bad field does not end
/// the body. It is also the field's real type, which is what lets utoipa register it as a named
/// component without anyone listing it.
///
/// A declaration may state a `default:`, which is what an absent field holding it becomes. It
/// emits [`Absent`](Absent) and puts the same value in the schema, so the document and the server
/// answer an omitted field from one expression. A field only takes it by asking, with
/// `#[validate(default)]`.
///
/// Reads as a `String`, or as an integer of any unsigned width — `u64` as it arrives, anything
/// narrower checked against that width first. A number is read whether the format spells it as a
/// number or as text, so a declaration never has to know whether it will be read from a body or
/// from a query string. It writes as whatever `serialize` returns — a `Result`, so a
/// value that cannot be written fails the response instead of panicking inside it. Writing
/// `deserialize` makes it readable in a request, writing `serialize` makes it writable in a
/// response, and writing both makes it one component shared by the two. Omitting a direction is
/// omitting a function.
///
/// Every declaration is a named component. For one whose name would tell a reader nothing — a
/// plain string, a count — put `#[schema(inline)]` on the field instead, so the document states
/// the shape rather than a reference to it.
#[macro_export]
macro_rules! newtype {
    // An enum whose words the domain does not spell itself: stated once here, with a `serialize`
    // match exhaustive over the domain enum, so a variant added there is a compile error rather
    // than a schema that quietly omits it. Tried first, because the general arm below takes any
    // body at all and would swallow a word list. The default comes first, so that it cannot be
    // read as one more word.
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident: String -> $domain:path {
            default: $default:ident,
            $($variant:ident => $word:literal),+ $(,)?
        }
    ) => {
        $crate::newtype!(@words [$default] $(#[$meta])* $vis $name: String -> $domain {
            $($variant => $word),+
        });
    };

    (
        $(#[$meta:meta])*
        $vis:vis $name:ident: String -> $domain:path {
            $($variant:ident => $word:literal),+ $(,)?
        }
    ) => {
        $crate::newtype!(@words [] $(#[$meta])* $vis $name: String -> $domain {
            $($variant => $word),+
        });
    };

    (@words [$($default:ident)?] $(#[$meta:meta])* $vis:vis $name:ident: String -> $domain:path {
        $($variant:ident => $word:literal),+
    }) => {
        $crate::newtype! {
            $(#[$meta])*
            $vis $name: String -> $domain {
                schema: ::utoipa::openapi::ObjectBuilder::new()
                    .schema_type(::utoipa::openapi::schema::Type::String)
                    .enum_values(Some([$($word),+]))
                    $(.default(Some($name::word(&{
                        use $domain as Domain;

                        Domain::$default
                    }).into())))?,
                deserialize: |raw: String| {
                    use $domain as Domain;

                    match raw.as_str() {
                        $($word => Ok(Domain::$variant),)+
                        other => Err(format!(
                            "{other:?} is not a recognised {}",
                            stringify!($name)
                        )),
                    }
                },
                serialize: |value: &$domain| Ok($name::word(value)),
            }
        }

        impl $name {
            fn word(value: &$domain) -> &'static str {
                use $domain as Domain;

                match value {
                    $(Domain::$variant => $word),+
                }
            }
        }

        $(
            impl $crate::Absent for $name {
                fn absent() -> $domain {
                    use $domain as Domain;

                    Domain::$default
                }
            }
        )?
    };

    // The reader is an implementation detail; a caller names the JSON type it reads as.
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident: String -> $parsed:ty { $($body:tt)* }
    ) => {
        $crate::newtype!(@read_as $crate::RawString; $(#[$meta])* $vis $name -> $parsed { $($body)* });
    };

    (
        $(#[$meta:meta])*
        $vis:vis $name:ident: u64 -> $parsed:ty { $($body:tt)* }
    ) => {
        $crate::newtype!(@read_as $crate::RawNumber; $(#[$meta])* $vis $name -> $parsed { $($body)* });
    };

    // Any narrower integer. Matched after `String` and `u64` because those spell their own arms
    // and an `ident` would swallow them. The macro narrows before the body runs, so the body is
    // handed the integer it asked for.
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident: $raw:ident -> $parsed:ty { $($body:tt)* }
    ) => {
        $crate::newtype!(@read_narrowed $raw; $(#[$meta])* $vis $name -> $parsed { $($body)* });
    };

    // The narrowed integer is the value, so there is nothing left to judge.
    (
        $(#[$meta:meta])*
        $vis:vis $name:ident: $raw:ident {
            schema: $schema:expr,
            $(default: $default:expr,)?
        }
    ) => {
        $crate::newtype!(@read_narrowed $raw; $(#[$meta])* $vis $name -> $raw {
            schema: $schema,
            $(default: $default,)?
            deserialize: |raw: $raw| ::std::result::Result::<$raw, ::std::string::String>::Ok(raw),
        });
    };

    (@read_narrowed $raw:ident; $(#[$meta:meta])* $vis:vis $name:ident -> $parsed:ty {
        schema: $schema:expr,
        $(default: $default:expr,)?
        deserialize: $deserialize:expr $(,)?
    }) => {
        $crate::newtype!(@type $(#[$meta])* $vis $name($parsed) {
            {
                let schema = $schema;
                $( let schema = schema.default(Some(($default).into())); )?
                schema
            }
        });
        $crate::newtype!(@carry $name($parsed));

        $(
            impl $crate::Absent for $name {
                fn absent() -> $parsed {
                    $default
                }
            }
        )?

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let raw = <$crate::RawNumber as ::serde::Deserialize>::deserialize(deserializer)?;

                Ok(Self(raw.into_inner().and_then(|number| {
                    let narrowed = <$raw>::try_from(number).map_err(|_| {
                        ::std::format!("expected an integer no larger than {}", <$raw>::MAX)
                    })?;

                    #[allow(clippy::redundant_closure_call)]
                    ($deserialize)(narrowed)
                        .map_err(|error| ::std::string::ToString::to_string(&error))
                })))
            }
        }
    };

    (@read_as $reader:ty; $(#[$meta:meta])* $vis:vis $name:ident -> $parsed:ty {
        schema: $schema:expr,
        deserialize: $deserialize:expr,
        serialize: $serialize:expr $(,)?
    }) => {
        $crate::newtype!(@type $(#[$meta])* $vis $name($parsed) { $schema });
        $crate::newtype!(@read $reader; $name -> $parsed { $deserialize });
        $crate::newtype!(@write $name($parsed) { $serialize });
    };

    // Read only. A response cannot write one of these, because there is no `Serialize`.
    (@read_as $reader:ty; $(#[$meta:meta])* $vis:vis $name:ident -> $parsed:ty {
        schema: $schema:expr,
        deserialize: $deserialize:expr $(,)?
    }) => {
        $crate::newtype!(@type $(#[$meta])* $vis $name($parsed) { $schema });
        $crate::newtype!(@read $reader; $name -> $parsed { $deserialize });
    };

    // Written only. A request cannot carry one of these, because there is no `Deserialize`.
    (@read_as $reader:ty; $(#[$meta:meta])* $vis:vis $name:ident -> $parsed:ty {
        schema: $schema:expr,
        serialize: $serialize:expr $(,)?
    }) => {
        $crate::newtype!(@type $(#[$meta])* $vis $name($parsed) { $schema });
        $crate::newtype!(@write $name($parsed) { $serialize });
    };

    (@type $(#[$meta:meta])* $vis:vis $name:ident($parsed:ty) { $schema:expr }) => {
        $(#[$meta])*
        $vis struct $name(Result<$parsed, String>);

        /// Opaque on purpose: a request struct derives `Debug`, and some of these hold secrets.
        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                match &self.0 {
                    Ok(_) => write!(f, "{}(accepted)", stringify!($name)),
                    Err(message) => write!(f, "{}(rejected: {message})", stringify!($name)),
                }
            }
        }

        impl ::utoipa::PartialSchema for $name {
            fn schema() -> ::utoipa::openapi::RefOr<::utoipa::openapi::schema::Schema> {
                $schema.into()
            }
        }

        impl ::utoipa::ToSchema for $name {}
    };

    (@read $reader:ty; $name:ident -> $parsed:ty { $deserialize:expr }) => {
        $crate::newtype!(@carry $name($parsed));

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> Result<Self, D::Error> {
                let raw = <$reader as ::serde::Deserialize>::deserialize(deserializer)?;

                Ok(Self(raw.into_inner().and_then(|raw| {
                    #[allow(clippy::redundant_closure_call)]
                    ($deserialize)(raw)
                        .map_err(|error| ::std::string::ToString::to_string(&error))
                })))
            }
        }

    };

    // What a carrier shares whether it is declared or a reader: absent is a failure it holds, and
    // judging it hands that failure to the walk.
    (@carry $name:ident($parsed:ty)) => {
        impl Default for $name {
            fn default() -> Self {
                Self(Err($crate::REQUIRED.to_owned()))
            }
        }

        impl $crate::Wire for $name {
            type Parsed = $parsed;

            fn parse(self, errors: &mut $crate::ValidationErrors) -> Option<$parsed> {
                match self.0 {
                    Ok(parsed) => Some(parsed),
                    Err(message) => {
                        errors.reject(message);

                        None
                    }
                }
            }
        }
    };

    (@write $name:ident($parsed:ty) { $serialize:expr }) => {
        impl From<$parsed> for $name {
            fn from(value: $parsed) -> Self {
                Self(Ok(value))
            }
        }

        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match &self.0 {
                    Ok(value) => {
                        #[allow(clippy::redundant_closure_call)]
                        let written: ::std::result::Result<_, ::std::string::String> =
                            ($serialize)(value);

                        match written {
                            // Whatever the hook returns decides the shape on the wire, so a
                            // number goes out as a number.
                            Ok(written) => ::serde::Serialize::serialize(&written, serializer),
                            // A value that cannot be written is a failed response rather than a
                            // panic: panicking here kills the connection and answers nothing.
                            Err(cause) => Err(::serde::ser::Error::custom(cause)),
                        }
                    }
                    // A response is built from a domain value, so this is unreachable. It answers
                    // 500 rather than panicking if that ever stops being true.
                    Err(_) => Err(::serde::ser::Error::custom(
                        "a rejected value cannot be written to a response",
                    )),
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use utoipa::IntoParams;

    use super::*;

    newtype! {
        pub Word: String -> String {
            schema: utoipa::openapi::ObjectBuilder::new()
                .schema_type(utoipa::openapi::schema::Type::String),
            deserialize: |raw: String| {
                if raw == "ok" { Ok(raw) } else { Err("expected ok") }
            },
            serialize: |word: &String| Ok(word.clone()),
        }
    }

    newtype! {
        pub Count: u64 -> u64 {
            schema: utoipa::openapi::ObjectBuilder::new()
                .schema_type(utoipa::openapi::schema::Type::Integer),
            deserialize: |raw: u64| Ok::<_, &str>(raw),
            serialize: |count: &u64| Ok(*count),
        }
    }

    newtype! {
        pub EvenU8: u8 -> u8 {
            schema: utoipa::openapi::ObjectBuilder::new()
                .schema_type(utoipa::openapi::schema::Type::Integer),
            default: 8,
            deserialize: |raw: u8| {
                if raw.is_multiple_of(2) { Ok(raw) } else { Err("expected an even number") }
            },
        }
    }

    newtype! {
        pub PlainU16: u16 {
            schema: utoipa::openapi::ObjectBuilder::new()
                .schema_type(utoipa::openapi::schema::Type::Integer),
        }
    }

    newtype! {
        pub UnwritableText: String -> String {
            schema: utoipa::openapi::ObjectBuilder::new()
                .schema_type(utoipa::openapi::schema::Type::String),
            serialize: |_: &String| Err::<String, _>("this value cannot be written".to_owned()),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Direction {
        Up,
        Down,
    }

    newtype! {
        pub Heading: String -> Direction {
            default: Down,
            Up => "up",
            Down => "down",
        }
    }

    struct HostAndPort(Word, Word);

    impl Wire for HostAndPort {
        type Parsed = ();

        fn parse(self, errors: &mut ValidationErrors) -> Option<()> {
            errors.field("host", self.0);
            errors.field("port", self.1);

            None
        }
    }

    #[derive(Deserialize)]
    struct WordPatch {
        #[serde(default)]
        word: Patch<Word>,
    }

    #[derive(Deserialize)]
    struct ClearableWordPatch {
        #[serde(default)]
        word: Patch<Option<Word>>,
    }

    #[validate]
    #[derive(Debug, Deserialize, utoipa::ToSchema)]
    struct WordUpdate {
        leave: Option<Word>,
        clear: Patch<Option<Word>>,
    }

    #[validate]
    #[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
    #[into_params(parameter_in = Query)]
    struct PageQuery {
        size: Option<EvenU8>,
        #[validate(default)]
        page: EvenU8,
        name: String,
    }

    #[validate]
    #[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
    #[into_params(parameter_in = Query)]
    struct DefaultsQuery {
        #[validate(default)]
        heading: Heading,
        #[validate(default = "ok".to_owned())]
        word: Word,
    }

    #[validate]
    #[derive(Debug, Deserialize, utoipa::ToSchema)]
    struct RenamedField {
        #[serde(rename = "type")]
        word_type: Word,
    }

    #[validate]
    #[derive(Deserialize, utoipa::ToSchema)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum EnumWithSkippedVariants {
        #[serde(skip)]
        Bytes(Vec<u8>),
        #[serde(skip)]
        End,
        Count {
            count: EvenU8,
        },
    }

    fn parse_or_field_errors<W: Wire>(
        wire_value: W,
    ) -> Result<W::Parsed, BTreeMap<String, Vec<String>>> {
        let mut errors = ValidationErrors::new();
        let parsed = wire_value.parse(&mut errors);

        errors
            .build(|| parsed)
            .map_err(ValidationErrors::into_fields)
    }

    fn from_json<T: for<'de> Deserialize<'de>>(json_value: serde_json::Value) -> T {
        serde_json::from_value(json_value).unwrap()
    }

    fn field_error(path: &str, message: &str) -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([(path.to_owned(), vec![message.to_owned()])])
    }

    fn query_parameters<Query: IntoParams>() -> serde_json::Value {
        serde_json::to_value(Query::into_params(|| {
            Some(utoipa::openapi::path::ParameterIn::Query)
        }))
        .unwrap()
    }

    #[test]
    fn parse_list_with_bad_elements_files_error_for_each_index() {
        let words: Vec<Word> = from_json(json!(["ok", "no", "ok", "no"]));

        let parsed = parse_or_field_errors(words);

        assert_eq!(
            parsed,
            Err(BTreeMap::from([
                ("[1]".to_owned(), vec!["expected ok".to_owned()]),
                ("[3]".to_owned(), vec!["expected ok".to_owned()]),
            ]))
        );
    }

    #[test]
    fn parse_list_of_valid_elements_returns_every_value() {
        let words: Vec<Word> = from_json(json!(["ok", "ok"]));

        let parsed = parse_or_field_errors(words);

        assert_eq!(parsed, Ok(vec!["ok".to_owned(), "ok".to_owned()]));
    }

    #[test]
    fn parse_option_none_returns_none() {
        let parsed = parse_or_field_errors(None::<Word>);

        assert_eq!(parsed, Ok(None));
    }

    #[test]
    fn parse_option_with_bad_value_returns_error() {
        let bad_word: Option<Word> = from_json(json!("no"));

        let parsed = parse_or_field_errors(bad_word);

        assert_eq!(parsed, Err(field_error("", "expected ok")));
    }

    #[test]
    fn parse_derived_default_files_this_field_is_required() {
        #[validate]
        #[derive(Default, Debug, Deserialize, utoipa::ToSchema)]
        struct RequiredWord {
            word: Word,
        }

        let parsed = parse_or_field_errors(RequiredWord::default());

        assert_eq!(
            parsed.err(),
            Some(field_error("word", "this field is required"))
        );
    }

    #[test]
    fn field_with_own_error_files_it_under_field_name() {
        let mut errors = ValidationErrors::new();

        errors.field("username", from_json::<Word>(json!("no")));

        assert_eq!(errors.into_fields(), field_error("username", "expected ok"));
    }

    #[test]
    fn field_with_list_error_appends_index_to_field_name() {
        let mut errors = ValidationErrors::new();

        errors.field("roles", from_json::<Vec<Word>>(json!(["no"])));

        assert_eq!(errors.into_fields(), field_error("roles[0]", "expected ok"));
    }

    #[test]
    fn field_with_nested_errors_joins_paths_with_dot() {
        let mut errors = ValidationErrors::new();
        let host_and_port = HostAndPort(from_json(json!("no")), from_json(json!("no")));

        errors.field("connectivity", host_and_port);

        assert_eq!(
            errors.into_fields(),
            BTreeMap::from([
                (
                    "connectivity.host".to_owned(),
                    vec!["expected ok".to_owned()]
                ),
                (
                    "connectivity.port".to_owned(),
                    vec!["expected ok".to_owned()]
                ),
            ])
        );
    }

    #[test]
    fn required_absent_files_this_field_is_required() {
        let mut errors = ValidationErrors::new();

        let word = errors.required("word", Patch::<Word>::Absent);

        assert_eq!(word, None);
        assert_eq!(
            errors.into_fields(),
            field_error("word", "this field is required")
        );
    }

    #[test]
    fn required_present_returns_value() {
        let mut errors = ValidationErrors::new();

        let word = errors.required("word", Patch::Present(from_json::<Word>(json!("ok"))));

        assert_eq!(word, Some("ok".to_owned()));
    }

    #[test]
    fn optional_absent_returns_none() {
        let mut errors = ValidationErrors::new();
        let patch = from_json::<ClearableWordPatch>(json!({})).word;

        let word = errors.optional("word", patch);

        assert_eq!(word, Some(None));
    }

    #[test]
    fn optional_null_returns_none() {
        let mut errors = ValidationErrors::new();
        let patch = from_json::<ClearableWordPatch>(json!({ "word": null })).word;

        let word = errors.optional("word", patch);

        assert_eq!(word, Some(None));
    }

    #[test]
    fn optional_present_returns_value() {
        let mut errors = ValidationErrors::new();
        let patch = from_json::<ClearableWordPatch>(json!({ "word": "ok" })).word;

        let word = errors.optional("word", patch);

        assert_eq!(word, Some(Some("ok".to_owned())));
    }

    #[test]
    fn patch_absent_returns_no_change() {
        let mut errors = ValidationErrors::new();
        let patch = from_json::<WordPatch>(json!({})).word;

        let word = errors.patch("word", patch);

        assert_eq!(word, Some(None));
    }

    #[test]
    fn patch_bare_type_present_returns_value() {
        let mut errors = ValidationErrors::new();
        let patch = from_json::<WordPatch>(json!({ "word": "ok" })).word;

        let word = errors.patch("word", patch);

        assert_eq!(word, Some(Some("ok".to_owned())));
    }

    #[test]
    fn patch_bare_type_null_files_expected_a_string() {
        let mut errors = ValidationErrors::new();
        let patch = from_json::<WordPatch>(json!({ "word": null })).word;

        let word = errors.patch("word", patch);

        assert_eq!(word, None);
        assert_eq!(
            errors.into_fields(),
            field_error("word", "expected a string")
        );
    }

    #[test]
    fn patch_option_null_returns_clear() {
        let mut errors = ValidationErrors::new();
        let patch = from_json::<ClearableWordPatch>(json!({ "word": null })).word;

        let word = errors.patch("word", patch);

        assert_eq!(word, Some(Some(None)));
    }

    #[test]
    fn patch_option_present_returns_value() {
        let mut errors = ValidationErrors::new();
        let patch = from_json::<ClearableWordPatch>(json!({ "word": "ok" })).word;

        let word = errors.patch("word", patch);

        assert_eq!(word, Some(Some(Some("ok".to_owned()))));
    }

    #[test]
    fn build_after_rejection_returns_errors() {
        let mut errors = ValidationErrors::new();
        errors.field("word", from_json::<Word>(json!("no")));

        let built = errors.build(|| Some(()));

        assert_eq!(
            built.map_err(ValidationErrors::into_fields),
            Err(field_error("word", "expected ok"))
        );
    }

    #[test]
    fn deserialize_word_from_other_json_type_carries_expected_a_string() {
        let other_json_types = [
            json!(17),
            json!(1.5),
            json!(true),
            json!(null),
            json!(["a", "b"]),
            json!({ "a": 1 }),
        ];

        for json_value in other_json_types {
            let word: Word = from_json(json_value.clone());

            assert_eq!(
                parse_or_field_errors(word),
                Err(field_error("", "expected a string")),
                "{json_value}"
            );
        }
    }

    #[test]
    fn deserialize_after_rejected_nested_value_reads_next_field() {
        #[derive(Deserialize)]
        struct TwoWords {
            first: Word,
            second: Word,
        }

        let words: TwoWords = from_json(json!({
            "first": { "buried": ["deep", { "deeper": 1 }] },
            "second": "ok",
        }));

        assert_eq!(
            (
                parse_or_field_errors(words.first),
                parse_or_field_errors(words.second)
            ),
            (
                Err(field_error("", "expected a string")),
                Ok("ok".to_owned())
            )
        );
    }

    #[test]
    fn deserialize_raw_number_from_word_carries_expected_a_whole_number() {
        let number: RawNumber = from_json(json!("twenty"));

        assert_eq!(
            parse_or_field_errors(number),
            Err(field_error("", "expected a whole number"))
        );
    }

    #[test]
    fn deserialize_raw_number_negative_carries_expected_a_positive_whole_number() {
        let number: RawNumber = from_json(json!(-1));

        assert_eq!(
            parse_or_field_errors(number),
            Err(field_error("", "expected a positive whole number"))
        );
    }

    #[test]
    fn deserialize_u64_newtype_from_text_reads_number() {
        let count: Count = from_json(json!("7"));

        assert_eq!(parse_or_field_errors(count), Ok(7));
    }

    #[test]
    fn deserialize_narrowed_number_from_number_reads_it() {
        let even: EvenU8 = from_json(json!(4));

        assert_eq!(parse_or_field_errors(even), Ok(4));
    }

    #[test]
    fn deserialize_narrowed_number_from_text_reads_number() {
        let even: EvenU8 = from_json(json!("4"));

        assert_eq!(parse_or_field_errors(even), Ok(4));
    }

    #[test]
    fn deserialize_narrowed_number_at_maximum_reads_it() {
        let port: PlainU16 = from_json(json!("65535"));

        assert_eq!(parse_or_field_errors(port), Ok(65535));
    }

    #[test]
    fn deserialize_narrowed_number_from_word_carries_expected_a_whole_number() {
        let even: EvenU8 = from_json(json!("four"));

        assert_eq!(
            parse_or_field_errors(even),
            Err(field_error("", "expected a whole number"))
        );
    }

    #[test]
    fn deserialize_narrowed_number_above_maximum_carries_error() {
        let even: EvenU8 = from_json(json!("256"));

        assert_eq!(
            parse_or_field_errors(even),
            Err(field_error("", "expected an integer no larger than 255"))
        );
    }

    #[test]
    fn deserialize_narrowed_number_refused_by_hook_carries_hook_error() {
        let even: EvenU8 = from_json(json!("3"));

        assert_eq!(
            parse_or_field_errors(even),
            Err(field_error("", "expected an even number"))
        );
    }

    #[test]
    fn serialize_number_hook_writes_number() {
        let written = serde_json::to_value(Count::from(7)).unwrap();

        assert_eq!(written, json!(7));
    }

    #[test]
    fn serialize_string_hook_writes_string() {
        let written = serde_json::to_value(Word::from("ok".to_owned())).unwrap();

        assert_eq!(written, json!("ok"));
    }

    #[test]
    fn serialize_hook_error_returns_error() {
        let written = serde_json::to_value(UnwritableText::from("anything".to_owned()));

        assert_eq!(
            written.unwrap_err().to_string(),
            "this value cannot be written"
        );
    }

    #[test]
    fn serialize_rejected_value_returns_error() {
        let count: Count = from_json(json!("seven"));

        let written = serde_json::to_value(count);

        assert_eq!(
            written.unwrap_err().to_string(),
            "a rejected value cannot be written to a response"
        );
    }

    // Some of these values are secrets, and a request struct derives `Debug`.
    #[test]
    fn fmt_accepted_value_omits_it() {
        let text: RawString = from_json(json!("hunter2"));

        let debug_text = format!("{text:?}");

        assert_eq!(debug_text, "RawString(accepted)");
    }

    #[test]
    fn fmt_rejected_value_shows_only_the_reason() {
        let word: Word = from_json(json!("hunter2"));

        let debug_text = format!("{word:?}");

        assert_eq!(debug_text, "Word(rejected: expected ok)");
    }

    #[test]
    fn schema_of_validated_struct_describes_each_field_spelling() {
        #[validate]
        #[derive(Debug, Deserialize, utoipa::ToSchema)]
        struct EveryFieldSpelling {
            must: Word,
            plain: String,
            may: Option<Word>,
            strict: Patch<Word>,
            clearable: Patch<Option<Word>>,
        }

        let schema =
            serde_json::to_value(<EveryFieldSpelling as utoipa::PartialSchema>::schema()).unwrap();

        let word = json!({ "$ref": "#/components/schemas/Word" });
        let nullable_word = json!({ "oneOf": [{ "type": "null" }, word] });
        assert_eq!(
            schema,
            json!({
                "type": "object",
                "required": ["must", "plain"],
                "properties": {
                    "must": word,
                    "plain": { "type": "string" },
                    "may": nullable_word,
                    "strict": word,
                    "clearable": nullable_word,
                },
            })
        );
    }

    #[test]
    fn validated_with_left_out_option_and_null_patch_returns_none_and_clear() {
        let update: WordUpdate = from_json(json!({ "clear": null }));

        let valid_update = update.validated().unwrap();

        assert_eq!((valid_update.leave, valid_update.clear), (None, Some(None)));
    }

    #[test]
    fn validated_with_sent_option_and_patch_returns_values() {
        let update: WordUpdate = from_json(json!({ "leave": "ok", "clear": "ok" }));

        let valid_update = update.validated().unwrap();

        assert_eq!(
            (valid_update.leave, valid_update.clear),
            (Some("ok".to_owned()), Some(Some("ok".to_owned())))
        );
    }

    #[test]
    fn validated_with_twin_name_returns_named_twin() {
        #[validate(ValidWord)]
        #[derive(Debug, Deserialize, utoipa::ToSchema)]
        struct RequiredWord {
            word: Word,
        }

        let body: RequiredWord = from_json(json!({ "word": "ok" }));

        let valid_word: ValidWord = body.validated().unwrap();

        assert_eq!(valid_word.word, "ok");
    }

    #[test]
    fn validated_defaulted_field_left_out_returns_type_default() {
        let query: PageQuery = from_json(json!({ "name": "anything" }));

        let valid_query = query.validated().unwrap();

        assert_eq!(valid_query.page, 8);
    }

    #[test]
    fn validated_defaulted_field_sent_wrong_returns_error() {
        let query: PageQuery = from_json(json!({ "name": "anything", "page": "3" }));

        let errors = query.validated().err().map(ValidationErrors::into_fields);

        assert_eq!(errors, Some(field_error("page", "expected an even number")));
    }

    #[test]
    fn validated_defaulted_fields_left_out_return_type_and_field_defaults() {
        let query: DefaultsQuery = from_json(json!({}));

        let valid_query = query.validated().unwrap();

        assert_eq!(
            (valid_query.heading, valid_query.word.as_str()),
            (Direction::Down, "ok")
        );
    }

    #[test]
    fn validated_field_default_sent_wrong_returns_error() {
        let query: DefaultsQuery = from_json(json!({ "word": "wrong" }));

        let errors = query.validated().err().map(ValidationErrors::into_fields);

        assert_eq!(errors, Some(field_error("word", "expected ok")));
    }

    #[test]
    fn validated_renamed_field_wrong_files_error_under_sent_name() {
        let body: RenamedField = from_json(json!({ "type": "wrong" }));

        let errors = body.validated().err().map(ValidationErrors::into_fields);

        assert_eq!(errors, Some(field_error("type", "expected ok")));
    }

    #[test]
    fn validated_skipped_tuple_variant_returns_it_unchanged() {
        let validated = EnumWithSkippedVariants::Bytes(vec![1, 2]).validated();

        let Ok(EnumWithSkippedVariantsParsed::Bytes(bytes)) = validated else {
            panic!("expected the Bytes variant");
        };
        assert_eq!(bytes, [1, 2]);
    }

    #[test]
    fn validated_skipped_unit_variant_returns_it() {
        let validated = EnumWithSkippedVariants::End.validated();

        assert!(matches!(validated, Ok(EnumWithSkippedVariantsParsed::End)));
    }

    #[test]
    fn validated_variant_beside_skipped_ones_files_error() {
        let count: EnumWithSkippedVariants = from_json(json!({ "type": "count", "count": 3 }));

        let errors = count.validated().err().map(ValidationErrors::into_fields);

        assert_eq!(
            errors,
            Some(field_error("count", "expected an even number"))
        );
    }

    #[test]
    fn into_params_of_page_query_describes_each_field_inline() {
        let parameters = query_parameters::<PageQuery>();

        assert_eq!(
            parameters,
            json!([
                {
                    "in": "query",
                    "name": "size",
                    "required": false,
                    "schema": { "type": "integer", "default": 8 },
                },
                {
                    "in": "query",
                    "name": "page",
                    "required": false,
                    "schema": { "type": "integer", "default": 8 },
                },
                {
                    "in": "query",
                    "name": "name",
                    "required": true,
                    "schema": { "type": "string" },
                },
            ])
        );
    }

    #[test]
    fn into_params_of_defaults_query_describes_type_and_field_defaults() {
        let parameters = query_parameters::<DefaultsQuery>();

        assert_eq!(
            parameters,
            json!([
                {
                    "in": "query",
                    "name": "heading",
                    "required": false,
                    "schema": { "type": "string", "enum": ["up", "down"], "default": "down" },
                },
                {
                    "in": "query",
                    "name": "word",
                    "required": false,
                    "schema": { "oneOf": [{ "type": "string" }], "default": "ok" },
                },
            ])
        );
    }
}

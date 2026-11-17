//! The values a v1 request is read into: a text of so many characters, a text that can be `null`, a text with no bound.
//!
//! Each is one thing with two uses. [`Text::read`] reads a JSON value as DRF's `CharField` does, and (with the `openapi` feature) the
//! type writes its own schema, `minLength` and `maxLength` from the same two numbers. So a bound is written once, in a field's type
//! (`Text<1, 255>`), and the parser and the document cannot say different ones: there is no second place to change.
//!
//! What the type says about `null` is part of the type: [`Text`] never takes one, [`NullableText`] does and writes `type: [string,
//! null]`. Whether a key may be left out is the field's (`Option<_>` in the request struct), as it is in the core patches.

use serde_json::Value;
use thrust_contracts::Uuid;

use crate::fields::{char_field, choice_by, int_field, pk_field, uuid_field, PkRead};

/// A text of `MIN` to `MAX` characters, read as DRF's `CharField` reads one: trimmed, counted in characters, a number taken as its
/// text, a bool, a list and an object refused, and `null` refused. `MIN` is 0 (blank allowed) or 1 (blank refused): DRF's flag is that
/// and not a count, here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Text<const MIN: usize, const MAX: usize>(String);

impl<const MIN: usize, const MAX: usize> Text<MIN, MAX> {
    /// What `MIN` can be: `CharField` knows blank or not.
    const MIN_IS_BLANK_OR_NOT: () = assert!(MIN <= 1, "a Text has a minimum of 0 or 1: DRF's blank flag");

    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        let () = Self::MIN_IS_BLANK_OR_NOT;
        char_field(v, Some(MAX), MIN == 0, false).map(|text| Self(text.unwrap_or_default()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// A [`Text`] that can also be `null`, which is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NullableText<const MIN: usize, const MAX: usize>(Option<String>);

impl<const MIN: usize, const MAX: usize> NullableText<MIN, MAX> {
    const MIN_IS_BLANK_OR_NOT: () = assert!(MIN <= 1, "a Text has a minimum of 0 or 1: DRF's blank flag");

    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        let () = Self::MIN_IS_BLANK_OR_NOT;
        char_field(v, Some(MAX), MIN == 0, true).map(Self)
    }

    pub(crate) fn into_option(self) -> Option<String> {
        self.0
    }
}

/// The key of a related row as a body gave it: the UUID it reads as, and the text it was sent as, which is what DRF quotes when no row
/// has the key (`Invalid pk "<shown>" - object does not exist.`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Key {
    pub(crate) id: Uuid,
    pub(crate) shown: String,
}

/// A related row's key, or `null` (which is `None`): DRF's `PrimaryKeyRelatedField` over a UUID key, read as `pk_field` reads it. Only
/// the key is read here; that a row has it is the route's to ask (a second stage, which needs the database).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NullableKey(Option<Key>);

impl NullableKey {
    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        match pk_field(v) {
            PkRead::Null => Ok(Self(None)),
            PkRead::Key { id, shown } => Ok(Self(Some(Key { id, shown }))),
            PkRead::Refused(message) => Err(message),
        }
    }

    pub(crate) fn key(&self) -> Option<&Key> {
        self.0.as_ref()
    }

    pub(crate) fn into_id(self) -> Option<Uuid> {
        self.0.map(|key| key.id)
    }
}

/// A text with no bound that may be blank and is never `null` (a `TextField`: a description).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Prose(String);

impl Prose {
    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        char_field(v, None, true, false).map(|text| Self(text.unwrap_or_default()))
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// A [`Prose`] that can also be `null`, which is `None` (a `cover_image`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NullableProse(Option<String>);

impl NullableProse {
    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        char_field(v, None, true, true).map(Self)
    }

    pub(crate) fn into_option(self) -> Option<String> {
        self.0
    }
}

/// A whole number from `MIN` to `MAX`, read as DRF's `IntegerField` with those validators reads one: a float with no fraction is an
/// integer, a number as text is read, and a value outside the bounds is refused with DRF's words for the side it is out on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Whole<const MIN: i32, const MAX: i32>(i32);

impl<const MIN: i32, const MAX: i32> Whole<MIN, MAX> {
    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        let n = int_field(v, i64::from(MIN), i64::from(MAX))?;
        i32::try_from(n).map(Self).map_err(|e| e.to_string())
    }

    pub(crate) fn get(self) -> i32 {
        self.0
    }
}

/// A timezone: one of the names that `thrust_core::is_known_timezone` says yes to, which is the list the document gives for it, read as
/// DRF's `ChoiceField` reads one (as sent: no trimming).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Timezone(String);

impl Timezone {
    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        choice_by(v, thrust_core::is_known_timezone).map(Self)
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// A JSON value of any kind, which is kept as it is; `null` is `None` (a `JSONField`: `icon_prop`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AnyJson(Option<Value>);

impl AnyJson {
    #[allow(
        clippy::unnecessary_wraps,
        reason = "the reader of every value has the same shape: this is the one that reads anything"
    )]
    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        Ok(Self(if v.is_null() { None } else { Some(v.clone()) }))
    }

    pub(crate) fn into_option(self) -> Option<Value> {
        self.0
    }
}

/// A UUID or `null` (which is `None`), read as DRF's `UUIDField` reads one: what a project's two people are named by. That a row has
/// the key is the route's to ask (a second stage). Not [`NullableKey`]: that is a related field's reading, with its own words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NullableUuid(Option<Uuid>);

impl NullableUuid {
    pub(crate) fn read(v: &Value) -> Result<Self, String> {
        uuid_field(v).map(Self)
    }

    pub(crate) fn id(self) -> Option<Uuid> {
        self.0
    }
}

#[cfg(feature = "openapi")]
mod schema {
    //! What each type says in the document. An `Option<_>` field of one of these is written inline and not nullable
    //! (`#[schema(inline, nullable = false)]`): utoipa would wrap a nullable value of a type of ours in a `oneOf`, and a type says
    //! for itself whether it takes `null`.

    use std::borrow::Cow;

    use utoipa::openapi::schema::{KnownFormat, ObjectBuilder, SchemaFormat, SchemaType, Type};
    use utoipa::openapi::{RefOr, Schema};
    use utoipa::{PartialSchema, ToSchema};

    use super::{AnyJson, NullableKey, NullableProse, NullableText, NullableUuid, Prose, Text, Timezone, Whole};

    /// `type: string` (or `[string, null]`) with the bounds that are there.
    fn text(nullable: bool, min: usize, max: Option<usize>) -> RefOr<Schema> {
        let types = if nullable {
            [Type::String, Type::Null].into_iter().collect::<SchemaType>()
        } else {
            SchemaType::Type(Type::String)
        };
        let mut object = ObjectBuilder::new().schema_type(types).max_length(max);
        if min > 0 {
            object = object.min_length(Some(min));
        }
        RefOr::T(Schema::Object(object.build()))
    }

    impl<const MIN: usize, const MAX: usize> PartialSchema for Text<MIN, MAX> {
        fn schema() -> RefOr<Schema> {
            text(false, MIN, Some(MAX))
        }
    }

    impl<const MIN: usize, const MAX: usize> ToSchema for Text<MIN, MAX> {
        fn name() -> Cow<'static, str> {
            Cow::Owned(format!("Text_{MIN}_{MAX}"))
        }
    }

    impl<const MIN: usize, const MAX: usize> PartialSchema for NullableText<MIN, MAX> {
        fn schema() -> RefOr<Schema> {
            text(true, MIN, Some(MAX))
        }
    }

    impl<const MIN: usize, const MAX: usize> ToSchema for NullableText<MIN, MAX> {
        fn name() -> Cow<'static, str> {
            Cow::Owned(format!("NullableText_{MIN}_{MAX}"))
        }
    }

    impl PartialSchema for NullableKey {
        fn schema() -> RefOr<Schema> {
            uuid_or_null()
        }
    }

    impl ToSchema for NullableKey {
        fn name() -> Cow<'static, str> {
            Cow::Borrowed("NullableKey")
        }
    }

    fn uuid_or_null() -> RefOr<Schema> {
        let object = ObjectBuilder::new()
            .schema_type([Type::String, Type::Null].into_iter().collect::<SchemaType>())
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Uuid)));
        RefOr::T(Schema::Object(object.build()))
    }

    impl PartialSchema for NullableUuid {
        fn schema() -> RefOr<Schema> {
            uuid_or_null()
        }
    }

    impl ToSchema for NullableUuid {
        fn name() -> Cow<'static, str> {
            Cow::Borrowed("NullableUuid")
        }
    }

    impl PartialSchema for NullableProse {
        fn schema() -> RefOr<Schema> {
            text(true, 0, None)
        }
    }

    impl ToSchema for NullableProse {
        fn name() -> Cow<'static, str> {
            Cow::Borrowed("NullableProse")
        }
    }

    /// `type: integer` with the two bounds and no `format`, as the pin writes a validated integer.
    impl<const MIN: i32, const MAX: i32> PartialSchema for Whole<MIN, MAX> {
        fn schema() -> RefOr<Schema> {
            let object = ObjectBuilder::new()
                .schema_type(Type::Integer)
                .minimum(Some(i64::from(MIN)))
                .maximum(Some(i64::from(MAX)));
            RefOr::T(Schema::Object(object.build()))
        }
    }

    impl<const MIN: i32, const MAX: i32> ToSchema for Whole<MIN, MAX> {
        fn name() -> Cow<'static, str> {
            Cow::Owned(format!("Whole_{MIN}_{MAX}"))
        }
    }

    /// `type: string` with the zones `is_known_timezone` knows as its enum: the list a route checks a body against.
    impl PartialSchema for Timezone {
        fn schema() -> RefOr<Schema> {
            let object = ObjectBuilder::new()
                .schema_type(Type::String)
                .enum_values(Some(thrust_core::known_timezones().iter().copied()));
            RefOr::T(Schema::Object(object.build()))
        }
    }

    impl ToSchema for Timezone {
        fn name() -> Cow<'static, str> {
            Cow::Borrowed("Timezone")
        }
    }

    /// No type and no bound: any value, which is `{}`.
    impl PartialSchema for AnyJson {
        fn schema() -> RefOr<Schema> {
            RefOr::T(Schema::Object(ObjectBuilder::new().schema_type(SchemaType::AnyValue).build()))
        }
    }

    impl ToSchema for AnyJson {
        fn name() -> Cow<'static, str> {
            Cow::Borrowed("AnyJson")
        }
    }

    impl PartialSchema for Prose {
        fn schema() -> RefOr<Schema> {
            text(false, 0, None)
        }
    }

    impl ToSchema for Prose {
        fn name() -> Cow<'static, str> {
            Cow::Borrowed("Prose")
        }
    }
}

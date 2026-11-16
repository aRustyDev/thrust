//! The values a v1 request is read into: a text of so many characters, a text that can be `null`, a text with no bound.
//!
//! Each is one thing with two uses. [`Text::read`] reads a JSON value as DRF's `CharField` does, and (with the `openapi` feature) the
//! type writes its own schema, `minLength` and `maxLength` from the same two numbers. So a bound is written once, in a field's type
//! (`Text<1, 255>`), and the parser and the document cannot say different ones: there is no second place to change.
//!
//! What the type says about `null` is part of the type: [`Text`] never takes one, [`NullableText`] does and writes `type: [string,
//! null]`. Whether a key may be left out is the field's (`Option<_>` in the request struct), as it is in the core patches.

use serde_json::Value;

use crate::fields::char_field;

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

#[cfg(feature = "openapi")]
mod schema {
    //! What each type says in the document. An `Option<_>` field of one of these is written inline and not nullable
    //! (`#[schema(inline, nullable = false)]`): utoipa would wrap a nullable value of a type of ours in a `oneOf`, and a type says
    //! for itself whether it takes `null`.

    use std::borrow::Cow;

    use utoipa::openapi::schema::{ObjectBuilder, SchemaType, Type};
    use utoipa::openapi::{RefOr, Schema};
    use utoipa::{PartialSchema, ToSchema};

    use super::{NullableText, Prose, Text};

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

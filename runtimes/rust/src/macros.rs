//! Macros that generated code invokes for what serde's derives cannot express.

/// Implements `Serialize` and `Deserialize` for a discriminated union whose members are structs
/// that keep the tag field (serde's internally tagged enums would remove it). The enum has one
/// newtype variant per member, and optionally a variant holding the JSON of unlisted tags:
///
/// ```ignore
/// enum Pet { Cat(Cat), Dog(Dog), Unknown(serde_json::Value) }
/// tsurara_client::tagged_union! {
///     Pet, tag "petType" {
///         Cat: ["cat"],
///         Dog: ["dog"],
///     }
///     unknown Unknown;
/// }
/// ```
///
/// `default Member;` names a variant (not in the list) that values with an unlisted tag decode
/// as, the `defaultMapping` of OpenAPI 3.2; it comes before `unknown`.
#[macro_export]
macro_rules! tagged_union {
    (
        $name:ident, tag $tag:literal {
            $( $variant:ident : [ $( $value:literal ),+ $(,)? ] ),+ $(,)?
        }
        $( default $default:ident; )?
        $( unknown $unknown:ident; )?
    ) => {
        impl $crate::serde::Serialize for $name {
            fn serialize<S: $crate::serde::Serializer>(&self, serializer: S) -> ::std::result::Result<S::Ok, S::Error> {
                match self {
                    $( $name::$variant(v) => $crate::serde::Serialize::serialize(v, serializer), )+
                    $( $name::$default(v) => $crate::serde::Serialize::serialize(v, serializer), )?
                    $( $name::$unknown(v) => $crate::serde::Serialize::serialize(v, serializer), )?
                }
            }
        }

        impl<'de> $crate::serde::Deserialize<'de> for $name {
            fn deserialize<D: $crate::serde::Deserializer<'de>>(deserializer: D) -> ::std::result::Result<Self, D::Error> {
                use $crate::serde::de::Error as _;
                let value = <$crate::serde_json::Value as $crate::serde::Deserialize>::deserialize(deserializer)?;
                let tag = value.get($tag).and_then($crate::serde_json::Value::as_str).map(::std::borrow::ToOwned::to_owned);
                match tag.as_deref() {
                    $(
                        $( Some($value) )|+ => $crate::serde_json::from_value(value)
                            .map($name::$variant)
                            .map_err(D::Error::custom),
                    )+
                    _ => {
                        $(
                            return $crate::serde_json::from_value(value)
                                .map($name::$default)
                                .map_err(D::Error::custom);
                        )?
                        $( return Ok($name::$unknown(value)); )?
                        #[allow(unreachable_code)]
                        Err(D::Error::custom(format!(
                            "`{}` is not a listed value of the discriminator `{}`",
                            tag.unwrap_or_default(),
                            $tag
                        )))
                    }
                }
            }
        }
    };
}

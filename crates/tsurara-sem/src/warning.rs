//! Warnings for approximate conversions (design.md 6.8).

use std::fmt;
use tsurara_loader::Loc;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum WarningKind {
    /// A keyword the model cannot represent was dropped (`not`, `if`, `dependentSchemas`), or a value
    /// cannot be what the definition says (`format: binary` inside JSON).
    Unsupported,
    /// `$dynamicRef` was resolved statically per reference site.
    DynamicRef,
    /// A name was changed to avoid a collision.
    Renamed,
    /// A reference could not be resolved and became `unknown`.
    UnresolvedRef,
    /// The definition has an unexpected shape and was interpreted as far as possible.
    InvalidDefinition,
    /// A keyword written next to `$ref` was ignored (OpenAPI 3.0 ignores all siblings of `$ref`).
    IgnoredSibling,
    /// A wildcard media type (`*/*`, `application/*`) with a schema that is not binary. The body is
    /// parsed by its actual Content-Type (`auto`) and sent as JSON (design.md 8.5).
    WildcardMediaType,
    /// A mapped type (`x-tsurara-type` and so on) has no entry for the language; the schema's type is used.
    UnmappedType,
    /// A required property refers back to its own declaration, so its sample value stops at a fixed
    /// depth with `{}` (design.md 18.7.1).
    SampleCycle,
    /// A string with `pattern` and no `example`: its sample value may not match (design.md 18.7.1).
    SamplePattern,
}

impl WarningKind {
    /// Every kind, in declaration order (`tsurara inspect warning`).
    pub const ALL: &[WarningKind] = &[
        WarningKind::Unsupported,
        WarningKind::DynamicRef,
        WarningKind::Renamed,
        WarningKind::UnresolvedRef,
        WarningKind::InvalidDefinition,
        WarningKind::IgnoredSibling,
        WarningKind::WildcardMediaType,
        WarningKind::UnmappedType,
        WarningKind::SampleCycle,
        WarningKind::SamplePattern,
    ];

    /// The kind with code `code` (`unresolved-ref`).
    pub fn from_code(code: &str) -> Option<WarningKind> {
        Self::ALL.iter().copied().find(|k| k.as_str() == code)
    }

    /// What the warning means and how to resolve it (`tsurara inspect warning`, design.md 6.8).
    pub fn explanation(self) -> &'static str {
        match self {
            WarningKind::Unsupported => {
                "A keyword the generator cannot represent was dropped (`not`, `if`/`then`/`else`, `dependentSchemas`, `unevaluatedProperties`, ...), or a value cannot be what the definition says (`format: binary` inside JSON). The generated type is wider than the schema: it accepts values the schema rejects. Rewrite the schema without the keyword if the type matters, or accept the approximation."
            }
            WarningKind::DynamicRef => {
                "`$dynamicRef` was resolved statically at each place it is used, so a generic schema becomes one concrete type per use. Nothing to do unless a use resolves to the wrong anchor; then use `$ref` instead."
            }
            WarningKind::Renamed => {
                "A generated name collided with another (two schemas that differ only in case, a name derived from an operation that equals a schema name, a reserved word) and was changed. Set the name you want with `naming.overrides` (keyed by the JSON Pointer in the warning), or rename the schema or `operationId` in the definition."
            }
            WarningKind::UnresolvedRef => {
                "A `$ref` could not be resolved (missing file, missing pointer, unreachable URL) and its type became `unknown`. Fix the reference, or make the referenced document reachable (`input.headers` for authenticated URLs)."
            }
            WarningKind::InvalidDefinition => {
                "The definition has a shape OpenAPI does not allow there (a wrong type for a keyword, a parameter without `in`, an external type that fails the grammar check of chapter 11, a path variable without an `in: path` parameter, which becomes a required string, or a path parameter that is not in the path, which is left out; a path, method, media type or channel with a control character, or a path without a leading `/`, which is left out), and it was interpreted as far as possible or ignored. Fix the definition at the JSON Pointer in the warning."
            }
            WarningKind::IgnoredSibling => {
                "A keyword written next to `$ref` was ignored: OpenAPI 3.0 ignores every sibling of `$ref`. Move the keyword into the referenced schema, wrap the reference in `allOf: [{ $ref: ... }]`, or use OpenAPI 3.1, where siblings apply."
            }
            WarningKind::WildcardMediaType => {
                "A wildcard media type (`*/*`, `application/*`) has a schema that is not binary. The body is parsed by its actual Content-Type and sent as JSON, and its type is the union of what it can be. List the concrete media types in the definition, or map the wildcard to JSON in the configuration."
            }
            WarningKind::UnmappedType => {
                "A mapped type (`x-tsurara-type`, `typeMappings`, `overrides`) has no entry for this language, so the schema's own type is used. Add an entry for the language if it should be mapped too."
            }
            WarningKind::SampleCycle => {
                "A required property refers back to its own declaration, so the schema has no finite value; its sample value (mocks, call examples) stops at depth two with `{}`. Make the property optional or nullable if the cycle ends in real data."
            }
            WarningKind::SamplePattern => {
                "A string has a `pattern` and no `example`, so its sample value (mocks, call examples) may not match it. Add an `example` that matches the pattern."
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WarningKind::Unsupported => "unsupported",
            WarningKind::DynamicRef => "dynamic-ref",
            WarningKind::Renamed => "renamed",
            WarningKind::UnresolvedRef => "unresolved-ref",
            WarningKind::InvalidDefinition => "invalid-definition",
            WarningKind::IgnoredSibling => "ignored-sibling",
            WarningKind::WildcardMediaType => "wildcard-media-type",
            WarningKind::UnmappedType => "unmapped-type",
            WarningKind::SampleCycle => "sample-cycle",
            WarningKind::SamplePattern => "sample-pattern",
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub kind: WarningKind,
    pub loc: Loc,
    pub message: String,
    /// The backend that reported it (`typescript`), or `None` for warnings about the definition itself
    /// (design.md 18.4.5).
    pub language: Option<String>,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.kind.as_str(), self.loc, self.message)
    }
}

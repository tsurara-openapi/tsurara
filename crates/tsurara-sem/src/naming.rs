//! Name requests (design.md 18.4.2, 18.6). The model holds no identifiers chosen for a language: it
//! records which names are needed, with their preferred names, in the order the TypeScript generator
//! has always claimed them. A backend replays the requests in this order with its own policy.

use tsurara_loader::Loc;

/// Identifies a requested name. References in the model (`Type::Named`) point to it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct NameId(pub u32);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct NameRequest {
    pub id: NameId,
    pub kind: NameKind,
    /// Where the named thing is defined. Its JSON Pointer is the key of `naming.overrides`.
    pub loc: Loc,
    /// Whether `loc` is in the root document (only names there can be overridden).
    pub in_root: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum NameKind {
    /// A schema declaration. `preserve`: the preferred name comes from the definition (a components
    /// or `$defs` key) and is kept when it is a valid identifier.
    Schema { preferred: String, preserve: bool },
    /// The `Writable` variant of a schema (design.md 6.5).
    Writable { of: NameId },
    /// An operation in `paths`.
    Operation {
        operation_id: Option<String>,
        method: String,
        path: String,
    },
    /// A webhook, by its name.
    Webhook { name: String },
    /// An event channel of `x-event-channels`, by its name.
    Channel { name: String },
    /// An operation of a callback. Its name is derived from the operation that has the callback.
    Callback {
        parent: NameId,
        label: String,
        /// The method, when the callback has several operations; empty otherwise.
        method: String,
    },
}

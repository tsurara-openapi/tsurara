//! What a backend generated, by name: for each declaration of the model and each
//! operation, the identifiers the backend chose and the file that declares them. Plugins of the
//! `extend` stage read it to write files that refer to generated code without re-deriving its names.

use std::collections::BTreeMap;
use tsurara_sem::naming::NameId;

/// Version of the manifest's JSON form. It changes only when the form changes incompatibly.
pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// `MANIFEST_VERSION`.
    pub manifest_version: u32,
    /// The backend's `id`.
    pub language: String,
    /// The declarations of the model (`Model::schemas`) the backend emitted, in model order.
    /// Declarations the backend adds on its own (hoisted types) are not listed.
    pub types: Vec<TypeEntry>,
    /// The operations, then the webhooks, in model order.
    pub operations: Vec<OperationEntry>,
}

impl Manifest {
    pub fn new(language: &str) -> Self {
        Self {
            manifest_version: MANIFEST_VERSION,
            language: language.to_owned(),
            types: Vec::new(),
            operations: Vec::new(),
        }
    }
}

/// An identifier and the file that declares it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Export {
    pub identifier: String,
    /// Path of the file relative to the output root (`/`-separated), as in `OutputFile::path`.
    pub file: String,
}

impl Export {
    pub fn new(identifier: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            file: file.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct TypeEntry {
    /// The declaration's `TypeDecl::id`.
    pub name: NameId,
    #[serde(flatten)]
    pub export: Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    Operation,
    Webhook,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct OperationEntry {
    /// `Operation::id`.
    pub operation_id: String,
    pub kind: OperationKind,
    /// The call function, if the backend emits one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<Export>,
    /// The OperationDef value, if the backend emits one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_def: Option<Export>,
    /// Types derived from the operation, by role: `data`, `result`, `responses`, `errors`, `options`,
    /// `event`, `item`, `request`, ... Each language lists the roles it has (docs/guide/plugins.md).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub types: BTreeMap<String, Export>,
    /// Other values generated for the operation, by role: with TypeScript's
    /// `query: tanstack`, `queryKey`, `queryOptions`, `queryFilter`, `setQueriesData` and
    /// `getQueriesData` (reads), `infiniteQueryKey` and `infiniteQueryOptions` (paginated reads) and
    /// `mutationOptions` (writes); with `mocks`, `mockHandler`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub helpers: BTreeMap<String, Export>,
}

/// `entries` sorted in the order of `model.schemas`, for backends whose declarations come in another
/// order. Entries of names the model does not declare go last, in their order.
pub fn in_model_order(model: &tsurara_sem::Model, mut entries: Vec<TypeEntry>) -> Vec<TypeEntry> {
    let index = |id: NameId| {
        model
            .schemas
            .iter()
            .position(|d| d.id == id)
            .unwrap_or(usize::MAX)
    };
    entries.sort_by_key(|e| index(e.name));
    entries
}

//! Generation options common to every language (design.md 18.10). Options of one language live in its
//! backend crate.

use crate::ir::External;
use std::collections::BTreeMap;

/// Filter for the operations to generate. If everything is empty, all are generated.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    /// Only operations with one of these tags or `operation_ids`; all when both are empty.
    pub tags: Vec<String>,
    pub operation_ids: Vec<String>,
    /// Operations with one of these tags or `exclude_operation_ids` are left out, also when they
    /// are included above.
    pub exclude_tags: Vec<String>,
    pub exclude_operation_ids: Vec<String>,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
            && self.operation_ids.is_empty()
            && self.exclude_tags.is_empty()
            && self.exclude_operation_ids.is_empty()
    }

    /// Whether an operation with this `operationId` and these tags is generated.
    pub fn selects(&self, id: Option<&str>, tags: &[String]) -> bool {
        let matches = |ids: &[String], among: &[String]| {
            tags.iter().any(|t| among.contains(t))
                || id.is_some_and(|id| ids.iter().any(|i| i == id))
        };
        let included = (self.tags.is_empty() && self.operation_ids.is_empty())
            || matches(&self.operation_ids, &self.tags);
        included && !matches(&self.exclude_operation_ids, &self.exclude_tags)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Options {
    /// Split types by readOnly/writeOnly.
    pub split_visibility: bool,
    /// Preferred names per JSON Pointer (such as `#/components/schemas/Error`); each language's naming
    /// rules still apply to them.
    pub name_overrides: BTreeMap<String, String>,
    /// External types per `format`, then per language.
    pub type_mappings: BTreeMap<String, BTreeMap<String, External>>,
    /// External types per JSON Pointer, then per language.
    pub type_overrides: BTreeMap<String, BTreeMap<String, External>>,
    /// Directory, relative to the output root, that relative `import.from` specifiers in `x-tsurara-type`
    /// are written against (the directory of the configuration file). Empty means the output root.
    pub external_import_base: String,
    pub filter: Filter,
    /// Treat wildcard media types (`*/*`, `application/*`) as JSON, for APIs that only return JSON.
    pub wildcard_as_json: bool,
    /// Generate the operations of `paths` (OperationDefs, call functions and their types). Without
    /// them the output holds the declarations, webhooks and channels, which need only the types layer
    /// of the runtime.
    pub operations: bool,
    /// Where generated code finds its runtime.
    pub runtime: RuntimeSource,
}

/// Where generated code finds its runtime library.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum RuntimeSource {
    /// The package of the language's registry (`@tsurara/client`, `tsurara-client`, ...).
    #[default]
    Package,
    /// A copy of the runtime's sources written into the output, which generated code imports
    /// instead of the package.
    Vendor,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            split_visibility: true,
            name_overrides: BTreeMap::new(),
            type_mappings: BTreeMap::new(),
            type_overrides: BTreeMap::new(),
            external_import_base: String::new(),
            filter: Filter::default(),
            wildcard_as_json: false,
            operations: true,
            runtime: RuntimeSource::Package,
        }
    }
}

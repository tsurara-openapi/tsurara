//! Options of the TypeScript backend (design.md 18.10: the `typescript` section).

use std::collections::BTreeMap;

/// How schema names are converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NameCase {
    /// Keep valid identifiers as is.
    #[default]
    Preserve,
    /// Convert to PascalCase.
    Pascal,
}

/// How function names are made from operation ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FunctionCase {
    /// Keep operation ids that are identifiers as is.
    #[default]
    Preserve,
    /// Convert every operation id to camelCase.
    Camel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// One file per schema, operation, and webhook.
    #[default]
    Split,
    /// One file per tag for operations.
    Tags,
    /// Everything in one file.
    Single,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImportExtension {
    #[default]
    Js,
    Ts,
    None,
}

impl ImportExtension {
    pub fn suffix(self) -> &'static str {
        match self {
            ImportExtension::Js => ".js",
            ImportExtension::Ts => ".ts",
            ImportExtension::None => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnumStyle {
    /// A union of string literals.
    #[default]
    Union,
    /// An `as const` object paired with a union type of its values.
    Const,
}

#[derive(Debug, Clone)]
pub struct TypeScriptOptions {
    pub schema_name_case: NameCase,
    pub function_name_case: FunctionCase,
    /// Final identifiers per JSON Pointer, used as is (`typescript.naming.overrides`).
    pub name_overrides: BTreeMap<String, String>,
    pub layout: Layout,
    pub import_extension: ImportExtension,
    pub enum_style: EnumStyle,
    /// Emit `paths.ts` and `operations.ts` for generic clients.
    pub aggregate_maps: bool,
    /// Module name of the runtime library.
    pub client_module: String,
    /// Runtime library version range that generated code requires.
    pub client_version_range: String,
    /// Schemas for response validation (`typescript.validators`), emitted under `zod/`.
    pub validators: Option<Validators>,
    /// Also check the definition's constraints (lengths, bounds, `pattern`, counts, more formats) in
    /// the validation schemas (`typescript.validatorConstraints`). Off by default, so that
    /// response validation does not become stricter than the types.
    pub validator_constraints: bool,
    /// Query and mutation options for a data-fetching library (`typescript.query`), emitted under
    /// `query/`.
    pub query: Option<Query>,
    /// Pagination per operation id for infinite queries (`typescript.infiniteQueries`);
    /// wins over `x-tsurara-pagination` in the definition.
    pub infinite_queries: BTreeMap<String, tsurara_sem::operation::Pagination>,
    /// Mock factories and request handlers (`typescript.mocks`), emitted under `mocks/`.
    pub mocks: Option<Mocks>,
}

/// What `typescript.mocks` generates handlers for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mocks {
    /// MSW 2 and 3 request handlers (`HttpHandler`).
    Msw,
    /// Routes for `mockFetch` of `@tsurara/client/testing`.
    Fetch,
}

/// The library of `typescript.query`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    /// TanStack Query 5, through the types of `@tanstack/query-core` (every adapter accepts them).
    Tanstack,
}

/// The library of the validation schemas (`typescript.validators`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validators {
    /// Zod 4.
    Zod,
}

impl Default for TypeScriptOptions {
    fn default() -> Self {
        Self {
            schema_name_case: NameCase::Preserve,
            function_name_case: FunctionCase::Preserve,
            name_overrides: BTreeMap::new(),
            layout: Layout::Split,
            import_extension: ImportExtension::Js,
            enum_style: EnumStyle::Union,
            aggregate_maps: false,
            client_module: "@tsurara/client".to_owned(),
            client_version_range: crate::CLIENT_VERSION_RANGE.to_owned(),
            validators: None,
            validator_constraints: false,
            query: None,
            infinite_queries: BTreeMap::new(),
            mocks: None,
        }
    }
}

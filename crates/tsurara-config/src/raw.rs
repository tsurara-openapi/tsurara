//! Configuration file structures as written by the user (design.md chapter 10).
//!
//! These structs define the accepted keys and the JSON Schema in `schema/tsurara.config.schema.json`.
//! Every field is optional because entries are merged from the top level, `extends`, and `targets`
//! before required fields are checked.

use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;

/// An tsurara configuration file. Entries written at the top level are the defaults for every target.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(title = "tsurara configuration")]
#[allow(dead_code)] // Only used to generate the JSON Schema; files are validated per entry.
pub struct ConfigFile {
    /// JSON Schema of this file, for editors.
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    /// Base configuration files, resolved against this file. Objects are deep-merged and arrays are replaced.
    pub extends: Option<Extends>,
    /// Generation targets. Without `targets`, the top level is the only target.
    pub targets: Option<Vec<TargetConfig>>,
    #[serde(flatten)]
    pub defaults: TargetConfig,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum Extends {
    One(String),
    Many(Vec<String>),
}

/// Settings of one generation target.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetConfig {
    /// Name of the target, used by `tsurara generate --target`. Required when there are several targets.
    pub name: Option<String>,
    /// The definition: a path relative to this file, or a URL.
    pub input: Option<InputConfig>,
    /// Output directory, relative to this file.
    pub output: Option<String>,
    /// The language to generate (default `typescript`), whose options go in the section named after
    /// it; or a backend program as `{ command, options }` (design.md 18.11).
    pub language: Option<LanguageConfig>,
    /// Narrows the operations to generate.
    pub filter: Option<FilterConfig>,
    pub naming: Option<NamingConfig>,
    /// Emit `XWritable` types for schemas with readOnly/writeOnly (default true).
    pub split_visibility: Option<bool>,
    /// Treat wildcard media types (`*/*`, `application/*`) as JSON, for APIs that only return JSON
    /// (default false: parse by the actual Content-Type).
    pub wildcard_as_json: Option<bool>,
    /// Generate the operations (default true). With `false` the output holds the types of the
    /// schemas, webhooks and channels only, and needs only the types layer of the runtime: no HTTP
    /// client.
    pub operations: Option<bool>,
    /// Where generated code finds its runtime (default `package`): `vendor` writes a copy of the
    /// runtime's sources into the output and imports it instead of the package.
    pub runtime: Option<RuntimeConfig>,
    pub hooks: Option<HooksConfig>,
    /// Plugin modules (`@tsurara/plugin`): each runs in one process per run and takes part in the
    /// stages it declares. A path to the module, or `{ module, runtime }` with the command that runs
    /// it (default `[node]`). Paths are relative to the file they are written in.
    pub plugins: Option<Vec<PluginConfig>>,
    /// Options of the TypeScript backend (`language: typescript`).
    pub typescript: Option<TypeScriptConfig>,
    /// Options of the Python backend (`language: python`).
    pub python: Option<PythonConfig>,
    /// Options of the Rust backend (`language: rust`, experimental).
    pub rust: Option<RustConfig>,
    /// Options of the Go backend (`language: go`, experimental).
    pub go: Option<GoConfig>,
}

/// Where generated code finds its runtime.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeConfig {
    /// The package of the language's registry.
    Package,
    /// A copy of the runtime in the output.
    Vendor,
}

/// The `go` section (docs/languages/go.md).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoConfig {
    /// The package name of the generated files (default: the output directory's name, as a Go
    /// identifier).
    pub package: Option<String>,
    /// The import path of the runtime module (default `github.com/tsurara-openapi/tsurara/runtimes/go`).
    pub runtime_module: Option<String>,
    /// Replaces the emitted type per `format`: a type, with the import path of its package
    /// (`{ type: time.Time, import: { from: time } }`).
    pub type_mappings: Option<BTreeMap<String, TypeConfig>>,
    /// Replaces the emitted type per JSON Pointer (such as `#/components/schemas/Uuid`).
    pub overrides: Option<BTreeMap<String, TypeConfig>>,
    /// Emit mock values and routes for `tsuraratest` in files built with the tag `tsurara_mocks`
    /// (default false).
    pub mocks: Option<bool>,
}

/// The `rust` section (docs/languages/rust.md).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RustConfig {
    /// The path generated code uses for the runtime crate (default `tsurara_client`).
    pub runtime_crate: Option<String>,
    /// Emit mock values and routes for `MockTransport` in the module `mocks`, compiled for tests and
    /// with the crate feature `mocks` (default false).
    pub mocks: Option<bool>,
    /// Replaces the emitted type per `format`: a type path (`chrono::DateTime<chrono::Utc>`).
    pub type_mappings: Option<BTreeMap<String, TypeConfig>>,
    /// Replaces the emitted type per JSON Pointer (such as `#/components/schemas/Uuid`).
    pub overrides: Option<BTreeMap<String, TypeConfig>>,
}

/// The `python` section (design.md 18.10).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PythonConfig {
    /// Enums and discriminators reject values the definition does not list, instead of keeping them
    /// where values are received (default false).
    pub strict_enums: Option<bool>,
    /// Emit mock values and routes for `MockRouter` in the subpackage `mocks` (default false).
    pub mocks: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum LanguageConfig {
    Name(String),
    External(ExternalLanguageConfig),
}

/// A backend written as an external program. The command runs in the directory of this file.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalLanguageConfig {
    /// `[program, ...args]`. `describe` or `generate` is appended.
    pub command: Vec<String>,
    /// Passed to the program as `options`.
    pub options: Option<serde_json::Value>,
}

/// The `typescript` section (design.md 18.10).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptConfig {
    /// How generated code is split into files (default `split`).
    pub layout: Option<LayoutConfig>,
    /// Extension of relative imports in generated code (default `js`).
    pub import_extension: Option<ImportExtensionConfig>,
    /// How string enums are emitted (default `union`).
    pub enum_style: Option<EnumStyleConfig>,
    /// What happens to the body of an error response the definition gives no content (default
    /// `discard`; `read` reads a body that arrives with a Content-Type, as `unknown`).
    pub bodiless_responses: Option<BodilessResponsesConfig>,
    /// Emit schemas for response validation under `zod/` (default: none). Only `zod` (Zod 4).
    pub validators: Option<ValidatorsConfig>,
    /// Also check the definition's constraints (`minLength`, `maximum`, `pattern`, `minItems`, ...)
    /// and more formats (`uri`, `date`, ...) in the validation schemas (default false).
    pub validator_constraints: Option<bool>,
    /// Emit query and mutation options with typed query keys under `query/` (default: none). Only
    /// `tanstack` (TanStack Query 5).
    pub query: Option<QueryConfig>,
    /// Pagination per operation id for infinite query options (with `query: tanstack`); wins over
    /// `x-tsurara-pagination` in the definition.
    pub infinite_queries: Option<BTreeMap<String, InfiniteQueryConfig>>,
    /// Emit mock factories and request handlers under `mocks/` (default: none): `msw` for MSW
    /// handlers, `fetch` for routes of `mockFetch` (`@tsurara/client/testing`).
    pub mocks: Option<MocksConfig>,
    /// Emit `paths.ts` and `operations.ts` for generic clients (default false).
    pub aggregate_maps: Option<bool>,
    /// Module name of the runtime library (default `@tsurara/client`).
    pub client_module: Option<String>,
    /// Replaces the emitted type per `format`.
    pub type_mappings: Option<BTreeMap<String, TypeConfig>>,
    /// Replaces the emitted type per JSON Pointer (such as `#/components/schemas/Uuid`).
    pub overrides: Option<BTreeMap<String, TypeConfig>>,
    pub naming: Option<TypeScriptNamingConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeScriptNamingConfig {
    /// TypeScript identifiers per JSON Pointer, used as they are.
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
    /// How schema names are converted (default `preserve`).
    pub schema_case: Option<NameCaseConfig>,
    /// How function names are made from operation ids (default `preserve`).
    pub function_case: Option<FunctionCaseConfig>,
    /// Put before the names of schema types (`Api` gives `ApiPet`).
    pub type_prefix: Option<String>,
    /// How the keys of `enumStyle: const` objects are written (default `preserve`).
    pub enum_key_case: Option<EnumKeyCaseConfig>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InputConfig {
    Location(String),
    Detailed(InputObject),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputObject {
    /// A path relative to this file, or a URL.
    pub url: String,
    /// HTTP headers, only sent to the origin of `url`. `${VAR}` expands environment variables.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilterConfig {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub operation_ids: Vec<String>,
    /// Operations with one of these tags are left out.
    #[serde(default)]
    pub exclude_tags: Vec<String>,
    /// Operations with one of these `operationId`s are left out.
    #[serde(default)]
    pub exclude_operation_ids: Vec<String>,
}

/// A type of the target language, either as code or with the import it needs.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TypeConfig {
    Code(String),
    Detailed(TypeObject),
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeObject {
    /// The type expression.
    #[serde(rename = "type")]
    pub ty: String,
    pub import: Option<ImportConfig>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportConfig {
    /// Module specifier (TypeScript: paths starting with `.` are relative to this file; Go: the
    /// import path).
    pub from: String,
    /// Exported name. Defaults to `type`.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamingConfig {
    /// Preferred names per JSON Pointer (such as `#/components/schemas/Error`), for every language;
    /// each language's naming rules still apply.
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NameCaseConfig {
    Preserve,
    Pascal,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EnumKeyCaseConfig {
    Preserve,
    Constant,
    Pascal,
    Camel,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BodilessResponsesConfig {
    Discard,
    Read,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FunctionCaseConfig {
    Preserve,
    Camel,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LayoutConfig {
    Split,
    Tags,
    Single,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ImportExtensionConfig {
    Js,
    Ts,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ValidatorsConfig {
    Zod,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum QueryConfig {
    Tanstack,
}

/// How a read is paginated, for its infinite query options.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InfiniteQueryConfig {
    /// The parameter that carries the page: its `in`, a dot and its name (`query.cursor`).
    pub param: String,
    /// The value of the first page (default: the parameter is left out of the first request).
    pub initial: Option<serde_json::Value>,
    /// JSON Pointer to the next page's value in a success body (`/nextCursor`); absent or `null` there
    /// ends the query. Without it, the app gives `getNextPageParam`.
    pub next: Option<String>,
    /// JSON Pointer to the previous page's value.
    pub previous: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MocksConfig {
    Msw,
    Fetch,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EnumStyleConfig {
    Union,
    Const,
}

/// External command hooks (design.md chapter 11). Commands run in the directory of the file they are written in.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HooksConfig {
    /// Receives the root definition as JSON on stdin and returns the transformed definition on stdout.
    pub transform_document: Option<Commands>,
    /// Receives the model as JSON on stdin and returns the transformed model on stdout
    /// (design.md 18.4.6). The JSON form is versioned by `semVersion`.
    pub transform_model: Option<Commands>,
    /// Receives the model, the manifest of what the backend generated and the generated-file marker
    /// as JSON on stdin, and returns files to add (`[{ path, contents }]`) on stdout. Each added file
    /// must start with the marker.
    pub extend: Option<Commands>,
    /// Receives the output files as JSON on stdin and returns the transformed list on stdout.
    pub transform_files: Option<Commands>,
    /// Runs after writing. `{output}` is replaced with the output directory.
    pub after_write: Option<Commands>,
}

/// A plugin module.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PluginConfig {
    Module(String),
    WithRuntime {
        module: String,
        /// The command that runs the module, before the runner's arguments (default `[node]`).
        runtime: Option<Vec<String>>,
    },
}

/// One command or a list of commands, each written as `[program, ...args]`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Commands {
    One(Vec<String>),
    Many(Vec<Vec<String>>),
}

impl Commands {
    pub fn into_vec(self) -> Vec<Vec<String>> {
        match self {
            Commands::One(c) => vec![c],
            Commands::Many(cs) => cs,
        }
    }
}

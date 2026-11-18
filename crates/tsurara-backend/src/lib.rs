//! The interface between the model and the code generator of each language (design.md 18.3).
//! This crate depends on no language crate; `tsurara-cli` registers the built-in backends.
//!
//! Published for backends outside this repository, with one version for all tsurara crates,
//! released with the CLI and moving with `semVersion`. Stable: `Backend`, `Emitted`, `Output`,
//! `OutputFile`, `Markers`, `generate`, `manifest`, `plugin`, and the protocol types of `external`
//! and `session`.

pub mod external;
pub mod hoist;
pub mod manifest;
pub mod operation_def;
pub mod plugin;
pub mod session;

pub use manifest::Manifest;
use tsurara_sem::{Model, Options, Warning};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct OutputFile {
    /// Path relative to the output root (`/`-separated).
    pub path: String,
    pub contents: String,
}

/// Result of generation.
#[derive(Debug, Clone)]
pub struct Output {
    /// Output files sorted by path.
    pub files: Vec<OutputFile>,
    pub warnings: Vec<Warning>,
    /// What the backend generated, by name; `None` for external backends that do not return one.
    pub manifest: Option<Manifest>,
}

/// What `Backend::example` receives: the model, the manifest `emit` returned for it, and the sample
/// values of the operation (design.md 18.7.1).
#[derive(Debug, Clone, Copy)]
pub struct ExampleInput<'a> {
    pub model: &'a Model,
    pub opts: &'a Options,
    pub manifest: &'a Manifest,
    pub operation: &'a tsurara_sem::operation::Operation,
    pub samples: &'a tsurara_sample::OperationSamples,
}

/// The base URL call examples use: the definition's first server, with its variables at their
/// defaults, when that is an absolute URL; `https://api.example.com` (with the server's path, if
/// it is relative) otherwise.
pub fn example_base_url(model: &Model) -> String {
    let Some(server) = model.servers.first() else {
        return "https://api.example.com".into();
    };
    let mut url = server.url.clone();
    for v in &server.variables {
        url = url.replace(&format!("{{{}}}", v.name), &v.default);
    }
    if url.contains(['{', '"', '\\', '\'', '`']) || tsurara_sem::text::has_control(&url) {
        return "https://api.example.com".into();
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        url
    } else {
        format!(
            "https://api.example.com{}",
            if url.starts_with('/') {
                url
            } else {
                String::new()
            }
        )
    }
}

/// A call example: `code` compiles as the file `file` (relative to the output root) next to the
/// generated code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Example {
    pub file: String,
    pub code: String,
    /// How to use the file where placing it is not enough (a module declaration, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What a backend's `emit` returns.
#[derive(Debug, Clone)]
pub struct Emitted {
    pub files: Vec<OutputFile>,
    /// Warnings of the backend's own approximations; the model's are not included.
    pub warnings: Vec<Warning>,
    pub manifest: Option<Manifest>,
}

/// How the CLI recognizes a backend's files (design.md 18.8).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Markers {
    /// The first line of every generated file, in the language's comment syntax. The CLI deletes stale
    /// files that start with it and leaves other files alone.
    pub generated: String,
    /// Where generated code records the runtime library version range it requires.
    #[serde(default)]
    pub version: Option<VersionMarker>,
    /// Extensions of files that tools keep next to generated files (`.uid` for Godot). They are deleted
    /// together with a stale generated file.
    #[serde(default)]
    pub sidecars: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct VersionMarker {
    /// Output file that holds the line, relative to the output root.
    pub file: String,
    /// Prefix of the line; the rest of the line is the range.
    pub prefix: String,
}

impl Markers {
    /// The version range recorded in `files`, if the backend records one.
    pub fn recorded_version(&self, files: &[OutputFile]) -> Option<String> {
        let marker = self.version.as_ref()?;
        let file = files.iter().find(|f| f.path == marker.file)?;
        file.contents
            .lines()
            .find_map(|l| l.strip_prefix(marker.prefix.as_str()))
            .map(|r| r.trim().to_owned())
    }
}

/// A code generator for one language.
pub trait Backend {
    /// `typescript`, `python`, ... Used as the `language` value in the configuration.
    fn id(&self) -> &'static str;

    /// How the CLI recognizes this backend's files.
    fn markers(&self) -> Markers;

    /// Builds the output files from the model, with the warnings of the backend's own approximations
    /// and the manifest of what it generated.
    fn emit(&self, model: &Model, opts: &Options) -> Result<Emitted, Error>;

    /// A call example for one operation (`tsurara inspect operation`): a file that calls
    /// the generated function with the operation's sample values and handles its Result as the
    /// language does. `None` when the backend has no examples or the operation no function.
    fn example(&self, _input: &ExampleInput) -> Option<Example> {
        None
    }

    /// The options of the language as JSON, as plugins receive them (`ExtendInput::options`): what a
    /// plugin needs to write files that match the generated code, such as TypeScript's import
    /// extension. `null` by default.
    fn options_json(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sem(#[from] tsurara_sem::Error),
    /// A backend program failed or answered something that does not follow the protocol.
    #[error("{0}")]
    External(String),
    /// Options the backend cannot generate with.
    #[error("{0}")]
    Options(String),
    #[error("plugin `{name}`: {source}")]
    Plugin {
        name: String,
        #[source]
        source: plugin::PluginError,
    },
}

/// Builds the output files from the model with `backend`, together with the model's warnings.
pub fn generate_from_model(
    backend: &dyn Backend,
    model: &Model,
    opts: &Options,
) -> Result<Output, Error> {
    let Emitted {
        files,
        warnings: backend_warnings,
        manifest,
    } = backend.emit(model, opts)?;
    let mut warnings = model.warnings.clone();
    // Warnings of the backend's own approximations name the language (design.md 18.4.5).
    warnings.extend(backend_warnings.into_iter().map(|w| Warning {
        language: Some(backend.id().to_owned()),
        ..w
    }));
    warnings.sort();
    warnings.dedup();
    Ok(Output {
        files,
        warnings,
        manifest,
    })
}

/// Builds the model from a loaded definition and generates with `backend`.
pub fn generate(
    backend: &dyn Backend,
    set: &tsurara_loader::DocumentSet,
    opts: &Options,
) -> Result<Output, Error> {
    let model = tsurara_sem::build_model(set, opts)?;
    generate_from_model(backend, &model, opts)
}

pub use plugin::generate_with_plugins;

/// The JSON Schemas of what tsurara exchanges with programs outside it, by file name in `spec/`
///: the model (design.md 18.4.6), the manifest (11.1), and the messages of the plugin and
/// external backend protocols (11, 18.11).
#[cfg(feature = "schema")]
pub fn json_schemas() -> Vec<(&'static str, String)> {
    use schemars::generate::SchemaSettings;
    let render = |schema: schemars::Schema| {
        let mut text = serde_json::to_string_pretty(&schema).expect("schemas serialize");
        text.push('\n');
        text
    };
    let settings = || SchemaSettings::draft2020_12();
    let model = settings()
        .into_generator()
        .into_root_schema_for::<tsurara_sem::Model>();
    let manifest = settings()
        .into_generator()
        .into_root_schema_for::<Manifest>();

    // The protocol: one definition per message, and the root accepts any of them.
    let mut generator = settings().into_generator();
    let messages = [
        ("Describe", generator.subschema_for::<external::Describe>()),
        (
            "GenerateRequest",
            generator.subschema_for::<external::GenerateRequest>(),
        ),
        (
            "GenerateResponse",
            generator.subschema_for::<external::GenerateResponse>(),
        ),
        (
            "ExtendInput",
            generator.subschema_for::<plugin::ExtendInput>(),
        ),
        ("Files", generator.subschema_for::<Vec<OutputFile>>()),
        (
            "SessionRequest",
            generator.subschema_for::<session::SessionRequest>(),
        ),
        (
            "SessionResponse",
            generator.subschema_for::<session::SessionResponse>(),
        ),
    ];
    let mut defs = generator.take_definitions(true);
    // Messages that are not a definition of their own (a list of files) become one.
    let refs: Vec<serde_json::Value> = messages
        .into_iter()
        .map(|(name, schema)| {
            let value = schema.to_value();
            if value.get("$ref").is_some() {
                value
            } else {
                defs.insert(name.into(), value);
                serde_json::json!({ "$ref": format!("#/$defs/{name}") })
            }
        })
        .collect();
    let protocol = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "tsurara plugin and external backend protocol",
        "description": "`describe` answers with Describe; `generate` receives GenerateRequest and answers with GenerateResponse; the extend hook receives ExtendInput and answers with Files; transformFiles receives and answers with Files; transformModel receives and answers with the model (sem-model.schema.json). Plugin modules exchange SessionRequest and SessionResponse, one per line.",
        "anyOf": refs,
        "$defs": defs,
    });
    vec![
        ("sem-model.schema.json", render(model)),
        ("manifest.schema.json", render(manifest)),
        (
            "plugin-protocol.schema.json",
            format!(
                "{}\n",
                serde_json::to_string_pretty(&protocol).expect("serializes")
            ),
        ),
    ]
}

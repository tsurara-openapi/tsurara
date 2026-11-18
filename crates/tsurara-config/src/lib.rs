//! Loads tsurara configuration files and builds the generation options for each target (design.md chapter 10).
//!
//! Loading goes file by file: environment variables are expanded, keys are validated, and relative paths
//! are resolved against that file before it is merged with the files it `extends`. So a path written in a
//! base configuration stays relative to the base file.

mod paths;
pub mod raw;

use raw::{
    EnumStyleConfig, ImportExtensionConfig, InputConfig, LayoutConfig, NameCaseConfig,
    RuntimeConfig, TargetConfig, TypeConfig, ValidatorsConfig,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::options::{EnumStyle, ImportExtension, Layout, NameCase};
use tsurara_sem::ir::{External, Import};
use tsurara_sem::options::{Filter, Options, RuntimeSource};
use url::Url;

/// File names searched for by [`find`], in order.
pub const FILE_NAMES: &[&str] = &[
    "tsurara.config.yaml",
    "tsurara.config.yml",
    "tsurara.config.json",
    "tsurara.config.toml",
];

/// Keys of a configuration file that are not target settings.
const FILE_KEYS: &[&str] = &["$schema", "extends", "targets"];

const HOOK_KEYS: &[&str] = &[
    "transformDocument",
    "transformModel",
    "extend",
    "transformFiles",
    "afterWrite",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
    #[error("{}: {message}", path.display())]
    Invalid { path: PathBuf, message: String },
    #[error("{}: environment variable `{name}` is not set", path.display())]
    Env { path: PathBuf, name: String },
    #[error("{}: `extends` forms a cycle", path.display())]
    ExtendsCycle { path: PathBuf },
    #[error("{}: invalid input `{input}`: {source}", path.display())]
    Input {
        path: PathBuf,
        input: String,
        #[source]
        source: tsurara_loader::LocationError,
    },
    #[error("target `{target}`: `{field}` is required")]
    Missing { target: String, field: &'static str },
    #[error("targets[{index}]: `name` is required when there are several targets")]
    UnnamedTarget { index: usize },
    #[error("target name `{0}` is used more than once")]
    DuplicateTarget(String),
    #[error(
        "target `{target}`: `{key}` is mapped to `{ty}`, but JSON values are never `{ty}` at runtime; \
         keep the default type or use a type that does not change the runtime value (design.md 10.4)"
    )]
    TypeMapping {
        target: String,
        key: String,
        ty: String,
    },
    /// An external type in `typeMappings` or `overrides` that is not a type expression of the
    /// restricted grammar.
    #[error("target `{target}`: the type for `{key}` cannot be used: {reason}")]
    InvalidType {
        target: String,
        key: String,
        reason: String,
    },
}

/// A loaded configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Absolute path of the configuration file.
    pub path: PathBuf,
    pub targets: Vec<Target>,
}

impl Config {
    pub fn target(&self, name: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.name == name)
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub input: Input,
    /// Absolute output directory.
    pub output: PathBuf,
    /// Options common to every language.
    pub options: Options,
    pub language: Language,
    pub hooks: Hooks,
    /// Plugin modules, in order.
    pub plugins: Vec<PluginModule>,
}

/// A plugin module (`plugins`), resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginModule {
    /// Absolute path of the module.
    pub module: PathBuf,
    /// The command that runs it (default `[node]`).
    pub runtime: Vec<String>,
    /// The directory of the file it is written in; the process runs there.
    pub dir: PathBuf,
}

/// The language a target generates, with that language's options.
#[derive(Debug, Clone)]
pub enum Language {
    TypeScript(TypeScriptOptions),
    Python(tsurara_lang_python::PythonOptions),
    /// Experimental.
    Rust(tsurara_lang_rust::RustOptions),
    /// Experimental.
    Go(tsurara_lang_go::GoOptions),
    /// A backend program (design.md 18.11), with the `options` passed to it.
    External {
        command: Command,
        options: Value,
    },
}

impl Language {
    /// The `language` value of the configuration.
    pub fn id(&self) -> &'static str {
        match self {
            Language::TypeScript(_) => "typescript",
            Language::Python(_) => "python",
            Language::Rust(_) => "rust",
            Language::Go(_) => "go",
            Language::External { .. } => "external",
        }
    }
}

/// Languages that can be selected with `language`.
pub const LANGUAGES: &[&str] = &["typescript", "python", "rust", "go"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// `file://` or `http(s)://` URL of the root definition.
    pub url: Url,
    /// HTTP headers sent to the origin of `url`.
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hooks {
    pub transform_document: Vec<Command>,
    pub transform_model: Vec<Command>,
    pub extend: Vec<Command>,
    pub transform_files: Vec<Command>,
    pub after_write: Vec<Command>,
}

/// An external command. `args[0]` is the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub args: Vec<String>,
    /// Directory of the configuration file the command is written in; the command runs there.
    pub dir: PathBuf,
}

/// Finds a configuration file in `dir` or its ancestors.
pub fn find(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .flat_map(|d| FILE_NAMES.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// Loads a configuration file, expanding `${VAR}` from the process environment.
pub fn load(path: &Path) -> Result<Config, Error> {
    load_with_env(path, &|name| std::env::var(name).ok())
}

/// Loads a configuration file, expanding `${VAR}` with `env`.
pub fn load_with_env(path: &Path, env: &dyn Fn(&str) -> Option<String>) -> Result<Config, Error> {
    let path = absolute(path);
    let mut loader = Loader {
        env,
        stack: Vec::new(),
    };
    let mut merged = loader.load_file(&path)?;
    let dir = parent(&path);
    let targets = match merged.remove("targets") {
        Some(Value::Array(ts)) => ts,
        _ => vec![Value::Object(Map::new())],
    };
    let single = targets.len() == 1;
    let mut built = Vec::new();
    for (index, t) in targets.into_iter().enumerate() {
        let mut entry = Value::Object(merged.clone());
        merge(&mut entry, t);
        let Value::Object(entry) = entry else {
            unreachable!()
        };
        let target = build_target(&path, &dir, entry, index, single)?;
        if built.iter().any(|b: &Target| b.name == target.name) {
            return Err(Error::DuplicateTarget(target.name));
        }
        built.push(target);
    }
    Ok(Config {
        path,
        targets: built,
    })
}

struct Loader<'a> {
    env: &'a dyn Fn(&str) -> Option<String>,
    /// Files being loaded, for detecting `extends` cycles.
    stack: Vec<PathBuf>,
}

impl Loader<'_> {
    /// Loads one file and the files it extends, and returns the merged contents with paths resolved.
    fn load_file(&mut self, path: &Path) -> Result<Map<String, Value>, Error> {
        if self.stack.iter().any(|p| p == path) {
            return Err(Error::ExtendsCycle {
                path: path.to_owned(),
            });
        }
        let mut own = read_file(path)?;
        expand_env(&mut own, self.env).map_err(|name| Error::Env {
            path: path.to_owned(),
            name,
        })?;
        let Value::Object(mut own) = own else {
            return Err(invalid(path, "a configuration file must be an object"));
        };
        validate(path, &own)?;
        let dir = parent(path);
        resolve_entry(path, &dir, &mut own)?;
        if let Some(Value::Array(targets)) = own.get_mut("targets") {
            for t in targets {
                if let Value::Object(t) = t {
                    resolve_entry(path, &dir, t)?;
                }
            }
        }
        let bases = match own.remove("extends") {
            Some(Value::String(s)) => vec![s],
            Some(Value::Array(a)) => a
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        };
        own.remove("$schema");
        self.stack.push(path.to_owned());
        let mut merged = Value::Object(Map::new());
        for base in bases {
            let base = paths::resolve(&dir, &base);
            merge(&mut merged, Value::Object(self.load_file(&base)?));
        }
        self.stack.pop();
        merge(&mut merged, Value::Object(own));
        let Value::Object(merged) = merged else {
            unreachable!()
        };
        Ok(merged)
    }
}

fn read_file(path: &Path) -> Result<Value, Error> {
    let bytes = std::fs::read(path).map_err(|source| Error::Read {
        path: path.to_owned(),
        source,
    })?;
    let parse_error = |message: String| Error::Parse {
        path: path.to_owned(),
        message,
    };
    if path.extension().is_some_and(|e| e == "toml") {
        let text = std::str::from_utf8(&bytes).map_err(|e| parse_error(e.to_string()))?;
        toml::from_str(text).map_err(|e| parse_error(e.to_string()))
    } else {
        tsurara_loader::parse_document(&bytes).map_err(|e| parse_error(e.to_string()))
    }
}

/// Checks the keys of a file before merging, so that errors point at the file they are written in.
fn validate(path: &Path, file: &Map<String, Value>) -> Result<(), Error> {
    match file.get("extends") {
        None | Some(Value::String(_)) => {}
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => {}
        Some(_) => return Err(invalid(path, "`extends` must be a path or a list of paths")),
    }
    let defaults: Map<String, Value> = file
        .iter()
        .filter(|(k, _)| !FILE_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    check_entry(path, "the top level", Value::Object(defaults))?;
    match file.get("targets") {
        None => {}
        Some(Value::Array(targets)) => {
            for (i, t) in targets.iter().enumerate() {
                check_entry(path, &format!("targets[{i}]"), t.clone())?;
            }
        }
        Some(_) => return Err(invalid(path, "`targets` must be a list")),
    }
    Ok(())
}

fn check_entry(path: &Path, what: &str, entry: Value) -> Result<(), Error> {
    // Language sections first, so that an error names the section it is in.
    fn section<T: serde::de::DeserializeOwned>(
        path: &Path,
        what: &str,
        key: &str,
        entry: &Value,
    ) -> Result<(), Error> {
        match entry.get(key) {
            Some(v) => serde_json::from_value::<T>(v.clone())
                .map(drop)
                .map_err(|e| invalid(path, format!("{what}: {key}: {e}"))),
            None => Ok(()),
        }
    }
    section::<raw::TypeScriptConfig>(path, what, "typescript", &entry)?;
    section::<raw::PythonConfig>(path, what, "python", &entry)?;
    section::<raw::RustConfig>(path, what, "rust", &entry)?;
    section::<raw::GoConfig>(path, what, "go", &entry)?;
    serde_json::from_value::<TargetConfig>(entry)
        .map(drop)
        .map_err(|e| invalid(path, format!("{what}: {e}")))
}

/// Resolves the relative paths of one entry (the top level or a target) against `dir`.
fn resolve_entry(path: &Path, dir: &Path, entry: &mut Map<String, Value>) -> Result<(), Error> {
    let input_url = match entry.get_mut("input") {
        Some(Value::Object(o)) => o.get_mut("url"),
        other => other,
    };
    if let Some(Value::String(s)) = input_url {
        let url = tsurara_loader::resolve_input(s, dir).map_err(|source| Error::Input {
            path: path.to_owned(),
            input: s.clone(),
            source,
        })?;
        *s = url.to_string();
    }
    if let Some(Value::String(s)) = entry.get_mut("output") {
        *s = paths::resolve(dir, s).to_string_lossy().into_owned();
    }
    for key in ["typeMappings", "overrides"] {
        let Some(Value::Object(types)) = entry.get_mut("typescript").and_then(|t| t.get_mut(key))
        else {
            continue;
        };
        for ty in types.values_mut() {
            if let Some(Value::String(from)) = ty.pointer_mut("/import/from") {
                if from.starts_with('.') {
                    // Kept as a file URL until the output directory is known.
                    let file = paths::resolve(dir, from);
                    if let Ok(url) = Url::from_file_path(&file) {
                        *from = url.to_string();
                    }
                }
            }
        }
    }
    // An external backend's command runs in the directory of the file it is written in, like hooks.
    if let Some(Value::Object(lang)) = entry.get_mut("language") {
        lang.insert("dir".to_owned(), serde_json::json!(dir));
    }
    if let Some(plugins) = entry.get_mut("plugins") {
        let list = serde_json::from_value::<Vec<raw::PluginConfig>>(plugins.clone())
            .map_err(|e| invalid(path, format!("plugins: {e}")))?;
        *plugins = Value::Array(
            list.into_iter()
                .map(|p| {
                    let (module, runtime) = match p {
                        raw::PluginConfig::Module(m) => (m, None),
                        raw::PluginConfig::WithRuntime { module, runtime } => (module, runtime),
                    };
                    serde_json::json!({
                        "module": paths::resolve(dir, &module),
                        "runtime": runtime.unwrap_or_else(|| vec!["node".to_owned()]),
                        "dir": dir,
                    })
                })
                .collect(),
        );
    }
    if let Some(Value::Object(hooks)) = entry.get_mut("hooks") {
        for key in HOOK_KEYS {
            if let Some(commands) = hooks.remove(*key) {
                let commands = serde_json::from_value::<raw::Commands>(commands)
                    .map_err(|e| invalid(path, format!("hooks.{key}: {e}")))?
                    .into_vec();
                let resolved = serde_json::json!({ "dir": dir, "commands": commands });
                hooks.insert((*key).to_owned(), resolved);
            }
        }
    }
    Ok(())
}

fn build_target(
    path: &Path,
    dir: &Path,
    mut entry: Map<String, Value>,
    index: usize,
    single: bool,
) -> Result<Target, Error> {
    let hooks = entry.remove("hooks");
    let plugins = build_plugins(path, entry.remove("plugins"))?;
    let language_entry = entry.remove("language");
    let cfg: TargetConfig =
        serde_json::from_value(Value::Object(entry)).map_err(|e| invalid(path, e.to_string()))?;
    let common = common_options(&cfg);
    let name = match cfg.name {
        Some(n) => n,
        None if single => "default".to_owned(),
        None => return Err(Error::UnnamedTarget { index }),
    };
    let missing = |field| Error::Missing {
        target: name.clone(),
        field,
    };
    let input = match cfg.input.ok_or_else(|| missing("input"))? {
        InputConfig::Location(url) => (url, BTreeMap::new()),
        InputConfig::Detailed(o) => (o.url, o.headers),
    };
    let input = Input {
        // Already resolved to a URL by `resolve_entry`.
        url: Url::parse(&input.0).map_err(|e| invalid(path, format!("input: {e}")))?,
        headers: input.1.into_iter().collect(),
    };
    let output = PathBuf::from(cfg.output.ok_or_else(|| missing("output"))?);

    let mut opts = Options::default();
    let mut ts = TypeScriptOptions::default();
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ExternalEntry {
        command: Vec<String>,
        #[serde(default)]
        options: Value,
        dir: PathBuf,
    }
    let language = match language_entry {
        None => "typescript".to_owned(),
        Some(Value::String(s)) => s,
        Some(v @ Value::Object(_)) => {
            let e: ExternalEntry =
                serde_json::from_value(v).map_err(|e| invalid(path, format!("language: {e}")))?;
            if e.command.is_empty() {
                return Err(invalid(
                    path,
                    "language.command: the command is empty".to_owned(),
                ));
            }
            let options = Options {
                external_import_base: paths::relative(&output, dir),
                ..common
            };
            return Ok(Target {
                name,
                input,
                output,
                options,
                language: Language::External {
                    command: Command {
                        args: e.command,
                        dir: e.dir,
                    },
                    options: e.options,
                },
                hooks: build_hooks(path, hooks)?,
                plugins,
            });
        }
        Some(other) => {
            return Err(invalid(
                path,
                format!("language: expected a name or {{ command, options }}, found {other}"),
            ));
        }
    };
    if !LANGUAGES.contains(&language.as_str()) {
        return Err(invalid(
            path,
            format!(
                "language: unknown language `{language}` (available: {})",
                LANGUAGES.join(", ")
            ),
        ));
    }
    // A section for a language other than the selected one is a mistake.
    for (section, present) in [
        ("typescript", cfg.typescript.is_some()),
        ("python", cfg.python.is_some()),
        ("rust", cfg.rust.is_some()),
        ("go", cfg.go.is_some()),
    ] {
        if present && section != language {
            return Err(invalid(
                path,
                format!("{section}: this target generates `{language}`, not `{section}`"),
            ));
        }
    }
    if language == "go" {
        let go = cfg.go.unwrap_or_default();
        let check = |e: &External| tsurara_lang_go::external::check(e);
        let mut common = common;
        common.type_mappings = language_types(&name, "go", go.type_mappings, &output, &check)?;
        common.type_overrides = language_types(&name, "go", go.overrides, &output, &check)?;
        let defaults = tsurara_lang_go::GoOptions::default();
        let package = match go.package {
            Some(p) => p,
            None => package_name(&output),
        };
        // A vendored runtime is a package of the user's module.
        let runtime_module = match (go.runtime_module, common.runtime) {
            (Some(m), _) => m,
            (None, RuntimeSource::Vendor) => {
                vendored_go_package(&output).ok_or_else(|| Error::Invalid {
                    path: path.to_owned(),
                    message: format!(
                        "target `{name}`: `runtime: vendor` needs a go.mod in {} or above it, or `go.runtimeModule`",
                        output.display()
                    ),
                })?
            }
            (None, RuntimeSource::Package) => defaults.runtime_module,
        };
        return Ok(Target {
            name,
            input,
            options: Options {
                external_import_base: paths::relative(&output, dir),
                ..common
            },
            output,
            language: Language::Go(tsurara_lang_go::GoOptions {
                package,
                runtime_module,
                mocks: go.mocks.unwrap_or(false),
            }),
            hooks: build_hooks(path, hooks)?,
            plugins,
        });
    }
    if language == "rust" {
        let rs = cfg.rust.unwrap_or_default();
        let check = |e: &External| tsurara_lang_rust::external::check(&e.code);
        let mut common = common;
        common.type_mappings = language_types(&name, "rust", rs.type_mappings, &output, &check)?;
        common.type_overrides = language_types(&name, "rust", rs.overrides, &output, &check)?;
        return Ok(Target {
            name,
            input,
            options: Options {
                external_import_base: paths::relative(&output, dir),
                ..common
            },
            output,
            language: Language::Rust(tsurara_lang_rust::RustOptions {
                runtime_crate: rs
                    .runtime_crate
                    .unwrap_or_else(|| tsurara_lang_rust::RustOptions::default().runtime_crate),
                mocks: rs.mocks.unwrap_or(false),
            }),
            hooks: build_hooks(path, hooks)?,
            plugins,
        });
    }
    if language == "python" {
        let py = cfg.python.unwrap_or_default();
        return Ok(Target {
            name,
            input,
            options: Options {
                external_import_base: paths::relative(&output, dir),
                ..common
            },
            output,
            language: Language::Python(tsurara_lang_python::PythonOptions {
                strict_enums: py.strict_enums.unwrap_or(false),
                mocks: py.mocks.unwrap_or(false),
            }),
            hooks: build_hooks(path, hooks)?,
            plugins,
        });
    }
    let tsc = cfg.typescript.unwrap_or_default();
    let externals =
        |types: Option<BTreeMap<String, TypeConfig>>| -> Result<BTreeMap<String, External>, Error> {
            types
                .unwrap_or_default()
                .into_iter()
                .map(|(key, ty)| {
                    let ext = external(ty, &output);
                    if let Err(reason) = tsurara_lang_typescript::external::check(&ext) {
                        return Err(Error::InvalidType {
                            target: name.clone(),
                            key,
                            reason,
                        });
                    }
                    if contradicts_runtime(&ext.code) {
                        return Err(Error::TypeMapping {
                            target: name.clone(),
                            key,
                            ty: ext.code,
                        });
                    }
                    Ok((key, ext))
                })
                .collect()
        };
    let per_language = |m: BTreeMap<String, External>| {
        m.into_iter()
            .map(|(k, ext)| (k, BTreeMap::from([("typescript".to_owned(), ext)])))
            .collect()
    };
    opts.type_mappings = per_language(externals(tsc.type_mappings)?);
    opts.type_overrides = per_language(externals(tsc.overrides)?);
    opts.external_import_base = paths::relative(&output, dir);
    if let Some(naming) = cfg.naming {
        opts.name_overrides = naming.overrides;
    }
    if let Some(naming) = tsc.naming {
        ts.name_overrides = naming.overrides;
        if let Some(case) = naming.schema_case {
            ts.schema_name_case = match case {
                NameCaseConfig::Preserve => NameCase::Preserve,
                NameCaseConfig::Pascal => NameCase::Pascal,
            };
        }
    }
    if let Some(filter) = cfg.filter {
        opts.filter = Filter {
            tags: filter.tags,
            operation_ids: filter.operation_ids,
            exclude_tags: filter.exclude_tags,
            exclude_operation_ids: filter.exclude_operation_ids,
        };
    }
    if let Some(b) = cfg.split_visibility {
        opts.split_visibility = b;
    }
    if let Some(b) = cfg.wildcard_as_json {
        opts.wildcard_as_json = b;
    }
    if let Some(b) = cfg.operations {
        opts.operations = b;
    }
    if let Some(r) = cfg.runtime {
        opts.runtime = match r {
            RuntimeConfig::Package => RuntimeSource::Package,
            RuntimeConfig::Vendor => RuntimeSource::Vendor,
        };
    }
    if let Some(layout) = tsc.layout {
        ts.layout = match layout {
            LayoutConfig::Split => Layout::Split,
            LayoutConfig::Tags => Layout::Tags,
            LayoutConfig::Single => Layout::Single,
        };
    }
    if let Some(ext) = tsc.import_extension {
        ts.import_extension = match ext {
            ImportExtensionConfig::Js => ImportExtension::Js,
            ImportExtensionConfig::Ts => ImportExtension::Ts,
            ImportExtensionConfig::None => ImportExtension::None,
        };
    }
    if let Some(style) = tsc.enum_style {
        ts.enum_style = match style {
            EnumStyleConfig::Union => EnumStyle::Union,
            EnumStyleConfig::Const => EnumStyle::Const,
        };
    }
    if let Some(b) = tsc.aggregate_maps {
        ts.aggregate_maps = b;
    }
    if let Some(v) = tsc.validators {
        ts.validators = Some(match v {
            ValidatorsConfig::Zod => tsurara_lang_typescript::options::Validators::Zod,
        });
    }
    if let Some(m) = tsc.mocks {
        ts.mocks = Some(match m {
            raw::MocksConfig::Msw => tsurara_lang_typescript::options::Mocks::Msw,
            raw::MocksConfig::Fetch => tsurara_lang_typescript::options::Mocks::Fetch,
        });
    }
    if let Some(v) = tsc.validator_constraints {
        ts.validator_constraints = v;
    }
    if let Some(raw::QueryConfig::Tanstack) = tsc.query {
        ts.query = Some(tsurara_lang_typescript::options::Query::Tanstack);
    }
    for (id, p) in tsc.infinite_queries.into_iter().flatten() {
        ts.infinite_queries.insert(
            id,
            tsurara_sem::operation::Pagination {
                param: p.param,
                initial: p.initial,
                next: p.next,
                previous: p.previous,
            },
        );
    }
    if let Some(m) = tsc.client_module {
        ts.client_module = m;
    }
    Ok(Target {
        name,
        input,
        output,
        options: opts,
        language: Language::TypeScript(ts),
        hooks: build_hooks(path, hooks)?,
        plugins,
    })
}

fn build_plugins(path: &Path, plugins: Option<Value>) -> Result<Vec<PluginModule>, Error> {
    #[derive(serde::Deserialize)]
    struct Resolved {
        module: PathBuf,
        runtime: Vec<String>,
        dir: PathBuf,
    }
    let Some(plugins) = plugins else {
        return Ok(Vec::new());
    };
    let list: Vec<Resolved> =
        serde_json::from_value(plugins).map_err(|e| invalid(path, format!("plugins: {e}")))?;
    list.into_iter()
        .map(|p| {
            if p.runtime.is_empty() {
                return Err(invalid(path, "plugins: `runtime` must not be empty"));
            }
            Ok(PluginModule {
                module: p.module,
                runtime: p.runtime,
                dir: p.dir,
            })
        })
        .collect()
}

fn build_hooks(path: &Path, hooks: Option<Value>) -> Result<Hooks, Error> {
    #[derive(serde::Deserialize)]
    struct Resolved {
        dir: PathBuf,
        commands: Vec<Vec<String>>,
    }
    let mut out = Hooks::default();
    let Some(Value::Object(mut hooks)) = hooks else {
        return Ok(out);
    };
    for (key, slot) in HOOK_KEYS.iter().zip([
        &mut out.transform_document,
        &mut out.transform_model,
        &mut out.extend,
        &mut out.transform_files,
        &mut out.after_write,
    ]) {
        if let Some(v) = hooks.remove(*key) {
            let r: Resolved = serde_json::from_value(v)
                .map_err(|e| invalid(path, format!("hooks.{key}: {e}")))?;
            *slot = r
                .commands
                .into_iter()
                .filter(|c| !c.is_empty())
                .map(|args| Command {
                    args,
                    dir: r.dir.clone(),
                })
                .collect();
        }
    }
    Ok(out)
}

/// The common options of a target that do not depend on a language section.
fn common_options(cfg: &TargetConfig) -> Options {
    let mut opts = Options::default();
    if let Some(naming) = &cfg.naming {
        opts.name_overrides = naming.overrides.clone();
    }
    if let Some(filter) = &cfg.filter {
        opts.filter = Filter {
            tags: filter.tags.clone(),
            operation_ids: filter.operation_ids.clone(),
            exclude_tags: filter.exclude_tags.clone(),
            exclude_operation_ids: filter.exclude_operation_ids.clone(),
        };
    }
    if let Some(b) = cfg.split_visibility {
        opts.split_visibility = b;
    }
    if let Some(b) = cfg.wildcard_as_json {
        opts.wildcard_as_json = b;
    }
    if let Some(b) = cfg.operations {
        opts.operations = b;
    }
    if let Some(r) = cfg.runtime {
        opts.runtime = match r {
            RuntimeConfig::Package => RuntimeSource::Package,
            RuntimeConfig::Vendor => RuntimeSource::Vendor,
        };
    }
    opts
}

/// Builds the IR type. Relative imports are rewritten relative to the output root.
/// `typeMappings` or `overrides` of a language section, checked by the language and keyed by it.
fn language_types(
    target: &str,
    language: &str,
    types: Option<BTreeMap<String, TypeConfig>>,
    output: &Path,
    check: &dyn Fn(&External) -> Result<(), String>,
) -> Result<BTreeMap<String, BTreeMap<String, External>>, Error> {
    types
        .unwrap_or_default()
        .into_iter()
        .map(|(key, ty)| {
            let ext = external(ty, output);
            check(&ext).map_err(|reason| Error::InvalidType {
                target: target.to_owned(),
                key: key.clone(),
                reason,
            })?;
            Ok((key, BTreeMap::from([(language.to_owned(), ext)])))
        })
        .collect()
}

fn external(ty: TypeConfig, output: &Path) -> External {
    match ty {
        TypeConfig::Code(code) => External { code, import: None },
        TypeConfig::Detailed(o) => {
            let import = o.import.map(|i| {
                let from = match Url::parse(&i.from).ok().and_then(|u| u.to_file_path().ok()) {
                    Some(file) if i.from.starts_with("file:") => {
                        paths::relative_specifier(output, &file)
                    }
                    _ => i.from,
                };
                Import {
                    from,
                    name: i.name.unwrap_or_else(|| o.ty.clone()),
                }
            });
            External { code: o.ty, import }
        }
    }
}

/// Whether a mapped type claims a runtime value that JSON parsing never produces (design.md 10.4).
fn contradicts_runtime(code: &str) -> bool {
    code.split('|')
        .map(str::trim)
        .any(|t| t == "Date" || t == "bigint")
}

/// Deep-merges `over` into `base`. Objects are merged; arrays and other values are replaced.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

/// Expands `${VAR}` in every string value. Returns the name of the first unset variable.
fn expand_env(v: &mut Value, env: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
    match v {
        Value::String(s) if s.contains("${") => {
            *s = expand(s, env)?;
            Ok(())
        }
        Value::Array(a) => a.iter_mut().try_for_each(|v| expand_env(v, env)),
        Value::Object(o) => o.values_mut().try_for_each(|v| expand_env(v, env)),
        _ => Ok(()),
    }
}

fn expand(s: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let name = after
            .find('}')
            .map(|end| &after[..end])
            .filter(|n| is_var_name(n));
        match name {
            Some(name) => {
                out.push_str(&env(name).ok_or_else(|| name.to_owned())?);
                rest = &after[name.len() + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

fn is_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn invalid(path: &Path, message: impl Into<String>) -> Error {
    Error::Invalid {
        path: path.to_owned(),
        message: message.into(),
    }
}

fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_owned())
    };
    paths::normalize(&joined)
}

fn parent(path: &Path) -> PathBuf {
    path.parent().map(Path::to_owned).unwrap_or_default()
}

/// JSON Schema of the configuration file, as shipped in `schema/tsurara.config.schema.json`.
pub fn json_schema() -> String {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(raw::ConfigFile)).expect("schema serializes");
    // `flatten` cannot carry `deny_unknown_fields`, so close the top level here.
    if let Value::Object(o) = &mut schema {
        o.insert("additionalProperties".to_owned(), Value::Bool(false));
    }
    let mut s = serde_json::to_string_pretty(&schema).expect("schema serializes");
    s.push('\n');
    s
}

/// Go keywords, which cannot name a package.
const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

/// A Go package name from a directory name: lower-case letters and digits (`pet-store` →
/// `petstore`), with `api` appended to a keyword (`go` → `goapi`), and `api` when nothing is left.
fn package_name(output: &std::path::Path) -> String {
    let base = output
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name: String = base
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    match name.chars().next() {
        Some(_) if GO_KEYWORDS.contains(&name.as_str()) => format!("{name}api"),
        Some(c) if c.is_ascii_lowercase() => name,
        _ => "api".into(),
    }
}

/// The import path of a vendored Go runtime in `output` (`<module>/<output>/internal/tsurara`), from
/// the nearest go.mod at or above it.
fn vendored_go_package(output: &Path) -> Option<String> {
    let mut dir = Some(output);
    let mut rest: Vec<String> = Vec::new();
    while let Some(d) = dir {
        if let Ok(text) = std::fs::read_to_string(d.join("go.mod")) {
            let module = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("module "))?
                .trim()
                .trim_matches('"')
                .to_owned();
            rest.reverse();
            let mut parts = vec![module];
            parts.extend(rest);
            parts.push("internal/tsurara".to_owned());
            return Some(parts.join("/"));
        }
        if let Some(name) = d.file_name() {
            rest.push(name.to_string_lossy().into_owned());
        }
        dir = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_are_not_go_keywords() {
        let name = |dir: &str| package_name(std::path::Path::new(dir));
        assert_eq!(name("out/pet-store"), "petstore");
        assert_eq!(name("out/go"), "goapi");
        assert_eq!(name("out/type"), "typeapi");
        assert_eq!(name("out/123"), "api");
    }

    #[test]
    fn expands_variables() {
        let env = |n: &str| (n == "TOKEN").then(|| "abc".to_owned());
        assert_eq!(expand("Bearer ${TOKEN}", &env).unwrap(), "Bearer abc");
        assert_eq!(expand("${TOKEN}-${TOKEN}", &env).unwrap(), "abc-abc");
        assert_eq!(
            expand("a ${not a var} ${", &env).unwrap(),
            "a ${not a var} ${"
        );
        assert_eq!(expand("${MISSING}", &env).unwrap_err(), "MISSING");
    }

    #[test]
    fn merges_objects_and_replaces_arrays() {
        let mut base = serde_json::json!({ "a": { "x": 1, "y": [1, 2] }, "b": 1 });
        merge(&mut base, serde_json::json!({ "a": { "y": [3] }, "c": 2 }));
        assert_eq!(
            base,
            serde_json::json!({ "a": { "x": 1, "y": [3] }, "b": 1, "c": 2 })
        );
    }

    #[test]
    fn rejects_runtime_mismatches() {
        assert!(contradicts_runtime("Date"));
        assert!(contradicts_runtime("bigint | null"));
        assert!(!contradicts_runtime(
            "string & { readonly __brand: 'DateTime' }"
        ));
    }
}

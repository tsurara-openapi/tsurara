//! The `tsurara` command (design.md chapter 12).

mod client_version;
mod external;
mod hooks;
mod inspect;
mod mcp;
mod plugins;
mod test_backend;
mod write;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use tsurara_cli::output::{
    CheckReport, CheckedTarget, DifferenceReport, GenerateReport, GeneratedTarget, WarningReport,
};
use tsurara_config::{Language, Target};
use tsurara_loader::{DefaultFetcher, DocumentSet, HttpOptions};
use tsurara_sem::Warning;

/// How a command prints its result.
#[derive(Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Format {
    /// Lines for people (the default).
    #[default]
    Text,
    /// One JSON object on stdout (spec/cli-output.schema.json for `generate` and `check`).
    Json,
}

#[derive(Parser)]
#[command(
    name = "tsurara",
    version,
    about = "Generates TypeScript types and API call functions from OpenAPI"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate and write files for each target.
    Generate {
        /// Configuration file. Searched for in the current directory and its ancestors by default.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Only generate this target.
        #[arg(long)]
        target: Option<String>,
        /// Treat warnings as errors. Nothing is written when there are warnings.
        #[arg(long)]
        strict: bool,
        /// Output format.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },
    /// Check that the generated files on disk match the definition, without writing anything.
    ///
    /// Exits with code 1 when a generated file was edited by hand, was not regenerated after the
    /// definition changed, or is stale, and when the installed runtime library does not satisfy the
    /// version range the generated code requires. It does not detect drift between the server's actual
    /// responses and the definition; use contract tests on the server for that.
    Check {
        /// Configuration file. Searched for in the current directory and its ancestors by default.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Only check this target.
        #[arg(long)]
        target: Option<String>,
        /// Output format.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },
    /// Answer questions about the definition and the generated code, without writing files: the
    /// operations, one operation with a call example, a schema, or what a warning means.
    Inspect {
        #[command(subcommand)]
        what: inspect::What,
    },
    /// Run a Model Context Protocol server over stdio for coding agents: tools that search and
    /// describe operations (with call examples) and schemas, generate and check, and the guides as
    /// resources.
    Mcp,
    /// Test a backend written outside tsurara: generate the edge-case corpus (and the definitions
    /// in `--definitions`) with it, and run its checker on each output. Exits with code 1 when a
    /// definition fails; its directory is kept for reproduction.
    TestBackend {
        /// The backend's command line (`node backend.mjs`), as in `language: { command }`. Arguments
        /// naming existing files are made absolute.
        #[arg(long)]
        backend: String,
        /// The checker's command line (`go vet ./...`), run in each output directory, with
        /// `TSURARA_OUTPUT` and `TSURARA_DEFINITION` set. Without it, only generation is tested.
        #[arg(long)]
        check: Option<String>,
        /// The backend's `options`, as JSON.
        #[arg(long)]
        options: Option<String>,
        /// More definitions: every `.yaml`, `.yml` and `.json` file of this directory (such as the
        /// real-world definitions `pnpm corpus:fetch` downloads). Repeatable.
        #[arg(long)]
        definitions: Vec<PathBuf>,
        /// Keep every output in this directory instead of a temporary one.
        #[arg(long)]
        keep: Option<PathBuf>,
    },
    /// Write the runtime contract (`spec/`: the JSON Schemas and the conformance vectors) of this
    /// version into a directory, for runtimes written outside tsurara.
    Spec {
        /// The directory to write into.
        dir: PathBuf,
    },
    /// Create `tsurara.config.yaml` in the current directory.
    Init {
        /// Overwrite an existing configuration file.
        #[arg(long)]
        force: bool,
        /// Also write the rules for coding agents into `AGENTS.md` (or `CLAUDE.md` when only that
        /// exists), replacing an earlier tsurara section. An existing configuration is then kept.
        #[arg(long)]
        agents: bool,
        /// Write a plugin skeleton, `tsurara.plugin.ts` (`@tsurara/plugin`), instead of a configuration.
        #[arg(long)]
        plugin: bool,
        /// Start from a configuration for a kind of app, and print the packages it needs.
        #[arg(long, value_enum)]
        preset: Option<Preset>,
    },
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(
        "no configuration file found in the current directory or its ancestors (looked for {})",
        tsurara_config::FILE_NAMES.join(", ")
    )]
    NoConfig,
    #[error(transparent)]
    Config(#[from] tsurara_config::Error),
    #[error("no target named `{0}`")]
    UnknownTarget(String),
    #[error("target `{target}`: {source}")]
    Load {
        target: String,
        #[source]
        source: Box<tsurara_loader::LoadError>,
    },
    #[error("target `{target}`: {message}")]
    Generate { target: String, message: String },
    #[error("{count} difference(s) found; run `tsurara generate`")]
    Drift { count: usize },
    #[error("{} already exists; use --force to overwrite it", .0.display())]
    ConfigExists(PathBuf),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("target `{target}`: {count} warning(s) with --strict")]
    Strict { target: String, count: usize },
    #[error("target `{target}`: afterWrite `{command}`: {source}")]
    AfterWrite {
        target: String,
        command: String,
        #[source]
        source: tsurara_backend::plugin::PluginError,
    },
    #[error("{0}")]
    Inspect(String),
    /// The error is already in the JSON report on stdout.
    #[error("see the report")]
    Reported,
    #[error("target `{target}`: {source}")]
    Write {
        target: String,
        #[source]
        source: write::WriteError,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Generate {
            config,
            target,
            strict,
            format,
        } => generate(config, target.as_deref(), strict, format),
        Commands::Check {
            config,
            target,
            format,
        } => check(config, target.as_deref(), format),
        Commands::Init {
            force,
            agents,
            plugin,
            preset,
        } => {
            if plugin {
                init_plugin(force)
            } else {
                init(force, agents, preset)
            }
        }
        Commands::Inspect { what } => inspect::run(what),
        Commands::Mcp => mcp::serve(),
        Commands::TestBackend {
            backend,
            check,
            options,
            definitions,
            keep,
        } => match test_backend::run(
            &backend,
            check.as_deref(),
            options.as_deref(),
            &definitions,
            keep.as_deref(),
        ) {
            Ok(0) => Ok(()),
            Ok(n) => Err(Error::Inspect(format!("{n} definition(s) failed"))),
            Err(e) => Err(Error::Inspect(e)),
        },
        Commands::Spec { dir } => match test_backend::write_spec(&dir) {
            Ok(n) => {
                println!("{n} file(s) written to {}", dir.display());
                Ok(())
            }
            Err(e) => Err(Error::Inspect(e)),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Reported) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn load_targets(config: Option<PathBuf>, only: Option<&str>) -> Result<Vec<Target>, Error> {
    let path = match config {
        Some(p) => p,
        None => {
            let cwd = std::env::current_dir().map_err(|_| Error::NoConfig)?;
            tsurara_config::find(&cwd).ok_or(Error::NoConfig)?
        }
    };
    let config = tsurara_config::load(&path)?;
    match only {
        Some(name) => Ok(vec![
            config
                .target(name)
                .ok_or_else(|| Error::UnknownTarget(name.to_owned()))?
                .clone(),
        ]),
        None => Ok(config.targets),
    }
}

/// Generates the targets, with the report of what was done.
fn generate_report(
    config: Option<PathBuf>,
    only: Option<&str>,
    strict: bool,
    format: Format,
) -> (GenerateReport, Result<(), Error>) {
    let mut report = GenerateReport {
        command: "generate".into(),
        ..GenerateReport::default()
    };
    let result = load_targets(config, only).and_then(|targets| {
        for target in targets {
            report
                .targets
                .push(generate_target(&target, strict, format)?);
        }
        Ok(())
    });
    report.error = result.as_ref().err().map(ToString::to_string);
    (report, result)
}

fn generate(
    config: Option<PathBuf>,
    only: Option<&str>,
    strict: bool,
    format: Format,
) -> Result<(), Error> {
    let (report, result) = generate_report(config, only, strict, format);
    if format == Format::Json {
        print_json(&report);
        return result.map_err(|_| Error::Reported);
    }
    result
}

fn print_json(value: &impl serde::Serialize) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("reports serialize")
    );
}

/// Configurations `tsurara init --preset` starts from.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum Preset {
    /// React with TanStack Query: query options, Zod schemas and MSW handlers (docs/guide/react.md).
    ReactQuery,
}

/// Template written by `tsurara init --preset react-query`.
const REACT_QUERY_TEMPLATE: &str = "\
# tsurara configuration for React with TanStack Query. See docs/guide/react.md.
input: ./openapi.yaml
output: ./src/generated
typescript:
  importExtension: none      # extensionless imports, as bundlers (Vite, Next.js) resolve them
  query: tanstack            # query and mutation options with typed keys (query/)
  validators: zod            # response schemas for validateResponses (zod/)
  mocks: msw                 # MSW handlers and mock factories for tests (mocks/)
  # Lists read page by page get infinite query options:
  # infiniteQueries:
  #   listItems: { param: query.cursor, next: /nextCursor }
";

/// Packages the React with TanStack Query preset needs.
const REACT_QUERY_PACKAGES: &str = "\
install the packages the generated files use:
  npm install @tsurara/client @tanstack/react-query zod
  npm install --save-dev msw";

/// Template written by `tsurara init`.
const CONFIG_TEMPLATE: &str = "\
# tsurara configuration. See docs/guide/configuration.md for every key.
input: ./openapi.yaml
output: ./src/generated
# language: typescript       # typescript or python
# naming:
#   overrides:
#     \"#/components/schemas/Error\": ApiError
# typescript:
#   importExtension: js      # js, ts or none
#   layout: split            # split, tags or single
";

/// The section `tsurara init --agents` writes for coding agents.
const AGENT_RULES: &str = "<!-- tsurara:start -->
## Generated API clients (tsurara)

`tsurara.config.yaml` generates API types and call functions from the OpenAPI definition.

- Do not edit generated files (they start with `Generated by tsurara` or `Code generated by tsurara`). Change the definition or `tsurara.config.yaml`, then run `tsurara generate`.
- Call the generated function for an endpoint instead of building requests by hand. Find it with `tsurara inspect operations --search \"<words>\"`, and see how to call it with `tsurara inspect operation <operationId>` (or the MCP server, `tsurara mcp`).
- Use what else is generated for an operation instead of writing it: TanStack Query options, keys and filters (`query/`) rather than a hand-written `queryFn` or query key, and mock handlers and factories (`mocks/`) in tests. `tsurara inspect operation <operationId>` lists them.
- Calls return Results: check success before using the data, or use `unwrap` where an exception is right.
- Run `tsurara check` before finishing; it fails when generated files are stale or edited.
- The tsurara skill (`skills/tsurara/SKILL.md` in the tsurara repository) has the details per language.
<!-- tsurara:end -->
";

/// Writes `AGENT_RULES` into `AGENTS.md`, or `CLAUDE.md` when only that exists, replacing an earlier
/// section between the markers.
fn write_agent_rules() -> Result<PathBuf, Error> {
    let agents = PathBuf::from("AGENTS.md");
    let claude = PathBuf::from("CLAUDE.md");
    let path = if !agents.exists() && claude.exists() {
        claude
    } else {
        agents
    };
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let new = match (
        old.find("<!-- tsurara:start -->"),
        old.find("<!-- tsurara:end -->"),
    ) {
        (Some(start), Some(end)) if start < end => {
            let end = end + "<!-- tsurara:end -->\n".len().min(old.len() - end);
            format!("{}{AGENT_RULES}{}", &old[..start], &old[end..])
        }
        _ if old.is_empty() => AGENT_RULES.to_owned(),
        _ => format!("{}\n\n{AGENT_RULES}", old.trim_end()),
    };
    std::fs::write(&path, new).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// The skeleton `tsurara init --plugin` writes.
const PLUGIN_TEMPLATE: &str = "\
// A tsurara plugin (docs/guide/plugins.md). Add it to tsurara.config.yaml with
// `plugins: [./tsurara.plugin.ts]`; it needs `@tsurara/plugin` and Node 22.18 or later.
import { definePlugin, ts, type TypeScriptTargetOptions } from '@tsurara/plugin';

export default definePlugin({
  name: 'my-plugin',
  // Adds one file per operation that re-exports its call function.
  extend({ language, manifest, marker, options }) {
    if (language !== 'typescript' || manifest == null) return [];
    const target = options as TypeScriptTargetOptions;
    return manifest.operations.flatMap((op) => {
      if (op.function == null) return [];
      const f = ts.file(`extra/${op.function.identifier}.ts`, target);
      const fn = f.import(op.function);
      f.line(`export { ${fn} };`);
      return [{ path: f.path, contents: f.render(marker) }];
    });
  },
});
";

fn init_plugin(force: bool) -> Result<(), Error> {
    let path = PathBuf::from("tsurara.plugin.ts");
    if !force && path.exists() {
        return Err(Error::ConfigExists(path));
    }
    std::fs::write(&path, PLUGIN_TEMPLATE).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    println!(
        "created {}; add `plugins: [./tsurara.plugin.ts]` to tsurara.config.yaml",
        path.display()
    );
    Ok(())
}

fn init(force: bool, agents: bool, preset: Option<Preset>) -> Result<(), Error> {
    let path = PathBuf::from(tsurara_config::FILE_NAMES[0]);
    if agents {
        let written = write_agent_rules()?;
        println!(
            "wrote the rules for coding agents into {}",
            written.display()
        );
        if !force && path.exists() {
            return Ok(());
        }
    }
    if !force && path.exists() {
        return Err(Error::ConfigExists(path));
    }
    let template = match preset {
        None => CONFIG_TEMPLATE,
        Some(Preset::ReactQuery) => REACT_QUERY_TEMPLATE,
    };
    std::fs::write(&path, template).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    println!("created {}", path.display());
    if let Some(Preset::ReactQuery) = preset {
        println!("{REACT_QUERY_PACKAGES}");
    }
    Ok(())
}

fn check(config: Option<PathBuf>, only: Option<&str>, format: Format) -> Result<(), Error> {
    let (report, result) = check_report(config, only, format);
    if format == Format::Json {
        print_json(&report);
        return result.map_err(|_| Error::Reported);
    }
    result
}

/// Checks the targets (printing lines with `Format::Text`), with the report.
fn check_report(
    config: Option<PathBuf>,
    only: Option<&str>,
    format: Format,
) -> (CheckReport, Result<(), Error>) {
    let mut report = CheckReport {
        command: "check".into(),
        ..CheckReport::default()
    };
    let mut count = 0;
    let result = load_targets(config, only).and_then(|targets| {
        for target in targets {
            let backend = backend(&target)?;
            let output = produce(&target, backend.as_ref())?;
            let markers = backend.markers();
            let diffs =
                write::diff_output(&target.output, &output.files, &markers).map_err(|source| {
                    Error::Write {
                        target: target.name.clone(),
                        source,
                    }
                })?;
            let (problems, runtime) =
                check_client_version(&target, markers.recorded_version(&output.files));
            if format == Format::Text {
                for d in &diffs {
                    println!("{}: {d}", target.name);
                }
                for line in &runtime {
                    println!("{}: {line}", target.name);
                }
                if diffs.is_empty() {
                    println!("{}: up to date ({})", target.name, target.output.display());
                }
            }
            count += diffs.len() + problems;
            report.targets.push(CheckedTarget {
                name: target.name.clone(),
                output: target.output.display().to_string(),
                up_to_date: diffs.is_empty() && problems == 0,
                differences: diffs
                    .iter()
                    .map(|d| {
                        let (kind, path) = match d {
                            write::Difference::Changed(p) => ("changed", p),
                            write::Difference::Missing(p) => ("missing", p),
                            write::Difference::Stale(p) => ("stale", p),
                        };
                        DifferenceReport {
                            kind: kind.into(),
                            path: path.clone(),
                        }
                    })
                    .collect(),
                runtime,
            });
        }
        if count > 0 {
            return Err(Error::Drift { count });
        }
        Ok(())
    });
    report.error = result.as_ref().err().map(ToString::to_string);
    (report, result)
}

/// Reports a runtime library that does not satisfy the range recorded in the generated code (through
/// the backend's version marker). Returns the number of problems (0 or 1) and the lines to report. A
/// library that is not installed is only reported.
fn check_client_version(target: &Target, range: Option<String>) -> (usize, Vec<String>) {
    let (Some(range), Some(module)) = (range, runtime_module(target)) else {
        return (0, Vec::new());
    };
    // Go records a module version (`v0.1.0`), which allows the same minor version.
    let range = match &target.language {
        Language::Go(_) => format!("^{}", range.trim_start_matches('v')),
        _ => range,
    };
    let (found, place) = match &target.language {
        Language::Rust(_) => (client_version::locked(&target.output, module), "Cargo.lock"),
        Language::Go(_) => match client_version::go_required(&target.output, module) {
            Some(client_version::GoRequirement::Replaced(go_mod)) => {
                return (
                    0,
                    vec![format!(
                        "{module} is replaced by a directory in {}; its version was not checked",
                        go_mod.display()
                    )],
                );
            }
            Some(client_version::GoRequirement::Version(go_mod, version)) => {
                (Some((go_mod, version)), "go.mod")
            }
            None => (None, "go.mod"),
        },
        _ => (
            client_version::installed(&target.output, module),
            "node_modules",
        ),
    };
    match found {
        None => (
            0,
            vec![format!(
                "{module} is not in a {place} above {}; its version was not checked",
                target.output.display()
            )],
        ),
        Some((manifest, version)) => match client_version::satisfies(&range, &version) {
            Some(true) => (0, Vec::new()),
            Some(false) => (
                1,
                vec![format!(
                    "{module} {version} ({}) does not satisfy {range}, which the generated code requires",
                    manifest.display()
                )],
            ),
            None => (
                0,
                vec![format!("cannot check {module} against the range {range}")],
            ),
        },
    }
}

/// The registry of backends: a built-in language configured with its section, or a program.
fn backend(target: &Target) -> Result<Box<dyn tsurara_backend::Backend>, Error> {
    Ok(match &target.language {
        Language::TypeScript(options) => Box::new(tsurara_lang_typescript::TypeScript {
            options: options.clone(),
        }),
        Language::Python(options) => Box::new(tsurara_lang_python::Python {
            options: options.clone(),
        }),
        Language::Rust(options) => Box::new(tsurara_lang_rust::Rust {
            options: options.clone(),
        }),
        Language::Go(options) => Box::new(tsurara_lang_go::Go {
            options: options.clone(),
        }),
        Language::External { command, options } => Box::new(
            external::ExternalBackend::describe(command, options).map_err(|message| {
                Error::Generate {
                    target: target.name.clone(),
                    message,
                }
            })?,
        ),
    })
}

/// The module name of the runtime library the target's generated code imports, if tsurara knows it.
fn runtime_module(target: &Target) -> Option<&str> {
    match &target.language {
        Language::TypeScript(options) => Some(&options.client_module),
        Language::Rust(_) => Some("tsurara-client"),
        Language::Go(options) => Some(&options.runtime_module),
        Language::Python(_) | Language::External { .. } => None,
    }
}

/// Loads the definition and generates the files of a target, running the hooks that transform them.
fn produce(
    target: &Target,
    backend: &dyn tsurara_backend::Backend,
) -> Result<tsurara_backend::Output, Error> {
    let fetcher = DefaultFetcher::with_http(HttpOptions {
        headers: target.input.headers.clone(),
        origin: Some(target.input.url.origin()),
        timeout: None,
    });
    let mut set = DocumentSet::load(&target.input.url, &fetcher).map_err(|source| Error::Load {
        target: target.name.clone(),
        source: Box::new(source),
    })?;
    let generate_error = |message: String| Error::Generate {
        target: target.name.clone(),
        message,
    };
    tsurara_loader::normalize_swagger2(&mut set).map_err(|e| generate_error(e.to_string()))?;
    let plugins: Vec<hooks::CommandPlugin> = target
        .hooks
        .transform_document
        .iter()
        .map(hooks::CommandPlugin::document)
        .chain(
            target
                .hooks
                .transform_model
                .iter()
                .map(hooks::CommandPlugin::model),
        )
        .chain(target.hooks.extend.iter().map(hooks::CommandPlugin::extend))
        .chain(
            target
                .hooks
                .transform_files
                .iter()
                .map(hooks::CommandPlugin::files),
        )
        .collect();
    // Plugin modules run after the hooks of each stage, in the order configured.
    let modules: Vec<plugins::PluginProcess> = target
        .plugins
        .iter()
        .map(plugins::PluginProcess::start)
        .collect::<Result<_, _>>()
        .map_err(generate_error)?;
    let plugins: Vec<&dyn tsurara_backend::plugin::Plugin> = plugins
        .iter()
        .map(|p| p as &dyn tsurara_backend::plugin::Plugin)
        .chain(
            modules
                .iter()
                .map(|p| p as &dyn tsurara_backend::plugin::Plugin),
        )
        .collect();
    tsurara_backend::generate_with_plugins(backend, set, &target.options, &plugins)
        .map_err(|e| generate_error(e.to_string()))
}

/// The third-party packages a vendored runtime needs: tsurara copies the runtime, not
/// its dependencies.
fn vendored_dependencies(target: &Target) -> Vec<&'static str> {
    if target.options.runtime != tsurara_sem::RuntimeSource::Vendor {
        return Vec::new();
    }
    match &target.language {
        Language::Python(_) if target.options.operations => vec!["httpx>=0.27"],
        Language::Rust(_) => tsurara_lang_rust::vendored_dependencies(target.options.operations),
        _ => Vec::new(),
    }
}

fn generate_target(
    target: &Target,
    strict: bool,
    format: Format,
) -> Result<GeneratedTarget, Error> {
    let backend = backend(target)?;
    if let Language::Rust(_) | Language::Go(_) = target.language {
        eprintln!(
            "{}: note: the {} backend is experimental; its output may change",
            target.name,
            if matches!(target.language, Language::Go(_)) {
                "Go"
            } else {
                "Rust"
            }
        );
    }
    let output = produce(target, backend.as_ref())?;
    if format == Format::Text {
        print_warnings(&target.name, &output.warnings);
        let deps = vendored_dependencies(target);
        if !deps.is_empty() {
            eprintln!(
                "{}: note: the vendored runtime needs these dependencies: {}",
                target.name,
                deps.join(", ")
            );
        }
    }
    if strict && !output.warnings.is_empty() {
        return Err(Error::Strict {
            target: target.name.clone(),
            count: output.warnings.len(),
        });
    }
    let stats = write::write_output(&target.output, &output.files, &backend.markers()).map_err(
        |source| Error::Write {
            target: target.name.clone(),
            source,
        },
    )?;
    if format == Format::Text {
        println!(
            "{}: {} written, {} unchanged, {} deleted ({})",
            target.name,
            stats.written.len(),
            stats.unchanged.len(),
            stats.deleted.len(),
            target.output.display()
        );
    }
    hooks::after_write(&target.hooks.after_write, &target.output).map_err(
        |(command, source)| Error::AfterWrite {
            target: target.name.clone(),
            command,
            source,
        },
    )?;
    Ok(GeneratedTarget {
        name: target.name.clone(),
        language: backend.id().to_owned(),
        output: target.output.display().to_string(),
        written: stats.written,
        unchanged: stats.unchanged,
        deleted: stats.deleted,
        warnings: warning_reports(&output.warnings),
    })
}

fn warning_reports(warnings: &[Warning]) -> Vec<WarningReport> {
    let cwd = std::env::current_dir().ok();
    warnings
        .iter()
        .map(|w| WarningReport {
            code: w.kind.as_str().to_owned(),
            document: display_url(&w.loc.doc, cwd.as_deref()),
            pointer: w.loc.pointer.clone(),
            message: w.message.clone(),
            language: w.language.clone(),
        })
        .collect()
}

fn print_warnings(target: &str, warnings: &[Warning]) {
    let cwd = std::env::current_dir().ok();
    for w in warnings {
        let doc = display_url(&w.loc.doc, cwd.as_deref());
        eprintln!(
            "warning[{}]: {target}: {doc}#{}",
            w.kind.as_str(),
            w.loc.pointer
        );
        match &w.language {
            // An approximation of that language's backend, not a problem in the definition.
            Some(language) => eprintln!("  {} ({language})", w.message),
            None => eprintln!("  {}", w.message),
        }
    }
}

/// Shows local files as paths, relative to the current directory when they are inside it.
fn display_url(url: &url::Url, cwd: Option<&std::path::Path>) -> String {
    let Ok(path) = url.to_file_path() else {
        return url.to_string();
    };
    let rel = cwd.and_then(|cwd| path.strip_prefix(cwd).ok());
    rel.unwrap_or(&path).to_string_lossy().replace('\\', "/")
}

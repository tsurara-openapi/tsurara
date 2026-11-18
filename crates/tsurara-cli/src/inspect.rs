//! `tsurara inspect` (design.md 12): answers about the definition and the generated code of
//! a target, from the model, the manifest and the generated files, without writing anything.

use crate::{Error, Format, backend, load_targets, produce};
use serde_json::{Value, json};
use std::path::PathBuf;
use tsurara_backend::{ExampleInput, Manifest};
use tsurara_config::Target;
use tsurara_loader::{DefaultFetcher, DocumentSet, HttpOptions};
use tsurara_sem::operation::Operation;
use tsurara_sem::{Model, WarningKind};

#[derive(clap::Args, Default)]
pub struct Common {
    /// Configuration file. Searched for in the current directory and its ancestors by default.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// The target to inspect; required when the configuration has several.
    #[arg(long)]
    pub target: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t)]
    pub format: Format,
}

#[derive(clap::Subcommand)]
pub enum What {
    /// The operations: id, method, path, summary, and the generated function.
    Operations {
        /// Only operations with this tag.
        #[arg(long)]
        tag: Option<String>,
        /// Only operations whose id, path or summary contains this text (case-insensitive).
        #[arg(long)]
        search: Option<String>,
        #[command(flatten)]
        common: Common,
    },
    /// One operation, by id or as "METHOD /path": parameters, body, responses, the generated function
    /// and a call example in the target's language.
    Operation {
        operation: String,
        #[command(flatten)]
        common: Common,
    },
    /// One schema, by its name in the definition or its generated name: the generated name, file and
    /// declaration.
    Schema {
        name: String,
        #[command(flatten)]
        common: Common,
    },
    /// What a warning code means and how to resolve it.
    Warning {
        code: String,
        /// Output format.
        #[arg(long, value_enum, default_value_t)]
        format: Format,
    },
}

/// What a warning code means.
pub fn warning(code: &str) -> Result<Value, Error> {
    let kind = WarningKind::from_code(code).ok_or_else(|| {
        Error::Inspect(format!(
            "no warning `{code}`; the codes are {}",
            WarningKind::ALL
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    Ok(json!({
        "code": kind.as_str(),
        "explanation": kind.explanation(),
    }))
}

/// The words of `text`, lower-cased, with camelCase and punctuation split (`getPetById` → get, pet,
/// by, id).
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut prev_lower = false;
    for c in text.chars() {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// The operations of the target with tag `tag` whose id, method, path, summary and tags contain every
/// word of `search` (case-insensitive; `find pet by id` finds `getPetById`), at most `limit` of them
/// after `offset`.
pub fn operations(
    common: &Common,
    tag: Option<&str>,
    search: Option<&str>,
    offset: usize,
    limit: Option<usize>,
) -> Result<Value, Error> {
    let ctx = Context::load(common)?;
    let query = search.map(words).unwrap_or_default();
    let matching: Vec<&Operation> = ctx
        .model
        .operations
        .iter()
        .filter(|op| tag.is_none_or(|t| op.tags.iter().any(|x| x == t)))
        .filter(|op| {
            let text = format!(
                "{} {} {} {} {}",
                op.id,
                op.method,
                op.path,
                summary(op),
                op.tags.join(" ")
            );
            let haystack = words(&text);
            query
                .iter()
                .all(|w| haystack.iter().any(|h| h.starts_with(w.as_str())))
        })
        .collect();
    let total = matching.len();
    let ops: Vec<Value> = matching
        .into_iter()
        .skip(offset)
        .take(limit.unwrap_or(usize::MAX))
        .map(|op| {
            json!({
                "operationId": op.id,
                "method": op.method,
                "path": op.path,
                "summary": summary(op),
                "tags": op.tags,
                "function": ctx.entry(op).and_then(|e| e.function.as_ref()),
            })
        })
        .collect();
    Ok(json!({
        "target": ctx.target.name,
        "language": ctx.language,
        "total": total,
        "operations": ops,
    }))
}

/// One operation by id or `METHOD /path`, with a call example.
pub fn operation(common: &Common, key: &str) -> Result<Value, Error> {
    let ctx = Context::load(common)?;
    let op = ctx.find_operation(key)?;
    let entry = ctx.entry(op);
    let example = ctx.manifest.as_ref().and_then(|manifest| {
        let (samples, _) = tsurara_sample::samples(&ctx.model);
        let s = samples
            .operations
            .iter()
            .find(|s| s.operation_id == op.id)?;
        ctx.backend.example(&ExampleInput {
            model: &ctx.model,
            opts: &ctx.target.options,
            manifest,
            operation: op,
            samples: s,
        })
    });
    Ok(json!({
        "target": ctx.target.name,
        "language": ctx.language,
        "operationId": op.id,
        "method": op.method,
        "path": op.path,
        "summary": summary(op),
        "tags": op.tags,
        "parameters": op.params.iter().map(|p| json!({
            "name": p.name,
            "in": p.location.group().trim_end_matches('s'),
            "required": p.required,
        })).collect::<Vec<_>>(),
        "requestBody": op.body.as_ref().map(|b| json!({
            "required": b.required,
            "mediaTypes": b.contents.iter().map(|c| &c.media_type).collect::<Vec<_>>(),
        })),
        "responses": op.responses.iter().map(|r| json!({
            "status": r.status.key(),
            "description": r.description,
            "mediaTypes": r.contents.iter().map(|c| &c.media_type).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "function": entry.and_then(|e| e.function.as_ref()),
        "types": entry.map(|e| &e.types),
        "helpers": entry.map(|e| &e.helpers),
        "example": example,
    }))
}

/// One schema by its name in the definition or its generated name.
pub fn schema(common: &Common, name: &str) -> Result<Value, Error> {
    let ctx = Context::load(common)?;
    let manifest = ctx
        .manifest
        .as_ref()
        .ok_or_else(|| Error::Inspect("the backend of this target returns no manifest".into()))?;
    let entry = manifest
        .types
        .iter()
        .find(|t| {
            t.export.identifier == name
                || ctx
                    .model
                    .schemas
                    .iter()
                    .any(|d| d.id == t.name && d.preferred_name == name && !d.writable)
        })
        .ok_or_else(|| {
            Error::Inspect(format!(
                "no schema `{name}` in target `{}`",
                ctx.target.name
            ))
        })?;
    let decl = ctx.model.schemas.iter().find(|d| d.id == entry.name);
    let declaration = ctx
        .files
        .iter()
        .find(|f| f.path == entry.export.file)
        .map(|f| f.contents.clone());
    Ok(json!({
        "target": ctx.target.name,
        "language": ctx.language,
        "name": decl.map(|d| &d.preferred_name),
        "pointer": decl.map(|d| format!("#{}", d.loc.pointer)),
        "identifier": entry.export.identifier,
        "file": entry.export.file,
        "declaration": declaration,
    }))
}

pub fn run(what: What) -> Result<(), Error> {
    match what {
        What::Warning { code, format } => {
            let report = warning(&code)?;
            match format {
                Format::Json => crate::print_json(&report),
                Format::Text => println!(
                    "{}\n\n{}",
                    text(&report["code"]),
                    text(&report["explanation"])
                ),
            }
        }
        What::Operations {
            tag,
            search,
            common,
        } => {
            let report = operations(&common, tag.as_deref(), search.as_deref(), 0, None)?;
            match common.format {
                Format::Json => crate::print_json(&report),
                Format::Text => {
                    for op in report["operations"].as_array().into_iter().flatten() {
                        let function = match &op["function"] {
                            Value::Null => String::new(),
                            f => format!(" -> {} ({})", text(&f["identifier"]), text(&f["file"])),
                        };
                        println!(
                            "{} {}  {}{function}",
                            text(&op["method"]),
                            text(&op["path"]),
                            text(&op["operationId"])
                        );
                        let s = text(&op["summary"]);
                        if !s.is_empty() {
                            println!("    {s}");
                        }
                    }
                }
            }
        }
        What::Operation { operation, common } => {
            let report = self::operation(&common, &operation)?;
            match common.format {
                Format::Json => crate::print_json(&report),
                Format::Text => print_operation(&report),
            }
        }
        What::Schema { name, common } => {
            let report = schema(&common, &name)?;
            match common.format {
                Format::Json => crate::print_json(&report),
                Format::Text => {
                    println!(
                        "{} -> {} ({})\n",
                        text(&report["name"]),
                        text(&report["identifier"]),
                        text(&report["file"])
                    );
                    print!("{}", text(&report["declaration"]));
                }
            }
        }
    }
    Ok(())
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

fn summary(op: &Operation) -> String {
    op.doc
        .as_ref()
        .and_then(|d| d.description.as_deref())
        .and_then(|d| d.lines().next())
        .unwrap_or("")
        .to_owned()
}

fn print_operation(r: &Value) {
    println!(
        "{}: {} {}",
        text(&r["operationId"]),
        text(&r["method"]),
        text(&r["path"])
    );
    let s = text(&r["summary"]);
    if !s.is_empty() {
        println!("  {s}");
    }
    if let Some(params) = r["parameters"].as_array().filter(|p| !p.is_empty()) {
        println!("parameters:");
        for p in params {
            let required = if p["required"] == true {
                ", required"
            } else {
                ""
            };
            println!("  {} ({}{required})", text(&p["name"]), text(&p["in"]));
        }
    }
    if let Some(b) = r["requestBody"].as_object() {
        let required = if b["required"] == true {
            " (required)"
        } else {
            ""
        };
        println!("request body{required}: {}", media(&b["mediaTypes"]));
    }
    println!("responses:");
    for resp in r["responses"].as_array().into_iter().flatten() {
        println!("  {} {}", text(&resp["status"]), media(&resp["mediaTypes"]));
    }
    if let Some(f) = r["function"].as_object() {
        println!(
            "function: {} ({})",
            text(&f["identifier"]),
            text(&f["file"])
        );
    }
    if let Some(h) = r["helpers"].as_object().filter(|h| !h.is_empty()) {
        println!("also generated:");
        for (role, export) in h {
            println!(
                "  {role}: {} ({})",
                text(&export["identifier"]),
                text(&export["file"])
            );
        }
    }
    if let Some(e) = r["example"].as_object() {
        println!("\nexample ({}):\n", text(&e["file"]));
        print!("{}", text(&e["code"]));
        if let Some(note) = e.get("note").and_then(Value::as_str) {
            println!("\nnote: {note}");
        }
    }
}

fn media(v: &Value) -> String {
    let list: Vec<String> = v.as_array().into_iter().flatten().map(text).collect();
    if list.is_empty() {
        "(no body)".into()
    } else {
        list.join(", ")
    }
}

/// A target with its model, generated files and manifest.
struct Context {
    target: Target,
    backend: Box<dyn tsurara_backend::Backend>,
    language: String,
    model: Model,
    files: Vec<tsurara_backend::OutputFile>,
    manifest: Option<Manifest>,
}

impl Context {
    fn load(common: &Common) -> Result<Self, Error> {
        let mut targets = load_targets(common.config.clone(), common.target.as_deref())?;
        if targets.len() > 1 {
            return Err(Error::Inspect(format!(
                "the configuration has {} targets; choose one with --target ({})",
                targets.len(),
                targets
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let target = targets.remove(0);
        let backend = backend(&target)?;
        let output = produce(&target, backend.as_ref())?;
        let model = model(&target)?;
        Ok(Self {
            language: backend.id().to_owned(),
            backend,
            model,
            files: output.files,
            manifest: output.manifest,
            target,
        })
    }

    fn entry(&self, op: &Operation) -> Option<&tsurara_backend::manifest::OperationEntry> {
        self.manifest
            .as_ref()?
            .operations
            .iter()
            .find(|e| e.operation_id == op.id)
    }

    /// An operation by id, or by `METHOD /path`.
    fn find_operation(&self, key: &str) -> Result<&Operation, Error> {
        let by_path = key
            .split_once(' ')
            .map(|(m, p)| (m.to_uppercase(), p.trim()));
        self.model
            .operations
            .iter()
            .find(|op| match &by_path {
                Some((method, path)) => op.method == *method && op.path == *path,
                None => op.id == key,
            })
            .ok_or_else(|| {
                Error::Inspect(format!(
                    "no operation `{key}` in target `{}`; `tsurara inspect operations` lists them",
                    self.target.name
                ))
            })
    }
}

/// The model of a target, built as `generate` builds it before hooks.
fn model(target: &Target) -> Result<Model, Error> {
    let fetcher = DefaultFetcher::with_http(HttpOptions {
        headers: target.input.headers.clone(),
        origin: Some(target.input.url.origin()),
        timeout: None,
    });
    let fail = |message: String| Error::Generate {
        target: target.name.clone(),
        message,
    };
    let mut set = DocumentSet::load(&target.input.url, &fetcher).map_err(|source| Error::Load {
        target: target.name.clone(),
        source: Box::new(source),
    })?;
    tsurara_loader::normalize_swagger2(&mut set).map_err(|e| fail(e.to_string()))?;
    tsurara_sem::build_model(&set, &target.options).map_err(|e| fail(e.to_string()))
}

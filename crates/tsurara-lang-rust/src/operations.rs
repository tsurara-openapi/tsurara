//! Call functions: one module per operation in `operations/`, with its OperationDef, an
//! options struct for what is optional, and an async function that calls `Client::request`.

use crate::HEADER;
use crate::naming::{RustPolicy, allocate, allocate_variants, ident};
use crate::print::{Printer, docs};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_backend::manifest::{Export, OperationEntry, OperationKind};
use tsurara_backend::operation_def::operation_def;
use tsurara_lower::tir::{Expr, OperationTypes};
use tsurara_naming::{Context, Names, NamingPolicy, pascal_case};
use tsurara_sem::operation::{BodyKind, Operation, ParamIn};

/// A generated operation module: its file name (without `.rs`), the identifier it is declared
/// with, and its contents.
pub(crate) struct OperationModule {
    file: String,
    module: String,
    contents: String,
    /// The manifest entry; its files are filled in by `files`.
    pub(crate) entry: OperationEntry,
}

/// Names the function uses itself; parameters with these names get a suffix.
const FUNCTION_NAMES: &[&str] = &["client", "options", "input", "body", "timeout"];

struct Param {
    field: String,
    wire: String,
    location: ParamIn,
    ty: String,
    required: bool,
}

/// The type of a result side: one type, `Option<T>` for a type or no content, otherwise any JSON.
fn result_type(p: &Printer, mut types: Vec<String>) -> String {
    types.sort();
    types.dedup();
    match types.as_slice() {
        [] => format!("{}::serde_json::Value", p.rt),
        [one] => one.clone(),
        [a, b] if a == "()" => format!("Option<{b}>"),
        [a, b] if b == "()" => format!("Option<{a}>"),
        _ => format!("{}::serde_json::Value", p.rt),
    }
}

/// The type a response body is read as: binary bodies are bytes.
fn response_type(p: &mut Printer, e: &Expr) -> String {
    match e {
        Expr::Bytes => "Vec<u8>".into(),
        _ => p.ty(e),
    }
}

/// A variant name for a request media type.
fn media_variant(media: &str) -> String {
    let essence = media.split(';').next().unwrap_or(media).trim();
    match essence {
        "application/json" => "Json".into(),
        "application/x-www-form-urlencoded" => "Form".into(),
        "text/plain" => "Text".into(),
        "application/octet-stream" => "OctetStream".into(),
        m if m.starts_with("multipart/") => "Multipart".into(),
        m if m.ends_with("+json") => format!("{}Json", pascal_case(m.trim_end_matches("+json"))),
        m => {
            let p = pascal_case(m);
            if p.is_empty() { "Media".into() } else { p }
        }
    }
}

/// A raw string literal that holds `text`.
fn raw_string(text: &str) -> String {
    let mut hashes = 1;
    while text.contains(&format!("\"{}", "#".repeat(hashes))) {
        hashes += 1;
    }
    let h = "#".repeat(hashes);
    format!("r{h}\"{text}\"{h}")
}

pub(crate) fn operation(
    p: &mut Printer,
    op: &Operation,
    types: &OperationTypes,
    scope: &mut Names,
    functions: &mut Names,
) -> OperationModule {
    let rt = p.rt.to_owned();
    let raw = RustPolicy.convert(Context::Function, &op.id, false);
    let function = functions
        .claim(&RustPolicy, Context::Function, "functions", &raw, &op.id)
        .name;
    let pascal = pascal_case(&function);
    let mut claim = |suffix: &str| {
        let raw = RustPolicy.convert(Context::Type, &format!("{pascal}{suffix}"), false);
        let pointer = format!("#{}/{suffix}", op.loc.pointer);
        scope
            .claim(&RustPolicy, Context::Type, "types", &raw, &pointer)
            .name
    };
    let options_name = claim("Options");

    // Parameters: required ones are arguments, optional ones fields of the options.
    let mut raws: Vec<&str> = FUNCTION_NAMES.to_vec();
    raws.extend(op.params.iter().map(|q| q.name.as_str()));
    let fields = allocate(Context::Member, &raws);
    let params: Vec<Param> = op
        .params
        .iter()
        .zip(&types.params)
        .zip(&fields[FUNCTION_NAMES.len()..])
        .map(|((q, e), field)| {
            // An optional parameter is left out with `None`; a `null` it allows is not sent apart.
            let e = match e {
                Expr::Nullable(inner) if !q.required => inner.as_ref(),
                e => e,
            };
            Param {
                field: field.clone(),
                wire: q.name.clone(),
                location: q.location,
                ty: p.ty(e),
                required: q.required,
            }
        })
        .collect();

    // The body: one type, or an enum with a variant per media type.
    let body = op.body.as_ref();
    let body_required = body.is_some_and(|b| b.required);
    let mut body_enum = None;
    let body_type = body.map(|b| {
        if b.contents.len() == 1 {
            p.ty(&types.body[0])
        } else {
            let name = claim("Body");
            let raws: Vec<String> = b.contents.iter().map(|c| media_variant(&c.media_type)).collect();
            let refs: Vec<&str> = raws.iter().map(String::as_str).collect();
            let variants = allocate_variants(&refs);
            let mut code = format!(
                "/// The body of `{function}`, in one of its media types.\n#[derive(Debug, Clone)]\npub enum {name} {{\n"
            );
            for ((c, e), v) in b.contents.iter().zip(&types.body).zip(&variants) {
                writeln!(
                    code,
                    "    /// `{}`\n    {v}({}),",
                    tsurara_sem::text::single_line(&c.media_type),
                    p.ty(e)
                )
                .unwrap();
            }
            code.push_str("}\n");
            body_enum = Some((name.clone(), code, variants));
            name
        }
    });
    // How an owned body value of a media type goes into the input.
    let encode = |media: &str, e: &Expr, value: &str| -> String {
        if media.to_ascii_lowercase().starts_with("multipart/") {
            format!("input.multipart_body(&{value});")
        } else if *e == Expr::Bytes {
            format!("input.binary_body({value});")
        } else {
            format!("input.value_body(&{value});")
        }
    };

    // The result.
    let has_success = op
        .responses
        .iter()
        .any(|r| r.status.is_success() == Some(true));
    let mut success = Vec::new();
    let mut errors = Vec::new();
    // Successes that are all read item by item (with no JSON media type next to them) are
    // `Items` of their item type (design.md 8.5).
    let mut item_types = Vec::new();
    let mut only_items = true;
    for ((r, exprs), seqs) in op
        .responses
        .iter()
        .zip(&types.responses)
        .zip(&types.sequences)
    {
        let json = r.contents.iter().any(|c| c.body_kind == BodyKind::Json);
        let is_success = match r.status.is_success() {
            Some(s) => s,
            None => !has_success,
        };
        if is_success {
            if !json && !seqs.is_empty() && seqs.iter().all(Option::is_some) {
                item_types.extend(exprs.iter().map(|e| p.ty(e)));
            } else {
                only_items = false;
            }
        }
    }
    let items = only_items && !item_types.is_empty();
    for (r, exprs) in op.responses.iter().zip(&types.responses) {
        // A response with a JSON media type is read as JSON (servers answer JSON when the request
        // does not ask for another type); a body in its other media types is a parse failure.
        let json = r.contents.iter().any(|c| c.body_kind == BodyKind::Json);
        let tys: Vec<String> = if exprs.is_empty() {
            vec!["()".into()]
        } else {
            r.contents
                .iter()
                .zip(exprs)
                .filter(|(c, _)| !json || c.body_kind == BodyKind::Json)
                .map(|(_, e)| response_type(p, e))
                .collect()
        };
        match r.status.is_success() {
            Some(true) => success.extend(tys),
            Some(false) => errors.extend(tys),
            None if has_success => errors.extend(tys),
            None => success.extend(tys),
        }
    }
    let success = if items {
        format!("{rt}::Items<{}>", result_type(p, item_types))
    } else if success.is_empty() {
        format!("{rt}::serde_json::Value")
    } else {
        result_type(p, success)
    };
    let error = result_type(p, errors);

    // The module.
    let def = serde_json::to_string(&operation_def(op, None)).expect("JSON");
    let mut code = format!(
        "{HEADER}\n\n#[allow(unused_imports)]\nuse super::super::types::*;\n\nstatic DEF: std::sync::LazyLock<{rt}::OperationDef> =\n    std::sync::LazyLock::new(|| {rt}::OperationDef::from_json({}));\n\n",
        raw_string(&def)
    );
    if let Some((_, enum_code, _)) = &body_enum {
        code.push_str(enum_code);
        code.push('\n');
    }
    writeln!(
        code,
        "/// The optional inputs of `{function}`.\n#[derive(Debug, Clone, Default)]\npub struct {options_name} {{"
    )
    .unwrap();
    for q in params.iter().filter(|q| !q.required) {
        writeln!(code, "    pub {}: Option<{}>,", ident(&q.field), q.ty).unwrap();
    }
    if let (Some(t), false) = (&body_type, body_required) {
        writeln!(code, "    pub body: Option<{t}>,").unwrap();
    }
    code.push_str("    /// Overrides the client's timeout.\n    pub timeout: Option<std::time::Duration>,\n}\n\n");

    let mut doc = op.doc.clone().unwrap_or_default();
    if doc.description.is_none() {
        doc.description = Some(format!("`{} {}`", op.method, op.path));
    }
    code.push_str(&docs(Some(&doc), ""));
    let mut args = vec![format!("client: &{rt}::Client")];
    for q in params.iter().filter(|q| q.required) {
        args.push(format!("{}: {}", ident(&q.field), q.ty));
    }
    if let (Some(t), true) = (&body_type, body_required) {
        args.push(format!("body: {t}"));
    }
    args.push(format!("options: {options_name}"));
    writeln!(
        code,
        "pub async fn {}({}) -> {rt}::ApiResult<{success}, {error}> {{",
        ident(&function),
        args.join(", ")
    )
    .unwrap();
    let mutable = if params.is_empty() && body.is_none() {
        ""
    } else {
        "mut "
    };
    code.push_str(&format!(
        "    let {mutable}input = {rt}::Input {{\n        timeout: options.timeout,\n        ..Default::default()\n    }};\n"
    ));
    for q in &params {
        let method = match q.location {
            ParamIn::Path => "path",
            ParamIn::Query => "query",
            ParamIn::Header => "header",
            ParamIn::Cookie => "cookie",
            ParamIn::QueryString => "querystring",
        };
        let call = if q.location == ParamIn::QueryString {
            "input.querystring(VALUE);".to_owned()
        } else {
            format!("input.{method}({:?}, VALUE);", q.wire)
        };
        let field = ident(&q.field);
        if q.required {
            writeln!(code, "    {}", call.replace("VALUE", &format!("&{field}"))).unwrap();
        } else {
            writeln!(
                code,
                "    if let Some(v) = &options.{field} {{\n        {}\n    }}",
                call.replace("VALUE", "v")
            )
            .unwrap();
        }
    }
    if let Some(b) = body {
        let value = if body_required { "body" } else { "v" };
        let statements = match &body_enum {
            None => encode(&b.contents[0].media_type, &types.body[0], value),
            Some((name, _, variants)) => {
                let mut m = format!("match {value} {{\n");
                for ((c, e), v) in b.contents.iter().zip(&types.body).zip(variants) {
                    writeln!(
                        m,
                        "            {name}::{v}(v) => {{\n                {}\n                input.content_type({:?});\n            }}",
                        encode(&c.media_type, e, "v"),
                        c.media_type
                    )
                    .unwrap();
                }
                m.push_str("        }");
                m
            }
        };
        if body_required {
            writeln!(code, "    {}", statements.replace("\n    ", "\n")).unwrap();
        } else {
            writeln!(
                code,
                "    if let Some(v) = options.body {{\n        {statements}\n    }}"
            )
            .unwrap();
        }
    }
    let request = if items { "request_items" } else { "request" };
    writeln!(code, "    client.{request}(&DEF, input).await\n}}").unwrap();

    let module = ident(&function);
    let mut file = function.clone();
    if file == "mod" {
        file.push('_');
    }
    let path = format!("operations/{file}.rs");
    let mut types = BTreeMap::from([("options".to_owned(), Export::new(&options_name, &path))]);
    if let Some((name, ..)) = &body_enum {
        types.insert("body".to_owned(), Export::new(name, &path));
    }
    OperationModule {
        file,
        module,
        contents: code,
        entry: OperationEntry {
            helpers: Default::default(),
            operation_id: op.id.clone(),
            kind: OperationKind::Operation,
            function: Some(Export::new(&function, &path)),
            operation_def: None,
            types,
        },
    }
}

/// The operation modules and `operations/mod.rs`, which re-exports them.
pub(crate) fn files(mut modules: Vec<OperationModule>) -> Vec<OutputFile> {
    modules.sort_by(|a, b| a.file.cmp(&b.file));
    let mut index = format!(
        "{HEADER}\n\n// Generated functions take one argument per required parameter; their error types and body\n// enums are as large as the definition makes them, and their docs follow the definition.\n#![allow(clippy::too_many_arguments, clippy::result_large_err, clippy::large_enum_variant, clippy::doc_lazy_continuation, clippy::doc_overindented_list_items, clippy::empty_docs)]\n\n"
    );
    for m in &modules {
        if m.module.trim_start_matches("r#") == m.file {
            writeln!(index, "mod {};", m.module).unwrap();
        } else {
            writeln!(index, "#[path = \"{}.rs\"]\nmod {};", m.file, m.module).unwrap();
        }
    }
    if !modules.is_empty() {
        index.push('\n');
    }
    for m in &modules {
        writeln!(index, "pub use {}::*;", m.module).unwrap();
    }
    let mut files: Vec<OutputFile> = modules
        .into_iter()
        .map(|m| OutputFile {
            path: format!("operations/{}.rs", m.file),
            contents: m.contents,
        })
        .collect();
    files.push(OutputFile {
        path: "operations/mod.rs".into(),
        contents: index,
    });
    files
}

/// A call example for `tsurara inspect operation`: `example.rs` next to `mod.rs`, which
/// calls the function with the operation's sample values (decoded with serde, so any argument type
/// works) and matches the Result.
pub(crate) fn example(
    input: &tsurara_backend::ExampleInput,
    runtime: &str,
) -> Option<tsurara_backend::Example> {
    let base_url = tsurara_backend::example_base_url(input.model);
    let op = input.operation;
    let entry = input
        .manifest
        .operations
        .iter()
        .find(|e| e.operation_id == op.id)?;
    let function = entry.function.as_ref()?;
    let decoded = |v: &serde_json::Value| format!("from_value(json!({v})).unwrap()");
    let mut args = vec!["&client".to_owned()];
    for p in op.params.iter().filter(|p| p.required) {
        let sample = input
            .samples
            .params
            .iter()
            .find(|s| s.name == p.name && s.location == p.location)?;
        args.push(decoded(&sample.value));
    }
    if let Some(body) = op.body.as_ref().filter(|b| b.required) {
        let json = body.contents.len() == 1
            && matches!(body.contents[0].body_kind, BodyKind::Json | BodyKind::Auto);
        match (&input.samples.body, json) {
            (Some(sample), true) => args.push(decoded(&sample.value)),
            _ => args.push("todo!(\"the request body\")".into()),
        }
    }
    let decodes = args.iter().any(|a| a.starts_with("from_value("));
    args.push("Default::default()".into());
    let serde = if decodes {
        format!("use {runtime}::serde_json::{{from_value, json}};\n")
    } else {
        String::new()
    };
    let code = format!(
        "//! A call to `{f}`. Add `mod example;` to the generated `mod.rs` to compile it.\n\n#![allow(dead_code)]\n\nuse super::operations::{f};\n{serde}\npub async fn example() {{\n    let client = {runtime}::Client::builder(\"{base_url}\").build();\n    let result = {f}(\n        {},\n    )\n    .await;\n    match result {{\n        Ok(success) => println!(\"{{}} {{:?}}\", success.status, success.data),\n        Err(failure) => eprintln!(\"{{failure:?}}\"),\n    }}\n}}\n",
        args.join(",\n        "),
        f = function.identifier,
    );
    Some(tsurara_backend::Example {
        file: "example.rs".into(),
        code,
        note: Some("add `mod example;` to the generated `mod.rs`".into()),
    })
}

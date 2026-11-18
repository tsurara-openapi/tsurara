//! Mock factories and routes (`rust.mocks`, design.md 18.7.1) in the module `mocks`, which
//! `mod.rs` declares under `#[cfg(any(test, feature = "mocks"))]`: `mocks/schemas.rs` decodes each
//! declaration from its sample value, and `mocks/operations.rs` has, per operation, an enum of the
//! documented responses and a function that adds its route to a `MockTransport` (the runtime's
//! `testing` feature).

use crate::HEADER;
use crate::print::{Assigned, Printer};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_backend::manifest::OperationEntry;
use tsurara_lower::tir::{Module, Origin};
use tsurara_naming::pascal_case;
use tsurara_sem::operation::{BodyKind, Status};
use tsurara_sem::{Model, Warning};

/// A raw string literal that holds `text`.
fn raw(text: &str) -> String {
    let mut hashes = 1;
    while text.contains(&format!("\"{}", "#".repeat(hashes))) {
        hashes += 1;
    }
    let h = "#".repeat(hashes);
    format!("r{h}\"{text}\"{h}")
}

pub(crate) fn files(
    model: &Model,
    module: &Module,
    names: &Assigned,
    newtypes: &HashMap<usize, bool>,
    entries: &[OperationEntry],
    rt: &str,
    warnings: &mut Vec<Warning>,
) -> Vec<OutputFile> {
    let (samples, _) = tsurara_sample::samples(model);
    let sample_of: HashMap<_, _> = samples.types.iter().map(|(id, v)| (*id, v)).collect();
    let writable: BTreeSet<_> = model
        .schemas
        .iter()
        .filter(|d| d.writable)
        .map(|d| d.id)
        .collect();

    let mut schemas = format!(
        "{HEADER}\n\n//! Values of the declarations with the sample values of the definition. Update fields with\n//! struct update syntax: `Pet {{ name: \"Tama\".into(), ..mock_pet() }}`.\n\n#![allow(dead_code)]\n\n#[allow(unused_imports)]\nuse super::super::types::*;\n"
    );
    for (i, d) in module.decls.iter().enumerate() {
        let Origin::Schema(id) = d.origin else {
            continue;
        };
        if writable.contains(&id) {
            continue;
        }
        let Some(sample) = sample_of.get(&id) else {
            continue;
        };
        let ty = &names.types[i];
        write!(
            schemas,
            "\n/// A `{ty}` with the sample values of the definition.\npub fn mock_{}() -> {ty} {{\n    {rt}::serde_json::from_str({}).expect(\"the sample of {ty} decodes\")\n}}\n",
            names.files[i],
            raw(&sample.to_string())
        )
        .unwrap();
    }

    let mut operations = format!(
        "{HEADER}\n\n//! Routes for `MockTransport` (the runtime's `testing` feature): per operation, an enum of the\n//! documented responses and a function that adds the route; `routes` adds every operation with its\n//! first documented success and the sample values.\n\n#![allow(dead_code, clippy::let_and_return)]\n\n#[allow(unused_imports)]\nuse super::super::types::*;\n#[allow(unused_imports)]\nuse {rt}::testing::{{MockRequest, MockResponse, MockTransport}};\n"
    );
    let mut defaults = Vec::new();
    for ((op, types), entry) in model.operations.iter().zip(&module.operations).zip(entries) {
        let Some(function) = &entry.function else {
            continue;
        };
        let fname = &function.identifier;
        let enum_name = format!(
            "{}MockResponse",
            pascal_case(fname.trim_start_matches("r#"))
        );
        let mut p = Printer {
            names,
            newtypes,
            rt,
            warnings,
            loc: op.loc.clone(),
            used: Default::default(),
        };
        let mut variants = Vec::new();
        let mut arms = Vec::new();
        for (r, exprs) in op.responses.iter().zip(&types.responses) {
            let (variant, status_field, status) = match r.status {
                Status::Code(c) => (format!("Status{c}"), false, c.to_string()),
                Status::Range(d) => (format!("Status{d}xx"), true, "status".to_owned()),
                Status::Default => ("Default".to_owned(), true, "status".to_owned()),
            };
            let content = r.contents.first().zip(exprs.first());
            let (ty, response) = match content {
                Some((c, e)) if c.body_kind != BodyKind::None => match c.body_kind {
                    BodyKind::Json | BodyKind::Auto => (
                        Some(p.ty(e)),
                        format!("MockResponse::json({status}, &body)"),
                    ),
                    BodyKind::Text => (
                        Some("String".to_owned()),
                        format!("MockResponse::text({status}, {:?}, &body)", c.media_type),
                    ),
                    _ => (
                        Some("Vec<u8>".to_owned()),
                        format!(
                            "MockResponse {{ status: {status}, headers: vec![(\"content-type\".into(), {:?}.into())], body }}",
                            c.media_type
                        ),
                    ),
                },
                _ => (None, format!("MockResponse::new({status})")),
            };
            let (fields, pattern) = match (status_field, &ty) {
                (true, Some(t)) => (format!("(u16, {t})"), "(status, body)"),
                (true, None) => ("(u16)".to_owned(), "(status)"),
                (false, Some(t)) => (format!("({t})"), "(body)"),
                (false, None) => (String::new(), ""),
            };
            variants.push(format!(
                "    /// `{}`.\n    {variant}{fields},",
                r.status.key()
            ));
            arms.push(format!(
                "            Self::{variant}{pattern} => {response},"
            ));
        }
        if variants.is_empty() {
            continue;
        }
        write!(
            operations,
            "\n/// A response of `{fname}` that the definition documents.\n#[derive(Debug, Clone)]\npub enum {enum_name} {{\n{}\n}}\n\nimpl {enum_name} {{\n    pub fn into_response(self) -> MockResponse {{\n        match self {{\n{}\n        }}\n    }}\n}}\n\n/// Answers `{}` with `respond`.\npub fn {fname}(\n    mock: MockTransport,\n    respond: impl Fn(&MockRequest) -> {enum_name} + Send + Sync + 'static,\n) -> MockTransport {{\n    mock.route({:?}, {:?}, move |request| Ok(respond(request).into_response()))\n}}\n",
            variants.join("\n"),
            arms.join("\n"),
            tsurara_sem::text::single_line(&format!("{} {}", op.method, op.path)),
            op.method,
            op.path,
        )
        .unwrap();

        // The default: the first documented success with a sample (or without a body).
        let index = model.operations.iter().position(|o| o.id == op.id);
        let sample = index.and_then(|i| samples.operations.get(i));
        let default = op
            .responses
            .iter()
            .filter(|r| r.status.is_success() == Some(true))
            .find_map(|r| {
                let (variant, status) = match r.status {
                    Status::Code(c) => (format!("Status{c}"), None),
                    Status::Range(d) => (format!("Status{d}xx"), Some(u16::from(d) * 100)),
                    Status::Default => ("Default".to_owned(), Some(200)),
                };
                let prefix = status.map(|s| format!("{s}, ")).unwrap_or_default();
                match r.contents.first() {
                    None => Some(match status {
                        Some(s) => format!("{enum_name}::{variant}({s})"),
                        None => format!("{enum_name}::{variant}"),
                    }),
                    Some(c) if c.body_kind == BodyKind::None => Some(match status {
                        Some(s) => format!("{enum_name}::{variant}({s})"),
                        None => format!("{enum_name}::{variant}"),
                    }),
                    Some(c) => {
                        let key = r.status.key();
                        let s = sample?
                            .responses
                            .iter()
                            .find(|s| s.status == key && s.media_type == c.media_type)?;
                        let body = match c.body_kind {
                            BodyKind::Json | BodyKind::Auto => format!(
                                "{rt}::serde_json::from_str({}).expect(\"the sample decodes\")",
                                raw(&s.value.to_string())
                            ),
                            BodyKind::Text => format!(
                                "{:?}.into()",
                                s.value
                                    .as_str()
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| s.value.to_string())
                            ),
                            _ => "Vec::new()".to_owned(),
                        };
                        Some(format!("{enum_name}::{variant}({prefix}{body})"))
                    }
                }
            });
        if let Some(d) = default {
            defaults.push(format!("    let mock = {fname}(mock, |_| {d});"));
        }
    }
    write!(
        operations,
        "\n/// Every operation with its first documented success and the sample values.\npub fn routes(mock: MockTransport) -> MockTransport {{\n{}\n    mock\n}}\n",
        defaults.join("\n")
    )
    .unwrap();

    vec![
        OutputFile {
            path: "mocks/mod.rs".into(),
            contents: format!(
                "{HEADER}\n\n//! Mock values and routes for tests (`rust.mocks`).\n\npub mod operations;\npub mod schemas;\n"
            ),
        },
        OutputFile {
            path: "mocks/schemas.rs".into(),
            contents: schemas,
        },
        OutputFile {
            path: "mocks/operations.rs".into(),
            contents: operations,
        },
    ]
}

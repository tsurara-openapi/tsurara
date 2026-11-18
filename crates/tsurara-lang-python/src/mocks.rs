//! Mock factories and routes (`python.mocks`, design.md 18.7.1), in the subpackage
//! `mocks`: `mocks/schemas.py` builds each declaration from its sample value with the package's
//! TypeDefs, and `mocks/operations.py` has, per operation, a constructor per documented status and a
//! route for `MockRouter` of `tsurara_client.testing`.

use crate::HEADER;
use crate::print::{Assigned, Printer, python_value};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_backend::manifest::OperationEntry;
use tsurara_lower::tir::{DeclKind, Expr, Module, Origin};
use tsurara_lower::typedef::type_ref;
use tsurara_sem::operation::{BodyKind, Status};
use tsurara_sem::{Model, Warning};

pub(crate) fn files(
    model: &Model,
    module: &Module,
    names: &Assigned,
    entries: &[OperationEntry],
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

    // Factories.
    let mut schemas = format!(
        "{HEADER}\n\"\"\"Values of the declarations with the sample values of the definition.\"\"\"\n\nfrom __future__ import annotations\n\nimport dataclasses\nfrom typing import Any\n\nfrom .._typedefs import CODEC\n"
    );
    let mut imports = Vec::new();
    let mut functions = String::new();
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
        let class = &names.classes[i];
        let function = format!("mock_{}", names.files[i]);
        imports.push(format!("from ..{} import {class}", names.files[i]));
        let decode = format!(
            "CODEC.decode({}, {})",
            python_value(&json!({ "ref": class }), 1),
            python_value(sample, 1)
        );
        if matches!(d.kind, DeclKind::Record(_)) {
            write!(
                functions,
                "\n\ndef {function}(**overrides: Any) -> {class}:\n    \"\"\"A `{class}` with the sample values; keyword arguments replace fields.\"\"\"\n    value: {class} = {decode}\n    return dataclasses.replace(value, **overrides)\n"
            )
            .unwrap();
        } else {
            write!(
                functions,
                "\n\ndef {function}() -> {class}:\n    \"\"\"A `{class}` with the sample values.\"\"\"\n    value: {class} = {decode}\n    return value\n"
            )
            .unwrap();
        }
    }
    imports.sort();
    for line in &imports {
        writeln!(schemas, "{line}").unwrap();
    }
    schemas.push_str(&functions);

    // Routes.
    let mut operations = format!(
        "{HEADER}\n\"\"\"Routes for `MockRouter` of `tsurara_client.testing`: one per operation, answered by default\nwith its first documented success and the sample values, and a constructor per documented status.\n\"\"\"\n\nfrom __future__ import annotations\n\nfrom collections.abc import Callable\nfrom typing import Any\n\nfrom tsurara_client.testing import MockRequest, MockResponse, MockRoute\n\nfrom .._typedefs import CODEC\n"
    );
    let mut op_imports = BTreeSet::new();
    let mut body = String::new();
    let mut defaults = Vec::new();
    for ((op, types), entry) in model.operations.iter().zip(&module.operations).zip(entries) {
        let Some(function) = &entry.function else {
            continue;
        };
        let file = function
            .file
            .strip_suffix(".py")
            .unwrap_or(&function.file)
            .replace('/', ".");
        let def = format!("{}_DEF", function.identifier.to_uppercase());
        op_imports.insert(format!("from ..{file} import DEF as {def}"));
        let fname = &function.identifier;
        let mut p = Printer {
            module,
            names,
            typing: BTreeSet::new(),
            collections: BTreeSet::new(),
            refs: BTreeSet::new(),
            externals: BTreeSet::new(),
            warnings,
            missing: false,
            file: false,
            loc: op.loc.clone(),
        };
        for (r, exprs) in op.responses.iter().zip(&types.responses) {
            let key = r.status.key().to_lowercase();
            let (status_param, status) = match r.status {
                Status::Code(c) => (String::new(), c.to_string()),
                _ => ("status: int, ".to_owned(), "status".to_owned()),
            };
            let content = r.contents.first().zip(exprs.first());
            match content {
                Some((c, e)) if c.body_kind != BodyKind::None => {
                    let (ann, value) = match c.body_kind {
                        BodyKind::Json | BodyKind::Auto => {
                            let ann = p.ann(e);
                            let encode = match reference(e, names) {
                                Some(r) => format!("CODEC.encode({}, body)", python_value(&r, 1)),
                                None => "body".to_owned(),
                            };
                            (ann, format!("json={encode}"))
                        }
                        BodyKind::Text => (
                            "str".to_owned(),
                            format!(
                                "text=body, content_type={}",
                                python_value(&json!(c.media_type), 0)
                            ),
                        ),
                        _ => (
                            "bytes".to_owned(),
                            format!(
                                "content=body, content_type={}",
                                python_value(&json!(c.media_type), 0)
                            ),
                        ),
                    };
                    write!(
                        body,
                        "\n\ndef {fname}_{key}({status_param}body: {ann}) -> MockResponse:\n    \"\"\"A `{}` response of `{fname}`.\"\"\"\n    return MockResponse({status}, {value})\n",
                        r.status.key()
                    )
                    .unwrap();
                }
                _ => {
                    write!(
                        body,
                        "\n\ndef {fname}_{key}({}) -> MockResponse:\n    \"\"\"A `{}` response of `{fname}` without a body.\"\"\"\n    return MockResponse({status})\n",
                        status_param.trim_end_matches(", "),
                        r.status.key()
                    )
                    .unwrap();
                }
            }
        }
        for r in p.refs.iter() {
            op_imports.insert(format!(
                "from ..{} import {}",
                names.files[*r], names.classes[*r]
            ));
        }
        // The default: the first documented success with a sample (or without a body).
        let index = model.operations.iter().position(|o| o.id == op.id);
        let sample = index.and_then(|i| samples.operations.get(i));
        let default = op
            .responses
            .iter()
            .filter(|r| r.status.is_success() == Some(true))
            .find_map(|r| {
                let status = match r.status {
                    Status::Code(c) => c,
                    Status::Range(d) => u16::from(d) * 100,
                    Status::Default => 200,
                };
                match r.contents.first() {
                    None => Some(format!("MockResponse({status})")),
                    Some(c) if c.body_kind == BodyKind::None => {
                        Some(format!("MockResponse({status})"))
                    }
                    Some(c) => {
                        let key = r.status.key();
                        let s = sample?
                            .responses
                            .iter()
                            .find(|s| s.status == key && s.media_type == c.media_type)?;
                        Some(match c.body_kind {
                            BodyKind::Json | BodyKind::Auto => format!(
                                "MockResponse({status}, json={})",
                                python_value(&s.value, 2)
                            ),
                            BodyKind::Text => format!(
                                "MockResponse({status}, text={}, content_type={})",
                                python_value(&Value::String(text_of(&s.value)), 2),
                                python_value(&json!(c.media_type), 0)
                            ),
                            _ => format!(
                                "MockResponse({status}, content=b\"\", content_type={})",
                                python_value(&json!(c.media_type), 0)
                            ),
                        })
                    }
                }
            });
        let default_line = match &default {
            Some(d) => format!("        if response is None:\n            return {d}\n"),
            None => "        if response is None:\n            raise ValueError(\"no documented success to answer with\")\n".to_owned(),
        };
        let doc = crate::print::docstring(Some(&tsurara_sem::ir::Doc {
            description: Some(format!(
                "Answers `{} {}` with `response` (by default its first documented success with the samples).",
                op.method, op.path
            )),
            tags: Vec::new(),
        }))
        .expect("the description is not empty");
        write!(
            body,
            "\n\ndef {fname}_route(\n    response: MockResponse | Callable[[MockRequest], MockResponse] | None = None,\n) -> MockRoute:\n    {doc}\n\n    def handler(request: MockRequest) -> MockResponse:\n{default_line}        return response(request) if callable(response) else response\n\n    return ({def}, handler)\n",
        )
        .unwrap();
        if default.is_some() {
            defaults.push(format!("{fname}_route()"));
        }
    }
    for line in &op_imports {
        writeln!(operations, "{line}").unwrap();
    }
    operations.push_str(&body);
    write!(
        operations,
        "\n\ndef routes() -> list[MockRoute]:\n    \"\"\"Every operation with its first documented success, for `MockRouter.extend`.\"\"\"\n    return [{}]\n\n\n_ = Any\n",
        defaults.join(", ")
    )
    .unwrap();

    vec![
        OutputFile {
            path: "mocks/__init__.py".into(),
            contents: format!(
                "{HEADER}\n\"\"\"Mock values and routes for tests (`python.mocks`).\"\"\"\n"
            ),
        },
        OutputFile {
            path: "mocks/schemas.py".into(),
            contents: schemas,
        },
        OutputFile {
            path: "mocks/operations.py".into(),
            contents: operations,
        },
    ]
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

/// The TypeDef reference of a body, or `None` for values the runtime passes through.
fn reference(e: &Expr, names: &Assigned) -> Option<Value> {
    match e {
        Expr::Any | Expr::Null | Expr::Bool | Expr::Bytes | Expr::Never => None,
        Expr::Scalar(s) if s.format.is_none() => None,
        _ => Some(type_ref(e, &names.classes, "python")),
    }
}

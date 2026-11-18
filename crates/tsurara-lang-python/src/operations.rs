//! Call functions: one module per operation in `operations/`, with its OperationDef
//! (`DEF`), its Result type and a function that takes either client of the runtime (tsurara-client).

use crate::HEADER;
use crate::naming::{PythonPolicy, allocate};
use crate::print::{Assigned, Printer, docstring, python_value, union};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_backend::manifest::{Export, OperationEntry, OperationKind};
use tsurara_backend::operation_def::{TypeRefs, operation_def};
use tsurara_lower::tir::{Expr, Module};
use tsurara_lower::typedef::type_ref;
use tsurara_naming::{Context, Names, NamingPolicy, pascal_case};
use tsurara_sem::operation::{Operation, ParamIn};
use tsurara_sem::{Model, Warning};

/// The minimum version of the runtime the generated functions use.
pub const RUNTIME_REQUIREMENT: &str = "tsurara-client[http]>=0.1,<0.2";

/// Names the functions use themselves; parameters with these names get a suffix.
const FUNCTION_NAMES: &[&str] = &["client", "body", "content_type", "timeout", "context"];

pub(crate) fn print(
    model: &Model,
    module: &Module,
    names: &Assigned,
    warnings: &mut Vec<Warning>,
    manifest: &mut Vec<OperationEntry>,
) -> Vec<OutputFile> {
    let mut functions = Names::new();
    let mut files = Vec::new();
    let mut exports = Vec::new();
    for (op, types) in model.operations.iter().zip(&module.operations) {
        let ident = PythonPolicy.convert(Context::Function, &op.id, false);
        let function = functions
            .claim(
                &PythonPolicy,
                Context::Function,
                "functions",
                &ident,
                &op.id,
            )
            .name;
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
        let (code, result) = operation_module(&mut p, op, types, &function);
        let path = format!("operations/{function}.py");
        manifest.push(OperationEntry {
            helpers: Default::default(),
            operation_id: op.id.clone(),
            kind: OperationKind::Operation,
            function: Some(Export::new(&function, &path)),
            operation_def: Some(Export::new("DEF", &path)),
            types: [("result".to_owned(), Export::new(&result, &path))].into(),
        });
        exports.push((function.clone(), result));
        files.push(OutputFile {
            path,
            contents: code,
        });
    }
    exports.sort();
    let mut init = format!(
        "{HEADER}\n\"\"\"Call functions, one module per operation. They need the runtime: {RUNTIME_REQUIREMENT}.\"\"\"\n"
    );
    let schemes = !model.security_schemes.is_empty();
    if schemes {
        init.push_str("\nfrom typing import Literal, TypeAlias\n");
    }
    if !exports.is_empty() {
        init.push('\n');
    }
    for (function, result) in &exports {
        writeln!(init, "from .{function} import {result}, {function}").unwrap();
    }
    let scheme_name = "SecuritySchemeName".to_owned();
    if schemes {
        let names: Vec<String> = model
            .security_schemes
            .iter()
            .map(|n| serde_json::to_string(n).unwrap())
            .collect();
        writeln!(
            init,
            "\n# The security schemes of the API, as the client's `auth` hook receives them (`scheme.scheme`).\nSecuritySchemeName: TypeAlias = Literal[{}]",
            names.join(", ")
        )
        .unwrap();
    }
    init.push_str("\n__all__ = [\n");
    let mut all: Vec<&String> = exports.iter().flat_map(|(f, r)| [r, f]).collect();
    if schemes {
        all.push(&scheme_name);
    }
    all.sort();
    for name in all {
        writeln!(init, "    {},", serde_json::to_string(name).unwrap()).unwrap();
    }
    init.push_str("]\n");
    files.push(OutputFile {
        path: "operations/__init__.py".into(),
        contents: init,
    });
    files
}

/// A type reference for the OperationDef, or `None` for values the runtime passes through.
fn reference(e: &Expr, names: &Assigned) -> Option<Value> {
    match e {
        Expr::Any | Expr::Null | Expr::Bool | Expr::Bytes | Expr::Never => None,
        Expr::Scalar(s) if s.format.is_none() => None,
        _ => Some(type_ref(e, &names.classes, "python")),
    }
}

/// The annotation of a response body: binary bodies are read as `bytes`.
fn response_ann(p: &mut Printer, e: &Expr) -> String {
    match e {
        Expr::Bytes => "bytes".into(),
        Expr::Union(ts) => union(ts.iter().map(|t| response_ann(p, t)).collect()),
        _ => p.ann(e),
    }
}

struct Param {
    /// The keyword argument.
    arg: String,
    wire: String,
    location: ParamIn,
    ann: String,
    required: bool,
}

fn operation_module(
    p: &mut Printer,
    op: &Operation,
    types: &tsurara_lower::tir::OperationTypes,
    function: &str,
) -> (String, String) {
    // Keyword arguments, after the names the function uses itself.
    let mut raws: Vec<(&str, bool)> = FUNCTION_NAMES.iter().map(|n| (*n, false)).collect();
    raws.extend(op.params.iter().map(|q| (q.name.as_str(), false)));
    let args = allocate(Context::Member, &raws);
    let params: Vec<Param> = op
        .params
        .iter()
        .zip(&types.params)
        .zip(&args[FUNCTION_NAMES.len()..])
        .map(|((q, e), arg)| Param {
            arg: arg.clone(),
            wire: q.name.clone(),
            location: q.location,
            ann: p.ann(e),
            required: q.required,
        })
        .collect();

    // The Result: successes, documented errors, and what every operation can return.
    let has_success = op
        .responses
        .iter()
        .any(|r| r.status.is_success() == Some(true));
    // Sequences are read item by item: an iterator with `Client`, an async iterator with
    // `AsyncClient`, so an operation with one has a Result for each (design.md 8.5).
    let streams = types.sequences.iter().flatten().any(Option::is_some);
    let members = |p: &mut Printer, iterator: &str| -> Vec<String> {
        let mut success = Vec::new();
        let mut errors = Vec::new();
        for ((r, exprs), seqs) in op
            .responses
            .iter()
            .zip(&types.responses)
            .zip(&types.sequences)
        {
            let anns: Vec<String> = if exprs.is_empty() {
                vec!["None".into()]
            } else {
                exprs
                    .iter()
                    .zip(seqs)
                    .map(|(e, s)| match s {
                        Some(_) => {
                            p.collections.insert(iterator.to_owned());
                            format!("{iterator}[{}]", p.ann(e))
                        }
                        None => response_ann(p, e),
                    })
                    .collect()
            };
            match r.status.is_success() {
                Some(true) => success.extend(anns),
                Some(false) => errors.extend(anns),
                None if has_success => errors.extend(anns),
                None => success.extend(anns),
            }
        }
        let mut out = Vec::new();
        if !success.is_empty() {
            out.push(format!("Success[{}]", union(success)));
        }
        if !errors.is_empty() {
            out.push(format!("HttpError[{}]", union(errors)));
        }
        out.push("UndocumentedStatus | TransportFailure".into());
        out
    };
    let result_members = members(p, "Iterator");
    let async_members = streams.then(|| members(p, "AsyncIterator"));
    let referenced: BTreeSet<&str> = p
        .refs
        .iter()
        .map(|r| p.names.classes[*r].as_str())
        .collect();
    let mut result = format!("{}Result", pascal_case(function));
    if referenced.contains(result.as_str()) {
        result = format!("{}OperationResult", pascal_case(function));
    }
    let mut async_result = format!("{}AsyncResult", pascal_case(function));
    if referenced.contains(async_result.as_str()) {
        async_result = format!("{}AsyncOperationResult", pascal_case(function));
    }

    // The body.
    let body = op.body.as_ref();
    let body_ann = union(types.body.iter().map(|e| p.ann(e)).collect());
    let media: Vec<&str> = body
        .map(|b| b.contents.iter().map(|c| c.media_type.as_str()).collect())
        .unwrap_or_default();

    // The OperationDef, with TypeDef references.
    let refs = TypeRefs {
        params: types.params.iter().map(|e| reference(e, p.names)).collect(),
        body: body
            .map(|b| {
                b.contents
                    .iter()
                    .zip(&types.body)
                    .map(|(c, e)| {
                        let binary = c.media_type.starts_with("text/") || *e == Expr::Bytes;
                        if binary { None } else { reference(e, p.names) }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        responses: types
            .responses
            .iter()
            .map(|rs| rs.iter().map(|e| reference(e, p.names)).collect())
            .collect(),
    };
    let def = operation_def(op, Some(&refs));

    // The signature: required arguments first, then optional ones.
    let mut sig = vec!["client: {CLIENT}".to_owned(), "*".to_owned()];
    for q in params.iter().filter(|q| q.required) {
        sig.push(format!("{}: {}", q.arg, q.ann));
    }
    let body_required = body.is_some_and(|b| b.required);
    if body.is_some() {
        if body_required {
            sig.push(format!("body: {body_ann}"));
        } else {
            sig.push(format!(
                "body: {} = None",
                union(vec![body_ann.clone(), "None".into()])
            ));
        }
    }
    for q in params.iter().filter(|q| !q.required) {
        sig.push(format!(
            "{}: {} = None",
            q.arg,
            union(vec![q.ann.clone(), "None".into()])
        ));
    }
    if media.len() > 1 {
        p.typing.insert("Literal");
        let lits: Vec<String> = media
            .iter()
            .map(|m| serde_json::to_string(m).unwrap())
            .collect();
        sig.push(format!(
            "content_type: Literal[{}] | None = None",
            lits.join(", ")
        ));
    }
    sig.push("timeout: float | None = None".into());
    sig.push("context: Any = None".into());

    // The call.
    let mut call = vec!["DEF".to_owned()];
    let mut support = false;
    for (location, keyword) in [
        (ParamIn::Path, "path"),
        (ParamIn::Query, "query"),
        (ParamIn::Header, "headers"),
        (ParamIn::Cookie, "cookies"),
    ] {
        let group: Vec<&Param> = params.iter().filter(|q| q.location == location).collect();
        if group.is_empty() {
            continue;
        }
        let entries: Vec<String> = group
            .iter()
            .map(|q| format!("{}: {}", serde_json::to_string(&q.wire).unwrap(), q.arg))
            .collect();
        let dict = format!("{{{}}}", entries.join(", "));
        if group.iter().all(|q| q.required) {
            call.push(format!("{keyword}={dict}"));
        } else {
            support = true;
            call.push(format!("{keyword}=present({dict})"));
        }
    }
    let mut missing = false;
    if let Some(q) = params.iter().find(|q| q.location == ParamIn::QueryString) {
        if q.required {
            call.push(format!("querystring={}", q.arg));
        } else {
            missing = true;
            call.push(format!(
                "querystring=MISSING if {0} is None else {0}",
                q.arg
            ));
        }
    }
    if body.is_some() {
        if body_required {
            call.push("body=body".into());
        } else {
            missing = true;
            call.push("body=MISSING if body is None else body".into());
        }
    }
    if media.len() > 1 {
        call.push("content_type=content_type".into());
    }
    call.extend([
        "type_defs=CODEC".to_owned(),
        "timeout=timeout".to_owned(),
        "context=context".to_owned(),
    ]);

    // The module.
    let mut code = format!("{HEADER}\n\nfrom __future__ import annotations\n\n");
    let mut collections: BTreeSet<&str> = p.collections.iter().map(String::as_str).collect();
    collections.insert("Awaitable");
    let collections: Vec<&str> = collections.into_iter().collect();
    writeln!(
        code,
        "from collections.abc import {}",
        collections.join(", ")
    )
    .unwrap();
    let mut typing: BTreeSet<&str> = p.typing.clone();
    typing.extend(["Any", "TYPE_CHECKING", "TypeAlias", "overload"]);
    let typing: Vec<&str> = typing.into_iter().collect();
    writeln!(code, "from typing import {}\n", typing.join(", ")).unwrap();
    let mut runtime = vec!["AsyncClient", "Client", "OperationDef"];
    if p.file {
        runtime.push("File");
    }
    if missing {
        runtime.push("MISSING");
    }
    runtime.sort_unstable();
    writeln!(code, "from tsurara_client import {}", runtime.join(", ")).unwrap();
    if support {
        code.push_str("from tsurara_client.support import present\n");
    }
    code.push_str("\nfrom .._typedefs import CODEC\n");
    code.push_str("\nif TYPE_CHECKING:\n");
    code.push_str(
        "    from tsurara_client import HttpError, Success, TransportFailure, UndocumentedStatus\n",
    );
    for (from, name) in &p.externals {
        writeln!(code, "    from {from} import {name}").unwrap();
    }
    let mut imports: Vec<(&str, &str)> = p
        .refs
        .iter()
        .map(|r| (p.names.files[*r].as_str(), p.names.classes[*r].as_str()))
        .collect();
    imports.sort_unstable();
    imports.dedup();
    if !imports.is_empty() {
        code.push('\n');
    }
    for (file, class) in imports {
        writeln!(code, "    from ..{file} import {class}").unwrap();
    }
    writeln!(
        code,
        "\n{result}: TypeAlias = {}",
        serde_json::to_string(&result_members.join(" | ")).unwrap()
    )
    .unwrap();
    let awaited = match &async_members {
        Some(members) => {
            writeln!(
                code,
                "{async_result}: TypeAlias = {}",
                serde_json::to_string(&members.join(" | ")).unwrap()
            )
            .unwrap();
            async_result
        }
        None => result.clone(),
    };
    writeln!(code, "\nDEF: OperationDef = {}", python_value(&def, 0)).unwrap();
    let params_text = sig.join(", ");
    for (client, returns) in [
        ("Client[Any]", result.clone()),
        ("AsyncClient[Any]", format!("Awaitable[{awaited}]")),
    ] {
        writeln!(
            code,
            "\n\n@overload\ndef {function}({}) -> {returns}: ...",
            params_text.replace("{CLIENT}", client)
        )
        .unwrap();
    }
    write!(
        code,
        "\n\ndef {function}({}) -> Any:\n",
        params_text.replace("{CLIENT}", "Client[Any] | AsyncClient[Any]")
    )
    .unwrap();
    let mut doc = op.doc.clone().unwrap_or_default();
    if doc.description.is_none() {
        doc.description = Some(format!("{} {}", op.method, op.path));
    }
    if let Some(d) = docstring(Some(&doc)) {
        let indented: Vec<String> = d
            .lines()
            .map(|l| {
                if l.is_empty() {
                    String::new()
                } else {
                    format!("    {l}")
                }
            })
            .collect();
        writeln!(code, "{}", indented.join("\n")).unwrap();
    }
    code.push_str("    return client.request(\n");
    for c in &call {
        writeln!(code, "        {c},").unwrap();
    }
    code.push_str("    )\n");
    (code, result)
}

/// A call example for `tsurara inspect operation`: `example.py` at the package root, which
/// calls the function with the operation's sample values and matches the Result.
pub(crate) fn example(
    module: &Module,
    names: &Assigned,
    input: &tsurara_backend::ExampleInput,
) -> Option<tsurara_backend::Example> {
    let base_url = tsurara_backend::example_base_url(input.model);
    let op = input.operation;
    let index = input.model.operations.iter().position(|o| o.id == op.id)?;
    let types = module.operations.get(index)?;
    let entry = input
        .manifest
        .operations
        .iter()
        .find(|e| e.operation_id == op.id)?;
    let function = entry.function.as_ref()?;
    let mut raws: Vec<(&str, bool)> = FUNCTION_NAMES.iter().map(|n| (*n, false)).collect();
    raws.extend(op.params.iter().map(|q| (q.name.as_str(), false)));
    let args = allocate(Context::Member, &raws);
    let mut codec = false;
    let mut cast = false;
    let value = |e: &Expr, v: &Value, codec: &mut bool| match reference(e, names) {
        None => python_value(v, 3),
        Some(r) => {
            *codec = true;
            format!(
                "CODEC.decode({}, {})",
                python_value(&r, 3),
                python_value(v, 3)
            )
        }
    };
    let mut call_args = Vec::new();
    for s in input.samples.params.iter().filter(|p| p.required) {
        let Some(i) = op
            .params
            .iter()
            .position(|p| p.name == s.name && p.location == s.location)
        else {
            continue;
        };
        let arg = &args[FUNCTION_NAMES.len() + i];
        call_args.push(format!(
            "{arg}={}",
            value(&types.params[i], &s.value, &mut codec)
        ));
    }
    if let (Some(sample), Some(body)) = (&input.samples.body, &op.body) {
        let i = body
            .contents
            .iter()
            .position(|c| c.media_type == sample.media_type)?;
        let json = matches!(
            body.contents[i].body_kind,
            tsurara_sem::operation::BodyKind::Json | tsurara_sem::operation::BodyKind::Auto
        );
        if json {
            call_args.push(format!(
                "body={}",
                value(&types.body[i], &sample.value, &mut codec)
            ));
        } else if sample.required {
            cast = true;
            call_args.push("body=cast(Any, None),  # bytes or form fields".into());
        }
    }
    let mut code = String::new();
    if cast {
        code.push_str("from typing import Any, cast\n\n");
    }
    code.push_str("from tsurara_client import Client, HttpError, Success\n\n");
    if codec {
        code.push_str("from ._typedefs import CODEC\n");
    }
    let module_path = function
        .file
        .strip_suffix(".py")
        .unwrap_or(&function.file)
        .replace('/', ".");
    writeln!(code, "from .{module_path} import {}", function.identifier).unwrap();
    let call = if call_args.is_empty() {
        format!("{}(client)", function.identifier)
    } else {
        format!(
            "{}(\n            client,\n            {},\n        )",
            function.identifier,
            call_args.join(",\n            ")
        )
    };
    write!(
        code,
        "\n\ndef example() -> None:\n    with Client(\"{base_url}\") as client:\n        result = {call}\n        match result:\n            case Success(data=data):\n                print(data)\n            case HttpError(status=status, error=error):\n                print(status, error)\n            case other:\n                print(\"failed:\", other.kind)\n"
    )
    .unwrap();
    Some(tsurara_backend::Example {
        file: "example.py".into(),
        code,
        note: None,
    })
}

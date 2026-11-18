//! Call functions: one file per operation with its OperationDef, an options struct for
//! what is optional, and a function that calls `tsurara.Do`.

use crate::naming::{GoPolicy, allocate, exported};
use crate::print::{Line, Printer, aligned, comment, quote};
use std::fmt::Write as _;
use tsurara_backend::operation_def::operation_def;
use tsurara_lower::tir::{Expr, OperationTypes};
use tsurara_naming::{Context, Names, NamingPolicy, words};
use tsurara_sem::operation::{BodyKind, Operation, ParamIn};

/// Names an argument may not take: Go's keywords and predeclared identifiers, and the names the
/// function uses itself.
const TAKEN: &[&str] = &[
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
    "any",
    "append",
    "bool",
    "byte",
    "cap",
    "clear",
    "close",
    "comparable",
    "complex",
    "complex64",
    "complex128",
    "copy",
    "delete",
    "error",
    "false",
    "float32",
    "float64",
    "imag",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "iota",
    "len",
    "make",
    "max",
    "min",
    "new",
    "nil",
    "panic",
    "print",
    "println",
    "real",
    "recover",
    "rune",
    "string",
    "true",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "ctx",
    "c",
    "opts",
    "in",
    "b",
    "body",
    "tsurara",
    "context",
    "sync",
    "time",
];

/// Unexported lower camel case for arguments: the first word in lower case (`petID`, `urlPath`).
fn unexported(raw: &str) -> String {
    let ws = words(raw);
    let Some((first, rest)) = ws.split_first() else {
        return "value".into();
    };
    let mut out = first.to_lowercase();
    if !rest.is_empty() {
        out.push_str(&exported(&rest.join(" "), ""));
    }
    match out.chars().next() {
        Some(c) if c.is_ascii_digit() => format!("v{out}"),
        Some(c) if !c.is_lowercase() => format!("x{out}"),
        _ => out,
    }
}

fn arguments(raws: &[&str]) -> Vec<String> {
    let mut taken: std::collections::HashSet<String> =
        TAKEN.iter().map(|s| s.to_string()).collect();
    raws.iter()
        .map(|raw| {
            let base = unexported(raw);
            let mut name = base.clone();
            let mut n = 2;
            while !taken.insert(name.clone()) {
                name = format!("{base}{n}");
                n += 1;
            }
            name
        })
        .collect()
}

/// A field name for a request media type.
fn media_field(media: &str) -> String {
    let essence = media.split(';').next().unwrap_or(media).trim();
    match essence {
        "application/json" => "JSON".into(),
        "application/x-www-form-urlencoded" => "Form".into(),
        "text/plain" => "Text".into(),
        "application/octet-stream" => "OctetStream".into(),
        m if m.starts_with("multipart/") => "Multipart".into(),
        m if m.ends_with("+json") => format!("{}JSON", exported(m.trim_end_matches("+json"), "")),
        m => exported(m, "Media"),
    }
}

/// The type of a result side: one type (the zero value when there is no content), otherwise any.
fn result_type(mut types: Vec<String>) -> String {
    types.sort();
    types.dedup();
    if types.len() > 1 {
        types.retain(|t| t != "struct{}");
    }
    match types.as_slice() {
        [one] => one.clone(),
        _ => "any".into(),
    }
}

/// A nil-able type for an optional value, and whether its value is reached through `*`.
fn optional(ty: String) -> (String, bool) {
    if ty.starts_with('*') || ty.starts_with("[]") || ty.starts_with("map[") || ty == "any" {
        (ty, false)
    } else {
        (format!("*{ty}"), true)
    }
}

/// A Go string literal for an OperationDef: raw unless it holds a backquote.
fn literal(text: &str) -> String {
    if text.contains('`') {
        quote(text)
    } else {
        format!("`{text}`")
    }
}

/// The names an operation takes in the package scope: its function and the types it declares.
pub(crate) struct OperationNames {
    pub(crate) function: String,
    pub(crate) options: String,
    pub(crate) body: Option<String>,
}

/// Claims the names of an operation after the types and constants.
pub(crate) fn claim(op: &Operation, scope: &mut Names) -> OperationNames {
    let pointer = format!("#{}", op.loc.pointer);
    let mut take = |context: Context, raw: &str, suffix: &str| {
        let id = GoPolicy.convert(context, raw, false);
        scope
            .claim(
                &GoPolicy,
                context,
                "package",
                &id,
                &format!("{pointer}/{suffix}"),
            )
            .name
    };
    let function = take(Context::Function, &op.id, "function");
    let options = take(Context::Type, &format!("{function}Options"), "options");
    let body = op
        .body
        .as_ref()
        .filter(|b| b.contents.len() > 1)
        .map(|_| take(Context::Type, &format!("{function}Body"), "body"));
    OperationNames {
        function,
        options,
        body,
    }
}

struct Param {
    field: String,
    arg: String,
    wire: String,
    location: ParamIn,
    ty: String,
    required: bool,
}

pub(crate) fn operation(
    p: &mut Printer,
    op: &Operation,
    types: &OperationTypes,
    names: &OperationNames,
) -> String {
    let function = &names.function;
    let options = &names.options;
    let rt = |p: &mut Printer, item: &str| {
        p.uses_rt = true;
        format!("tsurara.{item}")
    };

    // Parameters: required ones are arguments, optional ones fields of the options.
    let mut raws: Vec<&str> = vec!["Body", "Timeout"];
    raws.extend(op.params.iter().map(|q| q.name.as_str()));
    let fields = allocate(Context::Member, &raws);
    let params_raw: Vec<&str> = op.params.iter().map(|q| q.name.as_str()).collect();
    let args = arguments(&params_raw);
    let params: Vec<Param> = op
        .params
        .iter()
        .zip(&types.params)
        .zip(fields[2..].iter().zip(&args))
        .map(|((q, e), (field, arg))| {
            // An optional parameter is left out with nil; a null it allows is not sent apart.
            let e = match e {
                Expr::Nullable(inner) if !q.required => inner.as_ref(),
                e => e,
            };
            Param {
                field: field.clone(),
                arg: arg.clone(),
                wire: q.name.clone(),
                location: q.location,
                ty: p.ty(e),
                required: q.required,
            }
        })
        .collect();

    // The body: one type, or a struct with a field per media type.
    let body = op.body.as_ref();
    let body_required = body.is_some_and(|b| b.required);
    let mut code = String::new();
    let body_type = match (body, &names.body) {
        (Some(b), Some(name)) => {
            let raws: Vec<String> = b
                .contents
                .iter()
                .map(|c| media_field(&c.media_type))
                .collect();
            let refs: Vec<&str> = raws.iter().map(String::as_str).collect();
            let fields = allocate(Context::Member, &refs);
            let mut lines = Vec::new();
            for ((c, e), f) in b.contents.iter().zip(&types.body).zip(&fields) {
                lines.push(Line::Text(format!(
                    "// {}",
                    crate::print::comment_inline(&c.media_type)
                )));
                let (ty, _) = optional(p.ty(e));
                lines.push(Line::Cells(vec![f.clone(), ty]));
            }
            writeln!(
                code,
                "// {name} is the body of {function} in one of its media types: set one field.\ntype {name} struct {{\n{}}}\n",
                aligned(&lines, "\t")
            )
            .unwrap();
            Some((name.clone(), Some(fields)))
        }
        (Some(_), None) => Some((p.ty(&types.body[0]), None)),
        _ => None,
    };
    // How a body value of a media type goes into the input.
    let encode = |p: &mut Printer, media: &str, e: &Expr, value: &str| -> String {
        let _ = p;
        if media.to_ascii_lowercase().starts_with("multipart/") {
            format!("in.SetMultipart({value})")
        } else if *e == Expr::Bytes {
            format!("in.SetFile({value})")
        } else {
            format!("in.SetBody({value})")
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
    // `*tsurara.Items[T]` of their item type (design.md 8.5), as in Rust.
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
        // A response with a JSON media type is read as JSON, as in Rust.
        let json = r.contents.iter().any(|c| c.body_kind == BodyKind::Json);
        let tys: Vec<String> = if exprs.is_empty() {
            vec!["struct{}".into()]
        } else {
            r.contents
                .iter()
                .zip(exprs)
                .filter(|(c, _)| !json || c.body_kind == BodyKind::Json)
                .map(|(_, e)| match e {
                    Expr::Bytes => "[]byte".into(),
                    e => p.ty(e),
                })
                .collect()
        };
        match r.status.is_success() {
            Some(true) => success.extend(tys),
            Some(false) => errors.extend(tys),
            None if has_success => errors.extend(tys),
            None => success.extend(tys),
        }
    }
    let item = result_type(item_types);
    let success = if items {
        rt(p, &format!("Items[{item}]"))
    } else {
        result_type(success)
    };
    let error = result_type(errors);

    // The OperationDef, parsed once.
    let def = serde_json::to_string(&operation_def(op, None)).expect("JSON");
    let def_var = format!("{}Def", unexported(function));
    let def_type = rt(p, "OperationDef");
    let parse = rt(p, "ParseOperationDef");
    let mut out = format!(
        "var {def_var} = sync.OnceValue(func() *{def_type} {{\n\treturn {parse}({})\n}})\n\n",
        literal(&def)
    );
    out.push_str(&code);

    // The options.
    let mut lines = Vec::new();
    for q in params.iter().filter(|q| !q.required) {
        lines.push(Line::Cells(vec![q.field.clone(), optional(q.ty.clone()).0]));
    }
    if let (Some((t, _)), false) = (&body_type, body_required) {
        lines.push(Line::Cells(vec![fields[0].clone(), optional(t.clone()).0]));
    }
    lines.push(Line::Text(
        "// Timeout overrides the client's timeout.".into(),
    ));
    lines.push(Line::Cells(vec![fields[1].clone(), "time.Duration".into()]));
    writeln!(
        out,
        "// {options} are the optional inputs of {function}.\ntype {options} struct {{\n{}}}\n",
        aligned(&lines, "\t")
    )
    .unwrap();

    // The function.
    // Doc comments start with the function's name (golint, revive); the summary and description
    // follow as a paragraph.
    writeln!(
        out,
        "// {function} calls {}.",
        crate::print::comment_inline(&format!("{} {}", op.method, op.path))
    )
    .unwrap();
    let described = comment(op.doc.as_ref(), "");
    if !described.is_empty() {
        out.push_str("//\n");
        out.push_str(&described);
    }
    let client = rt(p, "Client");
    let mut sig = vec!["ctx context.Context".to_owned(), format!("c *{client}")];
    for q in params.iter().filter(|q| q.required) {
        sig.push(format!("{} {}", q.arg, q.ty));
    }
    if let (Some((t, _)), true) = (&body_type, body_required) {
        sig.push(format!("body {t}"));
    }
    sig.push(format!("opts *{options}"));
    let success_type = if items {
        rt(p, &format!("Success[*{success}]"))
    } else {
        rt(p, &format!("Success[{success}]"))
    };
    writeln!(
        out,
        "func {function}({}) (*{success_type}, error) {{\n\tif opts == nil {{\n\t\topts = &{options}{{}}\n\t}}\n\tin := &{}{{Timeout: opts.{}}}",
        sig.join(", "),
        rt(p, "Input"),
        fields[1]
    )
    .unwrap();
    for q in &params {
        let method = match q.location {
            ParamIn::Path => "PathParam",
            ParamIn::Query => "QueryParam",
            ParamIn::Header => "HeaderParam",
            ParamIn::Cookie => "CookieParam",
            ParamIn::QueryString => "SetQueryString",
        };
        let call = |value: &str| {
            if q.location == ParamIn::QueryString {
                format!("in.{method}({value})")
            } else {
                format!("in.{method}({}, {value})", quote(&q.wire))
            }
        };
        if q.required {
            writeln!(out, "\t{}", call(&q.arg)).unwrap();
        } else {
            let (_, deref) = optional(q.ty.clone());
            let field = format!("opts.{}", q.field);
            let value = if deref {
                format!("*{field}")
            } else {
                field.clone()
            };
            writeln!(out, "\tif {field} != nil {{\n\t\t{}\n\t}}", call(&value)).unwrap();
        }
    }
    if let (Some(b), Some((t, variants))) = (body, &body_type) {
        let indent = if body_required { "\t" } else { "\t\t" };
        let mut statements = String::new();
        let source = if body_required {
            "body".to_owned()
        } else {
            let (_, deref) = optional(t.clone());
            if deref {
                format!("*opts.{}", fields[0])
            } else {
                format!("opts.{}", fields[0])
            }
        };
        match variants {
            None => {
                writeln!(
                    statements,
                    "{indent}{}",
                    encode(p, &b.contents[0].media_type, &types.body[0], &source)
                )
                .unwrap();
            }
            Some(variants) => {
                writeln!(statements, "{indent}b := {source}\n{indent}switch {{").unwrap();
                for ((c, e), v) in b.contents.iter().zip(&types.body).zip(variants) {
                    let (_, deref) = optional(p.ty(e));
                    let value = if deref {
                        format!("*b.{v}")
                    } else {
                        format!("b.{v}")
                    };
                    writeln!(
                        statements,
                        "{indent}case b.{v} != nil:\n{indent}\t{}\n{indent}\tin.SetMediaType({})",
                        encode(p, &c.media_type, e, &value),
                        quote(&c.media_type)
                    )
                    .unwrap();
                }
                writeln!(statements, "{indent}}}").unwrap();
            }
        }
        if body_required {
            out.push_str(&statements);
        } else {
            writeln!(out, "\tif opts.{} != nil {{\n{statements}\t}}", fields[0]).unwrap();
        }
    }
    let run = if items {
        rt(p, &format!("DoItems[{item}, {error}]"))
    } else {
        rt(p, &format!("Do[{success}, {error}]"))
    };
    writeln!(out, "\treturn {run}(ctx, c, {def_var}(), in)\n}}").unwrap();
    out
}

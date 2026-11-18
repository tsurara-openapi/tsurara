//! Mock factories and request handlers (`typescript.mocks: msw | fetch`, design.md 8.9).
//!
//! Everything lives under `mocks/`, which the types, the functions and the barrel never import:
//!
//! - `mocks/schemas/{Name}.ts`: `mock{Name}()`, the sample value of the declaration (design.md
//!   18.7.1); object types take `overrides`. Named types inside that are their own sample are built
//!   by their factories.
//! - `mocks/operations/{function}.ts`: `{Op}MockResponse`, the union of documented statuses and
//!   bodies, and `{function}Handler(response?, options?)`, an MSW `HttpHandler` or a `MockRoute` for
//!   `mockFetch`.
//! - `mocks/support.ts`: the shared types and the function that turns a response into one.
//! - `mocks/index.ts`: `handlers` (MSW) or `routes` (fetch), every operation with its first success.

use crate::HEADER;
use crate::emit::{Unit, relative, render_file};
use crate::options::{Mocks, TypeScriptOptions};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_sem::ir::{Literal, Object, Property, Type};
use tsurara_sem::model::Model;
use tsurara_sem::naming::NameId;
use tsurara_sem::operation::{BodyKind, Operation, Status};
use tsurara_sem::print::{property_key, string_literal};
use tsurara_sem::schema::TypeDecl;

pub fn emit(
    original: &Model,
    lowered: &Model,
    operation_files: &[String],
    schema_files: &BTreeMap<String, String>,
    mocks: Mocks,
    opts: &TypeScriptOptions,
) -> Vec<OutputFile> {
    let (samples, _) = tsurara_sample::samples(original);
    let sample_of: HashMap<NameId, &Value> = samples.types.iter().map(|(id, v)| (*id, v)).collect();
    let names: HashMap<NameId, &str> = lowered
        .schemas
        .iter()
        .map(|d| (d.id, d.name.as_str()))
        .collect();
    let decls: HashMap<NameId, &TypeDecl> = original.schemas.iter().map(|d| (d.id, d)).collect();
    let ctx = Ctx {
        sample_of: &sample_of,
        names: &names,
        decls: &decls,
    };
    let mut out = Vec::new();

    // Files of values: factories, OperationDefs, the support module.
    let mut value_files: BTreeMap<String, String> = BTreeMap::new();
    for d in original.schemas.iter().filter(|d| !d.writable) {
        if sample_of.contains_key(&d.id) {
            let name = names[&d.id];
            value_files.insert(format!("mock{name}"), format!("mocks/schemas/{name}.ts"));
        }
    }
    for (op, file) in lowered.operations.iter().zip(operation_files) {
        if let Some(def) = op.type_names.get("{}Def") {
            value_files.insert(def.clone(), file.clone());
        }
    }
    value_files.insert("respond".into(), "mocks/support.ts".into());
    let mut type_files = schema_files.clone();
    for t in ["MockInfo", "MockOptions", "MockBodies"] {
        type_files.insert(t.into(), "mocks/support.ts".into());
    }

    // Factories.
    for d in original.schemas.iter().filter(|d| !d.writable) {
        let Some(sample) = sample_of.get(&d.id) else {
            continue;
        };
        let name = names[&d.id];
        let path = format!("mocks/schemas/{name}.ts");
        let mut unit = Unit::default();
        unit.refs.insert(name.to_owned());
        // A type with a construct a sample may not follow is written as plain JSON and asserted.
        let cast = deep_cast(&d.ty, &decls, &mut Default::default());
        let value = if cast {
            format!("({} as unknown as {name})", literal(sample, 1))
        } else {
            ctx.expr(&d.ty, sample, Some(d.id), &mut unit, 1)
        };
        // Overrides for objects with properties; `Partial` of an index signature adds `undefined`.
        let objectlike = matches!(
            &lowered_decl(lowered, d.id).ty,
            Type::Object(o) if o.index.is_none() && !o.properties.is_empty()
        );
        let code = if objectlike {
            // The sample's properties, then the overrides, in one object literal. A value that had
            // to be cast is cast to this type, so that it can be spread.

            let body = match value
                .strip_prefix("{\n")
                .and_then(|v| v.strip_suffix("\n  }"))
            {
                Some(fields) => format!("{{\n{fields}\n    ...overrides,\n  }}"),
                None => format!("{{ ...{value}, ...overrides }}"),
            };
            format!(
                "/** A `{name}` with the sample values of the definition; `overrides` replace properties. */\nexport function mock{name}(overrides?: Partial<{name}>): {name} {{\n  return {body};\n}}\n"
            )
        } else {
            format!(
                "/** A `{name}` with the sample values of the definition. */\nexport function mock{name}(): {name} {{\n  return {value};\n}}\n"
            )
        };
        unit.raw(&code);
        unit.declares.push(format!("mock{name}"));
        unit.has_values = true;
        out.push(OutputFile {
            contents: render_file(&path, &[unit], &type_files, &value_files, opts),
            path,
        });
    }

    // Handlers.
    let mut defaults = Vec::new();
    for (index, op) in lowered.operations.iter().enumerate() {
        let path = format!("mocks/operations/{}.ts", op.name);
        let sample = samples.operations.get(index);
        let (unit, has_default) =
            handler_unit(op, &original.operations[index], sample, &ctx, mocks, opts);
        if has_default {
            defaults.push((format!("{}Handler", op.name), path.clone()));
        }
        let mut contents = render_file(&path, &[unit], &type_files, &value_files, opts);
        let framework = match mocks {
            Mocks::Msw => "import { type HttpHandler, http } from 'msw';\n".to_owned(),
            Mocks::Fetch => format!(
                "import type {{ MockRoute }} from '{}/testing';\n",
                opts.client_module
            ),
        };
        contents = contents.replacen(
            &format!("{HEADER}\n\n"),
            &format!("{HEADER}\n\n{framework}"),
            1,
        );
        out.push(OutputFile { path, contents });
    }

    out.push(OutputFile {
        path: "mocks/support.ts".into(),
        contents: support(mocks, opts),
    });

    // The index.
    let ext = opts.import_extension.suffix();
    let mut index = format!("{HEADER}\n\n");
    match mocks {
        Mocks::Msw => index.push_str("import type { HttpHandler } from 'msw';\n"),
        Mocks::Fetch => writeln!(
            index,
            "import type {{ MockRoute }} from '{}/testing';",
            opts.client_module
        )
        .unwrap(),
    }
    for (handler, path) in &defaults {
        writeln!(
            index,
            "import {{ {handler} }} from '{}';",
            relative("mocks/index.ts", path, ext)
        )
        .unwrap();
    }
    let (name, ty) = match mocks {
        Mocks::Msw => ("handlers", "HttpHandler"),
        Mocks::Fetch => ("routes", "MockRoute"),
    };
    write!(
        index,
        "\n/** Every operation answered with its first documented success and the sample values. */\nexport const {name}: {ty}[] = [\n"
    )
    .unwrap();
    for (handler, _) in &defaults {
        writeln!(index, "  {handler}(),").unwrap();
    }
    index.push_str("];\n");
    out.push(OutputFile {
        path: "mocks/index.ts".into(),
        contents: index,
    });
    out
}

fn lowered_decl(model: &Model, id: NameId) -> &TypeDecl {
    model
        .schemas
        .iter()
        .find(|d| d.id == id)
        .expect("lowered declarations keep their ids")
}

struct Ctx<'a> {
    sample_of: &'a HashMap<NameId, &'a Value>,
    names: &'a HashMap<NameId, &'a str>,
    decls: &'a HashMap<NameId, &'a TypeDecl>,
}

impl Ctx<'_> {
    /// `value` (a sample of `ty`) as a TypeScript expression. A named type whose value is its own
    /// sample is built by its factory (except the declaration being written, `current`).
    fn expr(
        &self,
        ty: &Type,
        value: &Value,
        current: Option<NameId>,
        unit: &mut Unit,
        indent: usize,
    ) -> String {
        match ty {
            Type::Named(r) => {
                let own = self.sample_of.get(&r.id).copied();
                if Some(r.id) != current && own == Some(value) {
                    let name = self.names[&r.id];
                    unit.value_refs.insert(format!("mock{name}"));
                    return format!("mock{name}()");
                }
                match self.decls.get(&r.id) {
                    Some(d) => self.expr(&d.ty, value, current, unit, indent),
                    None => literal(value, indent),
                }
            }
            Type::Inline(i) => self.expr(&i.ty, value, current, unit, indent),
            Type::Mapped(_) | Type::External(_) => format!("({} as never)", literal(value, indent)),
            // A file part (`Blob | File` in TypeScript).
            Type::Binary => match value {
                Value::String(s) => format!("new Blob([{}])", string_literal(s)),
                _ => "new Blob()".to_owned(),
            },
            // Discriminators and `allOf` can make TypeScript types that a sample cannot follow (a
            // mapping that disagrees with the members' own tags gives `never`); their values are cast.
            _ if needs_cast(ty, self.decls, 0) => format!("({} as never)", literal(value, indent)),
            Type::Object(o) => match value {
                Value::Object(map) if !map.is_empty() => {
                    let pad = "  ".repeat(indent);
                    let fields: Vec<String> = map
                        .iter()
                        .map(|(k, v)| {
                            let inner = o
                                .properties
                                .iter()
                                .find(|p| p.name == *k)
                                .map(|p| self.expr(&p.ty, v, current, unit, indent + 1))
                                .unwrap_or_else(|| literal(v, indent + 1));
                            format!("{pad}  {}: {inner},", property_key(k))
                        })
                        .collect();
                    format!("{{\n{}\n{pad}}}", fields.join("\n"))
                }
                v => literal(v, indent),
            },
            Type::Array(item) => match value {
                Value::Array(items) => format!(
                    "[{}]",
                    items
                        .iter()
                        .map(|v| self.expr(item, v, current, unit, indent))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                v => literal(v, indent),
            },
            _ if contains_mapped(ty, self.decls, 0) => {
                format!("({} as never)", literal(value, indent))
            }
            _ => literal(value, indent),
        }
    }
}

/// Whether the value at a position of type `ty` must be cast: a discriminated union, an inherited tag
/// or an intersection, reached through unions and references (objects and arrays are written field
/// by field instead).
fn needs_cast(ty: &Type, decls: &HashMap<NameId, &TypeDecl>, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    match ty {
        Type::Choice(c) => {
            c.discriminator.is_some() || c.members.iter().any(|t| needs_cast(t, decls, depth + 1))
        }
        Type::Tag(_) | Type::Intersection(_) | Type::Mapped(_) | Type::External(_) => true,
        Type::Union(ts) => ts.iter().any(|t| needs_cast(t, decls, depth + 1)),
        Type::Named(r) => decls
            .get(&r.id)
            .is_some_and(|d| needs_cast(&d.ty, decls, depth + 1)),
        Type::Inline(i) => needs_cast(&i.ty, decls, depth + 1),
        _ => false,
    }
}

/// Whether a type contains, anywhere, a construct whose TypeScript type a sample may not follow
/// (`needs_cast`), through objects, arrays and references.
fn deep_cast(
    ty: &Type,
    decls: &HashMap<NameId, &TypeDecl>,
    seen: &mut std::collections::HashSet<NameId>,
) -> bool {
    if needs_cast(ty, decls, 0) {
        return true;
    }
    match ty {
        Type::Named(r) => {
            seen.insert(r.id)
                && decls
                    .get(&r.id)
                    .is_some_and(|d| deep_cast(&d.ty, decls, seen))
        }
        Type::Inline(i) => deep_cast(&i.ty, decls, seen),
        Type::Array(t) => deep_cast(t, decls, seen),
        Type::Tuple(t) => t.items.iter().any(|t| deep_cast(t, decls, seen)),
        Type::Union(ts) => ts.iter().any(|t| deep_cast(t, decls, seen)),
        Type::Choice(c) => c.members.iter().any(|t| deep_cast(t, decls, seen)),
        Type::Object(o) => {
            o.properties.iter().any(|p| deep_cast(&p.ty, decls, seen))
                || o.index.as_ref().is_some_and(|t| deep_cast(t, decls, seen))
        }
        _ => false,
    }
}

/// Whether a type contains a mapped or external type, whose sample may not have the external type.
fn contains_mapped(ty: &Type, decls: &HashMap<NameId, &TypeDecl>, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    match ty {
        Type::Mapped(_) | Type::External(_) => true,
        Type::Named(r) => decls
            .get(&r.id)
            .is_some_and(|d| contains_mapped(&d.ty, decls, depth + 1)),
        Type::Inline(i) => contains_mapped(&i.ty, decls, depth + 1),
        Type::Array(t) => contains_mapped(t, decls, depth + 1),
        Type::Union(ts) | Type::Intersection(ts) => {
            ts.iter().any(|t| contains_mapped(t, decls, depth + 1))
        }
        Type::Choice(c) => c
            .members
            .iter()
            .any(|t| contains_mapped(t, decls, depth + 1)),
        Type::Tuple(t) => t.items.iter().any(|t| contains_mapped(t, decls, depth + 1)),
        Type::Object(o) => o
            .properties
            .iter()
            .any(|p| contains_mapped(&p.ty, decls, depth + 1)),
        _ => false,
    }
}

/// A JSON value as a TypeScript literal (single-quoted strings, unquoted keys where possible).
fn literal(v: &Value, indent: usize) -> String {
    match v {
        Value::String(s) => string_literal(s),
        Value::Object(o) if o.is_empty() => "{}".into(),
        Value::Object(o) => {
            let pad = "  ".repeat(indent);
            let fields: Vec<String> = o
                .iter()
                .map(|(k, x)| format!("{pad}  {}: {},", property_key(k), literal(x, indent + 1)))
                .collect();
            format!("{{\n{}\n{pad}}}", fields.join("\n"))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter()
                .map(|x| literal(x, indent))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        v => v.to_string(),
    }
}

/// How a response body is written: its kind and media type.
fn body_kind(kind: BodyKind, media_type: &str) -> (&'static str, String) {
    let k = match kind {
        BodyKind::Json | BodyKind::Auto => "json",
        BodyKind::Text => "text",
        BodyKind::Blob | BodyKind::Stream => "binary",
        BodyKind::None => "none",
    };
    (k, media_type.to_owned())
}

/// The handler file of an operation; `true` when it has a default response (a documented success
/// with a sample).
fn handler_unit(
    op: &Operation,
    original: &Operation,
    samples: Option<&tsurara_sample::OperationSamples>,
    ctx: &Ctx,
    mocks: Mocks,
    opts: &TypeScriptOptions,
) -> (Unit, bool) {
    let mut unit = Unit::default();
    let pascal = {
        let mut c = op.name.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect::<String>())
            .unwrap_or_default()
    };
    let response_type = format!("{pascal}MockResponse");
    let handler = format!("{}Handler", op.name);

    // The documented responses.
    let mut members = Vec::new();
    let mut kinds = Vec::new();
    for r in &op.responses {
        let status = match r.status {
            Status::Code(c) => Type::Literal(Literal::Number(c.into())),
            Status::Range(_) | Status::Default => Type::Number,
        };
        let mut properties = vec![Property::new("status", status)];
        match r.contents.first() {
            Some(c) if c.body_kind != BodyKind::None => {
                // A stream is answered with its raw bytes.
                let body = if c.body_kind == BodyKind::Stream {
                    Type::Builtin("Blob".into())
                } else {
                    c.ty.clone()
                };
                properties.push(Property::new("body", body));
                let (kind, media) = body_kind(c.body_kind, &c.media_type);
                kinds.push((r.status.key(), kind, media));
            }
            _ => kinds.push((r.status.key(), "none", String::new())),
        }
        members.push(Type::Object(Object {
            properties,
            index: None,
        }));
    }
    let union = if members.is_empty() {
        Type::Never
    } else {
        Type::union(members)
    };
    let doc = tsurara_sem::ir::Doc {
        description: Some(format!(
            "A response of `{}` that the definition documents.",
            op.name
        )),
        tags: Vec::new(),
    };
    unit.alias(&response_type, &union, Some(&doc));

    // The default: the first success with a sample (or without a body).
    let default = original
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
                None => Some(format!("{{ status: {status} }}")),
                Some(c) if c.body_kind == BodyKind::None => Some(format!("{{ status: {status} }}")),
                Some(c) => {
                    let key = r.status.key();
                    let sample = samples?
                        .responses
                        .iter()
                        .find(|s| s.status == key && s.media_type == c.media_type)?;
                    let body = match c.body_kind {
                        BodyKind::Json | BodyKind::Auto
                            if deep_cast(&c.ty, ctx.decls, &mut Default::default())
                                && !matches!(&c.ty, Type::Named(r) if ctx.sample_of.get(&r.id) == Some(&&sample.value)) =>
                        {
                            format!("({} as never)", literal(&sample.value, 1))
                        }
                        BodyKind::Json | BodyKind::Auto => ctx.expr(&c.ty, &sample.value, None, &mut unit, 1),
                        BodyKind::Text => match &sample.value {
                            Value::String(s) => string_literal(s),
                            v => string_literal(&v.to_string()),
                        },
                        _ => "new Blob()".to_owned(),
                    };
                    Some(format!("{{ status: {status}, body: {body} }}"))
                }
            }
        });

    let kinds_literal = format!(
        "{{ {} }}",
        kinds
            .iter()
            .map(|(status, kind, media)| format!(
                "{}: [{}, {}]",
                property_key(status),
                string_literal(kind),
                string_literal(media)
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let response_param = format!(
        "response: {response_type} | ((info: MockInfo) => {response_type} | Promise<{response_type}>){}",
        default
            .as_ref()
            .map(|d| format!(" = {d}"))
            .unwrap_or_default()
    );
    unit.refs.insert("MockInfo".into());
    unit.refs.insert("MockOptions".into());
    unit.value_refs.insert("respond".into());
    let summary = format!(
        "`{}`",
        tsurara_sem::print::doc_inline(&format!("{} {}", op.method, op.path))
    );
    let code = match mocks {
        Mocks::Msw => {
            let (matcher, note) = msw_path(&op.path);
            let method = op.method.to_lowercase();
            let (call, check) = if ["get", "post", "put", "delete", "patch", "head", "options"]
                .contains(&method.as_str())
            {
                (method.clone(), String::new())
            } else {
                (
                    "all".to_owned(),
                    format!(
                        "      if (info.request.method !== {}) return undefined;\n",
                        string_literal(&op.method)
                    ),
                )
            };
            format!(
                "/**\n * Answers {summary} with `response` (by default its first documented success with the sample\n * values).{note}\n */\nexport function {handler}(\n  {response_param},\n  options: MockOptions = {{}},\n): HttpHandler {{\n  return http.{call}(\n    {matcher},\n    async (info) => {{\n{check}      const r = typeof response === 'function' ? await response(info) : response;\n      return respond(r, {kinds_literal});\n    }},\n    {{ once: options.once ?? false }},\n  );\n}}\n"
            )
        }
        Mocks::Fetch => {
            let def = op
                .type_names
                .get("{}Def")
                .cloned()
                .unwrap_or_else(|| format!("{}Def", op.name));
            unit.value_refs.insert(def.clone());
            format!(
                "/** Answers {summary} with `response` (by default its first documented success with the sample values). */\nexport function {handler}(\n  {response_param},\n): MockRoute {{\n  return {{\n    operation: {def},\n    handler: async (info) => {{\n      const r = typeof response === 'function' ? await response(info) : response;\n      return respond(r, {kinds_literal});\n    }},\n  }};\n}}\n"
            )
        }
    };
    let _ = opts;
    unit.raw(&code);
    unit.declares.push(handler);
    unit.has_values = true;
    (unit, default.is_some())
}

/// The MSW matcher of a path template: `*/pets/:petId` (with `options.baseUrl` in place of `*`), or
/// a RegExp when a parameter is not a whole segment, which MSW cannot name.
fn msw_path(template: &str) -> (String, String) {
    let segments: Vec<&str> = template.split('/').collect();
    let whole = segments.iter().all(|s| {
        !s.contains('{') || (s.starts_with('{') && s.ends_with('}') && s.matches('{').count() == 1)
    });
    if whole {
        let path: Vec<String> = segments
            .iter()
            .map(
                |s| match s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                    Some(name) => format!(":{name}"),
                    None => (*s).to_owned(),
                },
            )
            .collect();
        let path = path.join("/");
        (
            format!(
                "`${{options.baseUrl ?? '*'}}{}`",
                crate::emit::escape_template(&path)
            ),
            String::new(),
        )
    } else {
        let mut source = String::new();
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            let Some(end) = rest[start..].find('}') else {
                break;
            };
            source.push_str(&regex_escape(&rest[..start]));
            source.push_str("[^/]+");
            rest = &rest[start + end + 1..];
        }
        source.push_str(&regex_escape(rest));
        (
            format!("new RegExp({})", string_literal(&format!("{source}$"))),
            "\n * A parameter is part of a segment, which MSW cannot name: the path is matched with a RegExp,\n * `info.params` is empty, and `options.baseUrl` is not used.".to_owned(),
        )
    }
}

fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if ".*+?^${}()|[]\\/".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `mocks/support.ts`.
fn support(mocks: Mocks, opts: &TypeScriptOptions) -> String {
    let common = "/** What a response function receives: the request and the path parameters. */\nexport type MockInfo = {\n  readonly request: Request;\n  readonly params: Readonly<Record<string, string | readonly string[] | undefined>>;\n};\n\n/** How a handler matches. */\nexport type MockOptions = {\n  /** The origin and base path to match (MSW); `*` (any) by default. */\n  readonly baseUrl?: string;\n  /** Answer only the first matching request (MSW). */\n  readonly once?: boolean;\n};\n\n/** How each status of an operation writes its body: the kind and the media type. */\nexport type MockBodies = Readonly<Record<string, readonly ['json' | 'text' | 'binary' | 'none', string]>>;\n\nconst kindOf = (\n  status: number,\n  bodies: MockBodies,\n): readonly ['json' | 'text' | 'binary' | 'none', string] =>\n  bodies[String(status)] ?? bodies[`${Math.floor(status / 100)}XX`] ?? bodies.default ?? ['json', 'application/json'];\n";
    match mocks {
        Mocks::Msw => format!(
            "{HEADER}\n\n{common}\n/** A documented response as a `Response` for MSW. */\nexport async function respond(\n  r: {{ readonly status: number; readonly body?: unknown }},\n  bodies: MockBodies,\n): Promise<Response> {{\n  const [kind, mediaType] = kindOf(r.status, bodies);\n  const headers: Record<string, string> = mediaType === '' ? {{}} : {{ 'content-type': mediaType }};\n  if (kind === 'none' || r.body === undefined) return new Response(null, {{ status: r.status }});\n  if (kind === 'json') return new Response(JSON.stringify(r.body), {{ status: r.status, headers }});\n  if (kind === 'text') return new Response(String(r.body), {{ status: r.status, headers }});\n  return new Response(r.body as Blob, {{ status: r.status, headers }});\n}}\n"
        ),
        Mocks::Fetch => format!(
            "{HEADER}\n\nimport type {{ MockResponse }} from '{}/testing';\n\n{common}\n/** A documented response for `mockFetch`. */\nexport async function respond(\n  r: {{ readonly status: number; readonly body?: unknown }},\n  bodies: MockBodies,\n): Promise<MockResponse> {{\n  const [kind, contentType] = kindOf(r.status, bodies);\n  if (kind === 'none' || r.body === undefined) return {{ status: r.status }};\n  if (kind === 'json') return {{ status: r.status, json: r.body }};\n  if (kind === 'text') return {{ status: r.status, text: String(r.body), contentType }};\n  return {{ status: r.status, bytes: new Uint8Array(await (r.body as Blob).arrayBuffer()), contentType }};\n}}\n",
            opts.client_module
        ),
    }
}

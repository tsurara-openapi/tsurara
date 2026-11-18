//! Prints the lowered module as a Python package.

use crate::HEADER;
use crate::naming::{PythonPolicy, allocate, snake_case};
use serde_json::Value;
use std::collections::BTreeSet;
use tsurara_backend::OutputFile;
use tsurara_backend::manifest::{Export, Manifest, TypeEntry, in_model_order};
use tsurara_lower::tir::*;
use tsurara_lower::typedef::type_defs;
use tsurara_naming::{Context, Names, NamingPolicy};
use tsurara_pretty::{Doc as Pretty, Fit, render};
use tsurara_sem::ir::{Literal, ScalarKind};
use tsurara_sem::naming::NameKind;
use tsurara_sem::{Model, Options, Warning, WarningKind};

pub(crate) struct Assigned {
    pub(crate) classes: Vec<String>,
    pub(crate) files: Vec<String>,
    members: Vec<Vec<String>>,
    additional: Vec<Option<String>>,
    enum_members: Vec<Vec<String>>,
}

pub fn print(
    model: &Model,
    module: &Module,
    opts: &Options,
    mocks: bool,
) -> (Vec<OutputFile>, Vec<Warning>, Manifest) {
    let names = assign(model, module, opts);
    let mut manifest = Manifest::new("python");
    manifest.types = in_model_order(
        model,
        module
            .decls
            .iter()
            .enumerate()
            .filter_map(|(i, d)| match d.origin {
                Origin::Schema(name) => Some(TypeEntry {
                    name,
                    export: Export::new(&names.classes[i], format!("{}.py", names.files[i])),
                }),
                Origin::Candidate => None,
            })
            .collect(),
    );
    let mut warnings = Vec::new();
    let mut files = Vec::new();
    for (i, d) in module.decls.iter().enumerate() {
        let mut p = Printer {
            module,
            names: &names,
            typing: BTreeSet::new(),
            collections: BTreeSet::new(),
            refs: BTreeSet::new(),
            externals: BTreeSet::new(),
            warnings: &mut warnings,
            missing: false,
            file: false,
            loc: d.loc.clone(),
        };
        let body = p.decl(DeclId(i), d);
        let code = p.module_code(DeclId(i), d, body);
        files.push(OutputFile {
            path: format!("{}.py", names.files[i]),
            contents: code,
        });
    }
    files.push(OutputFile {
        path: "__init__.py".into(),
        contents: init(&names),
    });
    files.push(OutputFile {
        path: "_support.py".into(),
        contents: format!(
            "{HEADER}\n\nfrom tsurara_client import MISSING, Missing\n\n__all__ = [\"MISSING\", \"Missing\"]\n"
        ),
    });
    let defs = type_defs(
        module,
        &names.classes,
        &|id: DeclId, wire: &str| member_of(module, &names, id, wire),
        "python",
    );
    files.push(OutputFile {
        path: "_typedefs.py".into(),
        contents: format!(
            "{HEADER}\n\"\"\"TypeDefs of the declarations, for the runtime that converts JSON (spec/type-def.schema.json).\"\"\"\n\nfrom typing import Any\n\nfrom tsurara_client import TypeDefs\n\nTYPE_DEFS: dict[str, Any] = {}\n\nCODEC = TypeDefs(TYPE_DEFS, module=__package__)\n\"\"\"The TypeDefs with the classes of this package.\"\"\"\n",
            python_value(&defs, 0)
        ),
    });
    let operations = crate::operations::print(
        model,
        module,
        &names,
        &mut warnings,
        &mut manifest.operations,
    );
    // Without operations there is no `operations` package: nothing needs httpx.
    files.extend(operations.into_iter().filter(|_| opts.operations));
    if mocks {
        files.extend(crate::mocks::files(
            model,
            module,
            &names,
            &manifest.operations,
            &mut warnings,
        ));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if opts.runtime == tsurara_sem::RuntimeSource::Vendor {
        crate::vendor::vendor(&mut files);
    }
    (files, warnings, manifest)
}

fn member_of(module: &Module, names: &Assigned, id: DeclId, wire: &str) -> String {
    if let DeclKind::Record(r) = &module.decls[id.0].kind {
        if let Some(i) = r.fields.iter().position(|f| f.wire_name == wire) {
            return names.members[id.0][i].clone();
        }
    }
    snake_case(wire)
}

/// Names declarations in model order (the model's name requests, then hoisted declarations), then
/// files, members and enum members per declaration.
pub(crate) fn assign(model: &Model, module: &Module, opts: &Options) -> Assigned {
    let mut names = Names::new();
    let mut classes = vec![String::new(); module.decls.len()];
    let claim = |names: &mut Names, raw: &str, preserve: bool, pointer: &str| {
        let ident = PythonPolicy.convert(Context::Type, raw, preserve);
        names
            .claim(&PythonPolicy, Context::Type, "types", &ident, pointer)
            .name
    };
    for (i, d) in module.decls.iter().enumerate() {
        let pointer = format!("#{}", d.loc.pointer);
        let preferred = opts.name_overrides.get(&pointer).cloned();
        if let Origin::Schema(id) = d.origin {
            let request = model.names.iter().find(|r| r.id == id);
            let (raw, preserve) = match request.map(|r| &r.kind) {
                Some(NameKind::Schema {
                    preferred: p,
                    preserve,
                }) => (
                    preferred.clone().unwrap_or(p.clone()),
                    *preserve || preferred.is_some(),
                ),
                Some(NameKind::Writable { .. }) => (format!("{}Writable", d.preferred_name), false),
                _ => (d.preferred_name.clone(), false),
            };
            classes[i] = claim(&mut names, &raw, preserve, &pointer);
        }
    }
    for (i, d) in module.decls.iter().enumerate() {
        if d.origin == Origin::Candidate {
            let pointer = format!("#{}", d.loc.pointer);
            let raw = opts
                .name_overrides
                .get(&pointer)
                .cloned()
                .unwrap_or(d.preferred_name.clone());
            classes[i] = claim(&mut names, &raw, false, &pointer);
        }
    }
    let file_raws: Vec<(&str, bool)> = classes.iter().map(|c| (c.as_str(), false)).collect();
    let mut files = allocate(Context::File, &file_raws);
    for f in &mut files {
        // Modules of the package itself.
        if f.starts_with('_') {
            f.insert(0, 'm');
        }
    }
    let mut members = Vec::new();
    let mut additional = Vec::new();
    let mut enum_members = Vec::new();
    for d in &module.decls {
        match &d.kind {
            DeclKind::Record(r) => {
                let mut raws: Vec<(&str, bool)> = r
                    .fields
                    .iter()
                    .map(|f| (f.wire_name.as_str(), false))
                    .collect();
                if r.additional.is_some() {
                    raws.push(("additional_properties", false));
                }
                let mut all = allocate(Context::Member, &raws);
                additional.push(r.additional.as_ref().map(|_| all.pop().expect("claimed")));
                members.push(all);
                enum_members.push(Vec::new());
            }
            DeclKind::Enum(e) => {
                let texts: Vec<String> = e.values.iter().map(literal_name).collect();
                let raws: Vec<(&str, bool)> = texts.iter().map(|t| (t.as_str(), false)).collect();
                enum_members.push(allocate(Context::Constant, &raws));
                members.push(Vec::new());
                additional.push(None);
            }
            _ => {
                members.push(Vec::new());
                additional.push(None);
                enum_members.push(Vec::new());
            }
        }
    }
    Assigned {
        classes,
        files,
        members,
        additional,
        enum_members,
    }
}

fn literal_name(l: &Literal) -> String {
    match l {
        Literal::String(s) => s.clone(),
        Literal::Number(n) => n.to_string().replace(['.', '-'], "_"),
        Literal::Boolean(b) => b.to_string(),
    }
}

fn literal_code(l: &Literal) -> String {
    match l {
        Literal::String(s) => serde_json::to_string(s).expect("strings serialize"),
        Literal::Number(n) => n.to_string(),
        Literal::Boolean(true) => "True".into(),
        Literal::Boolean(false) => "False".into(),
    }
}

pub(crate) struct Printer<'a> {
    pub(crate) module: &'a Module,
    pub(crate) names: &'a Assigned,
    pub(crate) typing: BTreeSet<&'static str>,
    /// Names imported from `collections.abc` (`Iterator`, `AsyncIterator`).
    pub(crate) collections: BTreeSet<String>,
    /// Declarations referenced in annotations; imported for type checkers only.
    pub(crate) refs: BTreeSet<usize>,
    pub(crate) externals: BTreeSet<(String, String)>,
    pub(crate) warnings: &'a mut Vec<Warning>,
    pub(crate) missing: bool,
    /// Whether an annotation uses `File` (binary values that are sent).
    pub(crate) file: bool,
    /// The declaration being printed, for warnings.
    pub(crate) loc: tsurara_loader::Loc,
}

fn lines(ls: impl IntoIterator<Item = String>) -> Pretty {
    Pretty::concat(
        ls.into_iter()
            .flat_map(|l| [Pretty::text(l), Pretty::HardLine]),
    )
}

pub(crate) fn docstring(doc: Option<&tsurara_sem::ir::Doc>) -> Option<String> {
    let d = doc?.description.as_deref()?.trim();
    if d.is_empty() {
        return None;
    }
    let d = tsurara_sem::text::lines(d).join("\n");
    let mut body = d.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"");
    // A quote at the end would merge with the closing delimiter, unless it is already escaped (an odd
    // number of backslashes before it).
    let escaped = body
        .strip_suffix('"')
        .is_some_and(|rest| rest.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1);
    if body.ends_with('"') && !escaped {
        body.pop();
        body.push_str("\\\"");
    }
    Some(format!("\"\"\"{body}\"\"\""))
}

impl Printer<'_> {
    fn decl(&mut self, id: DeclId, d: &Decl) -> Pretty {
        let name = self.names.classes[id.0].clone();
        match &d.kind {
            DeclKind::Record(r) => self.record(id, &name, d, r),
            DeclKind::Enum(e) => {
                let mut body = Vec::new();
                if let Some(doc) = docstring(d.doc.as_ref()) {
                    body.push(Pretty::text(doc));
                    body.push(Pretty::HardLine);
                    body.push(Pretty::HardLine);
                }
                for (v, m) in e.values.iter().zip(&self.names.enum_members[id.0]) {
                    body.push(Pretty::text(format!("{m} = {}", literal_code(v))));
                    body.push(Pretty::HardLine);
                }
                if e.values.is_empty() {
                    body.push(Pretty::text("pass"));
                    body.push(Pretty::HardLine);
                }
                Pretty::concat([
                    Pretty::text(format!("class {name}(Enum):")),
                    Pretty::nest(4, Pretty::concat([Pretty::HardLine, Pretty::concat(body)])),
                ])
            }
            DeclKind::TaggedUnion(t) => {
                let mut members: Vec<String> = t.variants.iter().map(|v| self.ann(&v.ty)).collect();
                if let Some(d) = &t.default {
                    members.push(self.ann(d));
                }
                self.alias(&name, d, &union(members))
            }
            DeclKind::UntaggedUnion(u) => {
                let members = u.members.iter().map(|m| self.ann(m)).collect();
                self.alias(&name, d, &union(members))
            }
            DeclKind::Alias(e) => {
                let a = self.ann(e);
                self.alias(&name, d, &a)
            }
        }
    }

    fn alias(&mut self, name: &str, d: &Decl, ann: &str) -> Pretty {
        self.typing.insert("TypeAlias");
        let mut out = Vec::new();
        if let Some(doc) = d.doc.as_ref().and_then(|d| d.description.as_deref()) {
            out.extend(
                tsurara_sem::text::lines(doc.trim())
                    .iter()
                    .map(|l| format!("# {l}").trim_end().to_owned()),
            );
        }
        // A string, so that names imported for type checkers only are not evaluated.
        out.push(format!(
            "{name}: TypeAlias = {}",
            serde_json::to_string(ann).expect("strings serialize")
        ));
        lines(out)
    }

    fn record(&mut self, id: DeclId, name: &str, d: &Decl, r: &Record) -> Pretty {
        let base = r.base.map(|b| self.names.classes[b.0].clone());
        let mut body = Vec::new();
        if let Some(doc) = docstring(d.doc.as_ref()) {
            body.push(doc);
            body.push(String::new());
        }
        for (f, member) in r.fields.iter().zip(&self.names.members[id.0]) {
            if let Some(doc) = f.doc.as_ref().and_then(|d| d.description.as_deref()) {
                body.extend(
                    tsurara_sem::text::lines(doc.trim())
                        .iter()
                        .map(|l| format!("# {l}").trim_end().to_owned()),
                );
            }
            let mut parts = vec![self.ann(&f.ty)];
            let (extra, default) = match f.presence {
                Presence::Required => (vec![], None),
                Presence::Optional => (vec!["None".to_owned()], Some("None")),
                Presence::Nullable => (vec!["None".to_owned()], None),
                Presence::OptionalNullable => {
                    self.missing = true;
                    (
                        vec!["None".to_owned(), "Missing".to_owned()],
                        Some("MISSING"),
                    )
                }
            };
            parts.extend(extra);
            let ann = union(parts);
            if f.wire_name != *member {
                body.push(format!(
                    "# JSON: {}",
                    serde_json::to_string(&f.wire_name).expect("strings serialize")
                ));
            }
            match default {
                Some(v) => body.push(format!("{member}: {ann} = {v}")),
                None => body.push(format!("{member}: {ann}")),
            }
        }
        if let (Some(a), Some(member)) = (&r.additional, &self.names.additional[id.0]) {
            let v = self.ann(a);
            body.push(format!(
                "{member}: dict[str, {v}] = field(default_factory=dict)"
            ));
        }
        if body.iter().all(|l| l.is_empty() || l.starts_with("\"\"\"")) {
            body.push("pass".to_owned());
        }
        let head = match &base {
            Some(b) => format!("class {name}({b}):"),
            None => format!("class {name}:"),
        };
        Pretty::concat([
            lines(["@dataclass(kw_only=True)".to_owned()]),
            Pretty::text(head),
            Pretty::nest(4, Pretty::concat([Pretty::HardLine, lines(body)])),
        ])
    }

    /// The annotation of an expression, as text.
    pub(crate) fn ann(&mut self, e: &Expr) -> String {
        match e {
            Expr::Any | Expr::InlineRecord(_) | Expr::Intersection(_) => {
                self.typing.insert("Any");
                "Any".into()
            }
            Expr::Never => {
                self.typing.insert("Never");
                "Never".into()
            }
            Expr::Null => "None".into(),
            Expr::Bool => "bool".into(),
            // Binary values are only sent (multipart parts); `File` adds a name and a media type.
            Expr::Bytes => {
                self.file = true;
                "bytes | File".into()
            }
            Expr::Scalar(s) => match s.kind {
                ScalarKind::Integer => "int",
                ScalarKind::Number => "float",
                ScalarKind::String => "str",
            }
            .into(),
            Expr::List(t) => format!("list[{}]", self.ann(t)),
            Expr::Map(t) => format!("dict[str, {}]", self.ann(t)),
            Expr::Tuple {
                items,
                required,
                rest,
            } => {
                // A tuple per allowed length; Python has no optional tuple elements.
                let items: Vec<String> = items.iter().map(|i| self.ann(i)).collect();
                let rest = rest.as_deref().map(|r| self.ann(r));
                let mut variants = Vec::new();
                for n in *required..=items.len() {
                    let mut parts: Vec<String> = items[..n].to_vec();
                    if n == items.len() {
                        if let Some(r) = &rest {
                            parts.push(format!("*tuple[{r}, ...]"));
                        }
                    }
                    variants.push(if parts.is_empty() {
                        "tuple[()]".to_owned()
                    } else {
                        format!("tuple[{}]", parts.join(", "))
                    });
                }
                union(variants)
            }
            Expr::Named { decl, site } => {
                self.refs.insert(decl.0);
                let name = self.names.classes[decl.0].clone();
                match &self.module.decls[decl.0].kind {
                    // An open enum where values are received keeps unlisted values as they are.
                    DeclKind::Enum(en)
                        if en.open && matches!(site, Site::Received | Site::Both) =>
                    {
                        let raw = if en.values.iter().all(|v| matches!(v, Literal::Number(_))) {
                            "int"
                        } else {
                            "str"
                        };
                        union(vec![name, raw.to_owned()])
                    }
                    // So does an open tagged union, as the JSON object.
                    DeclKind::TaggedUnion(t)
                        if t.open && matches!(site, Site::Received | Site::Both) =>
                    {
                        self.typing.insert("Any");
                        union(vec![name, "dict[str, Any]".to_owned()])
                    }
                    _ => name,
                }
            }
            Expr::External { entries, schema } => match entries.get("python") {
                Some(ext) => match crate::external::check(ext) {
                    Ok(()) => {
                        if let Some(i) = &ext.import {
                            self.externals.insert((i.from.clone(), i.name.clone()));
                        }
                        ext.code.clone()
                    }
                    Err(why) => {
                        let fallback = self.ann(schema);
                        self.warnings.push(Warning {
                            kind: WarningKind::InvalidDefinition,
                            loc: self.loc.clone(),
                            message: format!("{why}; the schema's type is used"),
                            language: None,
                        });
                        fallback
                    }
                },
                None => {
                    let fallback = self.ann(schema);
                    self.warnings.push(Warning {
                        kind: WarningKind::UnmappedType,
                        loc: self.loc.clone(),
                        message: "no `python` entry; the schema's type is used".into(),
                        language: None,
                    });
                    fallback
                }
            },
            Expr::Nullable(t) => union(vec![self.ann(t), "None".into()]),
            Expr::Union(ts) => union(ts.iter().map(|t| self.ann(t)).collect()),
            Expr::Literal(l) => {
                self.typing.insert("Literal");
                format!("Literal[{}]", literal_code(l))
            }
        }
    }

    fn module_code(&mut self, id: DeclId, d: &Decl, body: Pretty) -> String {
        let mut head = vec![
            HEADER.to_owned(),
            String::new(),
            "from __future__ import annotations".to_owned(),
            String::new(),
        ];
        let base = match &d.kind {
            DeclKind::Record(r) => r.base,
            _ => None,
        };
        let mut stdlib = Vec::new();
        match &d.kind {
            DeclKind::Record(r) => {
                if r.additional.is_some() {
                    stdlib.push("from dataclasses import dataclass, field".to_owned());
                } else {
                    stdlib.push("from dataclasses import dataclass".to_owned());
                }
            }
            DeclKind::Enum(_) => stdlib.push("from enum import Enum".to_owned()),
            _ => {}
        }
        let mut typing: Vec<&str> = self.typing.iter().copied().collect();
        let checking: Vec<usize> = self
            .refs
            .iter()
            .copied()
            .filter(|r| *r != id.0 && Some(DeclId(*r)) != base)
            .collect();
        if !checking.is_empty() || !self.externals.is_empty() {
            typing.push("TYPE_CHECKING");
        }
        typing.sort_unstable();
        if !typing.is_empty() {
            stdlib.push(format!("from typing import {}", typing.join(", ")));
        }
        head.extend(stdlib);
        let mut local = Vec::new();
        if self.missing {
            local.push("from ._support import MISSING, Missing".to_owned());
        }
        if self.file {
            local.push("from tsurara_client import File".to_owned());
        }
        if let Some(b) = base {
            local.push(format!(
                "from .{} import {}",
                self.names.files[b.0], self.names.classes[b.0]
            ));
        }
        if !local.is_empty() {
            head.push(String::new());
            head.extend(local);
        }
        if !checking.is_empty() || !self.externals.is_empty() {
            head.push(String::new());
            head.push("if TYPE_CHECKING:".to_owned());
            // External types too: annotations are strings, so their modules need not run.
            for (from, name) in &self.externals {
                head.push(format!("    from {from} import {name}"));
            }
            for r in checking {
                head.push(format!(
                    "    from .{} import {}",
                    self.names.files[r], self.names.classes[r]
                ));
            }
        }
        let doc = Pretty::concat([lines(head), Pretty::HardLine, Pretty::HardLine, body]);
        let mut out = render(&Pretty::group(Fit::Never, doc), 100);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }
}

/// `A | B`, without repeated members.
pub(crate) fn union(parts: Vec<String>) -> String {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for p in parts {
        for piece in top_level_members(&p) {
            if seen.insert(piece.to_owned()) {
                out.push(piece.to_owned());
            }
        }
    }
    out.join(" | ")
}

/// The members of a union annotation, split at the ` | ` outside brackets (`list[str | None]` is one).
fn top_level_members(annotation: &str) -> Vec<&str> {
    let mut members = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let bytes = annotation.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b' ' if depth == 0 && annotation[i..].starts_with(" | ") => {
                members.push(&annotation[start..i]);
                i += 3;
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    members.push(&annotation[start..]);
    members
}

fn init(names: &Assigned) -> String {
    let mut pairs: Vec<(&String, &String)> = names.files.iter().zip(&names.classes).collect();
    pairs.sort();
    let mut s = format!("{HEADER}\n\n");
    for (file, class) in &pairs {
        s.push_str(&format!("from .{file} import {class}\n"));
    }
    s.push_str("\n__all__ = [\n");
    let mut classes: Vec<&String> = names.classes.iter().collect();
    classes.sort();
    for c in classes {
        s.push_str(&format!(
            "    {},\n",
            serde_json::to_string(c).expect("strings serialize")
        ));
    }
    s.push_str("]\n");
    s
}

/// A JSON value as a Python literal.
pub(crate) fn python_value(v: &Value, indent: usize) -> String {
    let pad = "    ".repeat(indent + 1);
    let end = "    ".repeat(indent);
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).expect("strings serialize"),
        Value::Array(a) if a.is_empty() => "[]".into(),
        Value::Array(a) => {
            let items: Vec<String> = a
                .iter()
                .map(|x| format!("{pad}{},", python_value(x, indent + 1)))
                .collect();
            format!("[\n{}\n{end}]", items.join("\n"))
        }
        Value::Object(o) if o.is_empty() => "{}".into(),
        Value::Object(o) => {
            let items: Vec<String> = o
                .iter()
                .map(|(k, x)| {
                    format!(
                        "{pad}{}: {},",
                        serde_json::to_string(k).expect("strings serialize"),
                        python_value(x, indent + 1)
                    )
                })
                .collect();
            format!("{{\n{}\n{end}}}", items.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{docstring, union};
    use tsurara_sem::ir::Doc;

    fn doc(text: &str) -> Option<String> {
        docstring(Some(&Doc {
            description: Some(text.to_owned()),
            tags: Vec::new(),
        }))
    }

    #[test]
    fn unions_merge_only_top_level_members() {
        assert_eq!(
            union(vec!["list[str | S0]".into(), "list[str | None]".into()]),
            "list[str | S0] | list[str | None]"
        );
        assert_eq!(
            union(vec!["int | None".into(), "None".into()]),
            "int | None"
        );
        assert_eq!(
            union(vec!["dict[str, int | str] | str".into(), "str".into()]),
            "dict[str, int | str] | str"
        );
    }

    #[test]
    fn docstrings_escape_quotes_and_backslashes() {
        assert_eq!(doc("plain").unwrap(), r#""""plain""""#);
        // A closing quote would merge with the delimiter.
        assert_eq!(doc(r#"see "x""#).unwrap(), r#""""see "x\"""""#);
        assert_eq!(doc(r#"a """ b"#).unwrap(), r#""""a \"\"\" b""""#);
        assert_eq!(doc(r"ends \").unwrap(), r#""""ends \\""""#);
        assert_eq!(doc(r#"ends """"#).unwrap(), r#""""ends \"\"\"""""#);
        assert_eq!(doc(r#"ends \""#).unwrap(), r#""""ends \\\"""""#);
    }
}

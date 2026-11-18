//! Prints the lowered module as a Rust module tree (docs/languages/rust.md).

use crate::naming::{RustPolicy, allocate, allocate_variants, ident};
use crate::{HEADER, RustOptions};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_backend::manifest::{Export, Manifest, TypeEntry, in_model_order};
use tsurara_lower::tir::*;
use tsurara_naming::{Context, Names, NamingPolicy};
use tsurara_sem::ir::{Literal, ScalarKind};
use tsurara_sem::naming::NameKind;
use tsurara_sem::{Model, Options, Warning, WarningKind};

pub(crate) struct Assigned {
    pub(crate) types: Vec<String>,
    pub(crate) files: Vec<String>,
}

/// Names declarations in model order (the model's name requests, then hoisted declarations). The
/// names stay claimed in the returned scope, for the types of the operations.
fn assign(model: &Model, module: &Module, opts: &Options) -> (Assigned, Names) {
    let mut names = Names::new();
    let mut types = vec![String::new(); module.decls.len()];
    let claim = |names: &mut Names, raw: &str, pointer: &str| {
        let id = RustPolicy.convert(Context::Type, raw, false);
        names
            .claim(&RustPolicy, Context::Type, "types", &id, pointer)
            .name
    };
    for (i, d) in module.decls.iter().enumerate() {
        if let Origin::Schema(id) = d.origin {
            let pointer = format!("#{}", d.loc.pointer);
            let raw = match (
                opts.name_overrides.get(&pointer),
                model.names.iter().find(|r| r.id == id).map(|r| &r.kind),
            ) {
                (Some(o), _) => o.clone(),
                (None, Some(NameKind::Schema { preferred, .. })) => preferred.clone(),
                (None, Some(NameKind::Writable { .. })) => format!("{}Writable", d.preferred_name),
                _ => d.preferred_name.clone(),
            };
            types[i] = claim(&mut names, &raw, &pointer);
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
            types[i] = claim(&mut names, &raw, &pointer);
        }
    }
    let raws: Vec<&str> = types.iter().map(String::as_str).collect();
    let mut files = allocate(Context::File, &raws);
    for f in &mut files {
        // `mod.rs` belongs to the module itself.
        if f == "mod" {
            f.push('_');
        }
    }
    (Assigned { types, files }, names)
}

pub(crate) struct Printer<'a> {
    pub(crate) names: &'a Assigned,
    /// Aliases that contain themselves, printed as newtypes (`true`: boxed, because the cycle
    /// passes through no list or map).
    pub(crate) newtypes: &'a HashMap<usize, bool>,
    pub(crate) rt: &'a str,
    pub(crate) warnings: &'a mut Vec<Warning>,
    pub(crate) loc: tsurara_loader::Loc,
    /// The generated types the printed code refers to, for explicit imports: a glob import of the
    /// parent module in every type module makes name resolution quadratic in the number of types
    ///.
    pub(crate) used: std::collections::BTreeSet<String>,
}

pub fn print(
    model: &Model,
    module: &Module,
    opts: &Options,
    rust: &RustOptions,
) -> (Vec<OutputFile>, Vec<Warning>, Manifest) {
    let (names, mut scope) = assign(model, module, opts);
    let mut functions = Names::new();
    let mut operations = Vec::new();
    let mut warnings = Vec::new();
    let mut files = Vec::new();
    let rt = rust.runtime_crate.as_str();
    let newtypes = recursive_aliases(module);
    for (i, d) in module.decls.iter().enumerate() {
        let mut p = Printer {
            names: &names,
            newtypes: &newtypes,
            rt,
            warnings: &mut warnings,
            loc: d.loc.clone(),
            used: Default::default(),
        };
        let body = p.decl(DeclId(i), d);
        p.used.remove(&names.types[i]);
        let imports = match p.used.len() {
            0 => String::new(),
            1 => format!("use super::{};\n", p.used.iter().next().expect("one name")),
            _ => format!(
                "use super::{{{}}};\n",
                p.used.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
        };
        let code = format!(
            "{HEADER}\n\n{imports}#[allow(unused_imports)]\nuse {rt}::serde::{{Deserialize, Serialize}};\n\n{body}"
        );
        files.push(OutputFile {
            path: format!("types/{}.rs", names.files[i]),
            contents: code,
        });
    }
    let mut types_mod = format!(
        "{HEADER}\n\n// Generated code follows the definition, not every style lint.\n#![allow(clippy::large_enum_variant, clippy::enum_variant_names, clippy::doc_lazy_continuation, clippy::doc_overindented_list_items, clippy::empty_docs)]\n\n"
    );
    let mut order: Vec<usize> = (0..module.decls.len()).collect();
    order.sort_by(|a, b| names.files[*a].cmp(&names.files[*b]));
    for i in &order {
        writeln!(types_mod, "mod {};", names.files[*i]).unwrap();
    }
    types_mod.push('\n');
    for i in &order {
        writeln!(types_mod, "pub use {}::*;", names.files[*i]).unwrap();
    }
    files.push(OutputFile {
        path: "types/mod.rs".into(),
        contents: types_mod,
    });
    for (op, types) in model.operations.iter().zip(&module.operations) {
        let mut p = Printer {
            names: &names,
            newtypes: &newtypes,
            rt,
            warnings: &mut warnings,
            loc: op.loc.clone(),
            used: Default::default(),
        };
        operations.push(crate::operations::operation(
            &mut p,
            op,
            types,
            &mut scope,
            &mut functions,
        ));
    }
    let mut manifest = Manifest::new("rust");
    manifest.types = in_model_order(
        model,
        module
            .decls
            .iter()
            .enumerate()
            .filter_map(|(i, d)| match d.origin {
                Origin::Schema(name) => Some(TypeEntry {
                    name,
                    export: Export::new(&names.types[i], format!("types/{}.rs", names.files[i])),
                }),
                Origin::Candidate => None,
            })
            .collect(),
    );
    manifest.operations = operations.iter().map(|m| m.entry.clone()).collect();
    // Without operations there is no `operations` module: the types need only the
    // runtime without its `client` feature.
    if opts.operations {
        files.extend(crate::operations::files(operations));
    }
    if rust.mocks {
        files.extend(crate::mocks::files(
            model,
            module,
            &names,
            &newtypes,
            &manifest.operations,
            rt,
            &mut warnings,
        ));
    }
    let vendored = opts.runtime == tsurara_sem::RuntimeSource::Vendor;
    // A vendored runtime is its own version; its dependencies go in the user's crate.
    let requires = if vendored {
        let mut s =
            String::from("// The runtime is vendored in `runtime`; it needs these dependencies:\n");
        for d in crate::vendor::dependencies(opts.operations) {
            s.push_str(&format!("//     {d}\n"));
        }
        s
    } else if opts.operations {
        format!(
            "// Requires tsurara-client {}\n",
            crate::RUNTIME_REQUIREMENT
        )
    } else {
        format!(
            "// Requires tsurara-client {}\n// Types only: the runtime without its `client` feature (`default-features = false`)\n",
            crate::RUNTIME_REQUIREMENT
        )
    };
    files.push(OutputFile {
        path: "mod.rs".into(),
        contents: format!(
            "{HEADER}\n{requires}\n{}{}{}pub mod types;\n",
            if rust.mocks {
                "// Mocks need the runtime's `testing` feature: a `mocks` feature of this crate enables it.\n#[cfg(any(test, feature = \"mocks\"))]\npub mod mocks;\n"
            } else {
                ""
            },
            if opts.operations {
                "pub mod operations;\n"
            } else {
                ""
            },
            if vendored { "pub mod runtime;\n" } else { "" }
        ),
    });
    if vendored {
        files.extend(crate::vendor::files(
            &rust.runtime_crate,
            opts.operations,
            rust.mocks,
        ));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    (files, warnings, manifest)
}

/// The declarations an expression names; with `direct`, only those outside lists and maps.
fn named_decls(e: &Expr, direct: bool, out: &mut Vec<DeclId>) {
    match e {
        Expr::Named { decl, .. } => out.push(*decl),
        Expr::List(t) | Expr::Map(t) => {
            if !direct {
                named_decls(t, direct, out);
            }
        }
        Expr::Tuple { items, rest, .. } => {
            items.iter().for_each(|t| named_decls(t, direct, out));
            if let Some(r) = rest {
                named_decls(r, direct, out);
            }
        }
        Expr::External { schema, .. } => named_decls(schema, direct, out),
        Expr::Nullable(t) => named_decls(t, direct, out),
        Expr::InlineRecord(r) => r
            .fields
            .iter()
            .for_each(|f| named_decls(&f.ty, direct, out)),
        Expr::Union(ts) | Expr::Intersection(ts) => {
            ts.iter().for_each(|t| named_decls(t, direct, out))
        }
        _ => {}
    }
}

/// Aliases that reach themselves through aliases alone, with whether the cycle stays outside lists
/// and maps (and so needs a `Box`).
fn recursive_aliases(module: &Module) -> HashMap<usize, bool> {
    let reaches = |start: usize, direct: bool| {
        let mut seen = HashSet::new();
        let mut stack = Vec::new();
        if let DeclKind::Alias(e) = &module.decls[start].kind {
            named_decls(e, direct, &mut stack);
        }
        while let Some(id) = stack.pop() {
            if id.0 == start {
                return true;
            }
            if seen.insert(id.0) {
                if let DeclKind::Alias(e) = &module.decls[id.0].kind {
                    named_decls(e, direct, &mut stack);
                }
            }
        }
        false
    };
    (0..module.decls.len())
        .filter(|&i| matches!(module.decls[i].kind, DeclKind::Alias(_)) && reaches(i, false))
        .map(|i| (i, reaches(i, true)))
        .collect()
}

/// Doc comments: `///` lines, with tabs as spaces, and opening code fences without a language (or
/// marked as Rust) marked as text so that rustdoc does not run them.
pub(crate) fn docs(doc: Option<&tsurara_sem::ir::Doc>, indent: &str) -> String {
    let Some(text) = doc.and_then(|d| d.description.as_deref()) else {
        return String::new();
    };
    let mut out = String::new();
    let mut fenced = false;
    for line in tsurara_sem::text::lines(text.trim()) {
        let line = line.trim_end().replace('\t', "    ");
        let line = match line.trim_start().strip_prefix("```") {
            Some(_) if fenced => {
                fenced = false;
                line
            }
            Some(rest) => {
                fenced = true;
                let lang = rest.trim();
                if lang.is_empty() || lang.starts_with("rust") {
                    line.replacen(&format!("```{rest}"), "```text", 1)
                } else {
                    line
                }
            }
            None => line,
        };
        if line.is_empty() {
            writeln!(out, "{indent}///").unwrap();
        } else {
            writeln!(out, "{indent}/// {line}").unwrap();
        }
    }
    out
}

fn literal_text(l: &Literal) -> String {
    match l {
        Literal::String(s) => s.clone(),
        Literal::Number(n) => n.to_string(),
        Literal::Boolean(b) => b.to_string(),
    }
}

impl Printer<'_> {
    fn value(&self) -> String {
        format!("{}::serde_json::Value", self.rt)
    }

    fn derive(&self, extra: &str) -> String {
        format!(
            "#[derive(Debug, Clone, PartialEq{extra}, Serialize, Deserialize)]\n#[serde(crate = \"{}::serde\")]\n",
            self.rt
        )
    }

    fn decl(&mut self, id: DeclId, d: &Decl) -> String {
        let name = self.names.types[id.0].clone();
        let mut out = docs(d.doc.as_ref(), "");
        match &d.kind {
            DeclKind::Record(r) => out.push_str(&self.record(&name, r)),
            DeclKind::Enum(e) => out.push_str(&self.enumeration(&name, e)),
            DeclKind::TaggedUnion(t) => out.push_str(&self.tagged(&name, t)),
            DeclKind::UntaggedUnion(u) => out.push_str(&self.untagged(&name, &u.members)),
            DeclKind::Alias(e) => {
                let ty = self.ty(e);
                match self.newtypes.get(&id.0) {
                    // Rust aliases cannot contain themselves.
                    Some(&boxed) => {
                        let ty = if boxed { format!("Box<{ty}>") } else { ty };
                        out.push_str(&self.derive(""));
                        writeln!(out, "#[serde(transparent)]\npub struct {name}(pub {ty});")
                            .unwrap();
                    }
                    None => writeln!(out, "pub type {name} = {ty};").unwrap(),
                }
            }
        }
        out
    }

    fn record(&mut self, name: &str, r: &Record) -> String {
        let mut raws: Vec<&str> = r.fields.iter().map(|f| f.wire_name.as_str()).collect();
        if r.additional.is_some() {
            raws.push("additional_properties");
        }
        let members = allocate(Context::Member, &raws);
        let mut out = self.derive("");
        writeln!(out, "pub struct {name} {{").unwrap();
        for (f, member) in r.fields.iter().zip(&members) {
            out.push_str(&docs(f.doc.as_ref(), "    "));
            let mut inner = self.ty(&f.ty);
            // A nullable field type is `Option<T>` already; presence decides the rest.
            let nullable_ty = matches!(f.ty, Expr::Nullable(_));
            if let (true, Expr::Nullable(t)) = (f.boxed, &f.ty) {
                inner = format!("Option<Box<{}>>", self.ty(t));
            } else if f.boxed {
                inner = format!("Box<{inner}>");
            }
            let mut attrs = Vec::new();
            if f.wire_name != *member {
                attrs.push(format!("rename = {:?}", f.wire_name));
            }
            let ty = match f.presence {
                Presence::Required => inner,
                Presence::Nullable => {
                    if nullable_ty {
                        inner
                    } else {
                        format!("Option<{inner}>")
                    }
                }
                Presence::Optional => {
                    attrs.push("default".into());
                    attrs.push("skip_serializing_if = \"Option::is_none\"".into());
                    if nullable_ty {
                        inner
                    } else {
                        format!("Option<{inner}>")
                    }
                }
                Presence::OptionalNullable => {
                    attrs.push("default".into());
                    attrs.push(format!(
                        "skip_serializing_if = \"{}::Nullable::is_missing\"",
                        self.rt
                    ));
                    let base = match (&f.ty, f.boxed) {
                        (Expr::Nullable(t), true) => format!("Box<{}>", self.ty(t)),
                        (Expr::Nullable(t), false) => self.ty(t),
                        _ => inner,
                    };
                    format!("{}::Nullable<{base}>", self.rt)
                }
            };
            if !attrs.is_empty() {
                writeln!(out, "    #[serde({})]", attrs.join(", ")).unwrap();
            }
            writeln!(out, "    pub {}: {ty},", ident(member)).unwrap();
        }
        if let (Some(a), Some(member)) = (&r.additional, members.last()) {
            let ty = self.ty(a);
            writeln!(
                out,
                "    #[serde(flatten)]\n    pub {}: std::collections::BTreeMap<String, {ty}>,",
                ident(member)
            )
            .unwrap();
        }
        out.push_str("}\n");
        out
    }

    fn enumeration(&mut self, name: &str, e: &EnumDecl) -> String {
        let strings = e.values.iter().all(|v| matches!(v, Literal::String(_)));
        if !strings || e.repr == EnumRepr::Constants {
            // Integer, boolean and mixed enums: their scalar type with named constants.
            let ty = if e
                .values
                .iter()
                .all(|v| matches!(v, Literal::Number(n) if n.is_i64() || n.is_u64()))
            {
                "i64".to_owned()
            } else if e.values.iter().all(|v| matches!(v, Literal::Number(_))) {
                "f64".to_owned()
            } else if strings {
                "String".to_owned()
            } else {
                self.value()
            };
            let mut out = format!("pub type {name} = {ty};\n");
            if ty == "i64" || ty == "f64" {
                let texts: Vec<String> = e.values.iter().map(literal_text).collect();
                let prefixed: Vec<String> = texts.iter().map(|t| format!("{name}_{t}")).collect();
                let raws: Vec<&str> = prefixed.iter().map(String::as_str).collect();
                for (v, c) in texts.iter().zip(allocate(Context::Constant, &raws)) {
                    let v = if ty == "f64" && !v.contains('.') {
                        format!("{v}.0")
                    } else {
                        v.clone()
                    };
                    writeln!(out, "pub const {c}: {name} = {v};").unwrap();
                }
            }
            return out;
        }
        let texts: Vec<String> = e.values.iter().map(literal_text).collect();
        let mut raws: Vec<&str> = texts.iter().map(String::as_str).collect();
        if e.open {
            raws.push("Unknown");
        }
        let variants = allocate_variants(&raws);
        let mut out = self.derive(if e.open {
            ", Eq, Hash"
        } else {
            ", Copy, Eq, Hash"
        });
        writeln!(out, "pub enum {name} {{").unwrap();
        for (t, v) in texts.iter().zip(&variants) {
            writeln!(out, "    #[serde(rename = {t:?})]\n    {v},").unwrap();
        }
        if e.open {
            writeln!(out, "    /// A value the definition does not list.\n    #[serde(untagged)]\n    {}(String),", variants.last().expect("unknown")).unwrap();
        }
        out.push_str("}\n");
        out
    }

    /// Variant names for union members: the type's name, or its kind.
    fn variant_names(&mut self, members: &[&Expr]) -> Vec<String> {
        let raws: Vec<String> = members
            .iter()
            .map(|m| match m {
                Expr::Named { decl, .. } => self.names.types[decl.0].clone(),
                Expr::Scalar(s) => match s.kind {
                    ScalarKind::Integer => "Integer".into(),
                    ScalarKind::Number => "Number".into(),
                    ScalarKind::String => "String".into(),
                },
                Expr::Bool => "Bool".into(),
                Expr::Null => "Null".into(),
                Expr::List(_) | Expr::Tuple { .. } => "List".into(),
                Expr::Map(_) => "Map".into(),
                _ => "Value".into(),
            })
            .collect();
        let refs: Vec<&str> = raws.iter().map(String::as_str).collect();
        allocate_variants(&refs)
    }

    fn tagged(&mut self, name: &str, t: &TaggedUnion) -> String {
        let mut members: Vec<&Expr> = t.variants.iter().map(|v| &v.ty).collect();
        if let Some(d) = &t.default {
            members.push(d);
        }
        let mut variants = self.variant_names(&members);
        // Unlisted tags decode as the default member when there is one, so no `Unknown` is needed.
        let unknown = if t.open && t.default.is_none() {
            let raws: Vec<&str> = variants
                .iter()
                .map(String::as_str)
                .chain(["Unknown"])
                .collect();
            variants = allocate_variants(&raws);
            variants.pop()
        } else {
            None
        };
        let mut out = format!("#[derive(Debug, Clone, PartialEq)]\npub enum {name} {{\n");
        for (m, v) in members.iter().zip(&variants) {
            writeln!(out, "    {v}({}),", self.ty(m)).unwrap();
        }
        if let Some(u) = &unknown {
            writeln!(
                out,
                "    /// A discriminator value the definition does not list.\n    {u}({}),",
                self.value()
            )
            .unwrap();
        }
        out.push_str("}\n\n");
        writeln!(
            out,
            "{}::tagged_union! {{\n    {name}, tag {:?} {{",
            self.rt, t.property
        )
        .unwrap();
        for (variant, v) in t.variants.iter().zip(&variants) {
            let values: Vec<String> = variant.values.iter().map(|x| format!("{x:?}")).collect();
            writeln!(out, "        {v}: [{}],", values.join(", ")).unwrap();
        }
        out.push_str("    }\n");
        if t.default.is_some() {
            writeln!(out, "    default {};", variants[t.variants.len()]).unwrap();
        }
        if let Some(u) = &unknown {
            writeln!(out, "    unknown {u};").unwrap();
        }
        out.push_str("}\n");
        out
    }

    fn untagged(&mut self, name: &str, members: &[Expr]) -> String {
        let refs: Vec<&Expr> = members.iter().collect();
        let variants = self.variant_names(&refs);
        let mut out = self.derive("");
        writeln!(out, "#[serde(untagged)]\npub enum {name} {{").unwrap();
        for (m, v) in members.iter().zip(&variants) {
            if matches!(m, Expr::Null) {
                writeln!(out, "    {v},").unwrap();
            } else {
                writeln!(out, "    {v}({}),", self.ty(m)).unwrap();
            }
        }
        out.push_str("}\n");
        out
    }

    /// The Rust type of an expression.
    pub(crate) fn ty(&mut self, e: &Expr) -> String {
        match e {
            Expr::Any
            | Expr::Never
            | Expr::InlineRecord(_)
            | Expr::Intersection(_)
            | Expr::Union(_) => self.value(),
            Expr::Null => "()".into(),
            Expr::Bool => "bool".into(),
            // Binary values in types are sent (bodies and multipart parts); responses are bytes.
            Expr::Bytes => format!("{}::File", self.rt),
            Expr::Scalar(s) => match (s.kind, s.format.as_deref()) {
                (ScalarKind::Integer, Some("int32")) => "i32".into(),
                (ScalarKind::Integer, _) => "i64".into(),
                (ScalarKind::Number, _) => "f64".into(),
                (ScalarKind::String, _) => "String".into(),
            },
            Expr::Literal(l) => match l {
                Literal::String(_) => "String".into(),
                Literal::Number(n) if n.is_i64() || n.is_u64() => "i64".into(),
                Literal::Number(_) => "f64".into(),
                Literal::Boolean(_) => "bool".into(),
            },
            Expr::List(t) => format!("Vec<{}>", self.ty(t)),
            Expr::Map(t) => format!("std::collections::BTreeMap<String, {}>", self.ty(t)),
            Expr::Tuple {
                items,
                required,
                rest,
            } => {
                if rest.is_none() && *required == items.len() && !items.is_empty() {
                    let parts: Vec<String> = items.iter().map(|i| self.ty(i)).collect();
                    if parts.len() == 1 {
                        format!("({},)", parts[0])
                    } else {
                        format!("({})", parts.join(", "))
                    }
                } else {
                    format!("Vec<{}>", self.value())
                }
            }
            Expr::Named { decl, .. } => {
                let name = self.names.types[decl.0].clone();
                self.used.insert(name.clone());
                name
            }
            Expr::Nullable(t) => format!("Option<{}>", self.ty(t)),
            Expr::External { entries, schema } => match entries.get("rust") {
                Some(ext) if crate::external::check(&ext.code).is_ok() => ext.code.clone(),
                Some(ext) => {
                    let why = crate::external::check(&ext.code).unwrap_err();
                    self.warnings.push(Warning {
                        kind: WarningKind::InvalidDefinition,
                        loc: self.loc.clone(),
                        message: format!("{why}; the schema's type is used"),
                        language: None,
                    });
                    self.ty(schema)
                }
                None => {
                    self.warnings.push(Warning {
                        kind: WarningKind::UnmappedType,
                        loc: self.loc.clone(),
                        message: "no `rust` entry; the schema's type is used".into(),
                        language: None,
                    });
                    self.ty(schema)
                }
            },
        }
    }
}

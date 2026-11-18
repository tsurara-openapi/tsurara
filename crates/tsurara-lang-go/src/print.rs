//! Prints the lowered module as one Go package (docs/languages/go.md), formatted as `gofmt` would.

use crate::naming::{GoPolicy, allocate};
use crate::{GoOptions, HEADER};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_backend::manifest::{
    Export, Manifest, OperationEntry, OperationKind, TypeEntry, in_model_order,
};
use tsurara_lower::tir::*;
use tsurara_naming::{Context, Names, NamingPolicy};
use tsurara_sem::ir::{Doc, Literal, ScalarKind};
use tsurara_sem::naming::NameKind;
use tsurara_sem::{Model, Options, Warning, WarningKind};

/// The name the runtime package is imported as.
pub(crate) const RT: &str = "tsurara";

pub(crate) struct Assigned {
    pub(crate) types: Vec<String>,
    pub(crate) files: Vec<String>,
    /// The constants of each enum declaration, by declaration index.
    pub(crate) constants: BTreeMap<usize, Vec<String>>,
}

/// The base type of an enum, when its values have one.
fn enum_base(e: &EnumDecl) -> Option<&'static str> {
    let all = |f: fn(&Literal) -> bool| !e.values.is_empty() && e.values.iter().all(f);
    if all(|v| matches!(v, Literal::String(_))) {
        Some("string")
    } else if all(|v| matches!(v, Literal::Number(n) if n.is_i64() || n.is_u64())) {
        Some("int64")
    } else if all(|v| matches!(v, Literal::Number(_))) {
        Some("float64")
    } else if all(|v| matches!(v, Literal::Boolean(_))) {
        Some("bool")
    } else {
        None
    }
}

fn literal_text(l: &Literal) -> String {
    match l {
        Literal::String(s) => s.clone(),
        Literal::Number(n) => n.to_string(),
        Literal::Boolean(b) => b.to_string(),
    }
}

/// Names declarations in model order (the model's name requests, then hoisted declarations), then
/// the constants of enums. Everything shares the package scope, which stays claimed in the returned
/// names for the operations.
fn assign(model: &Model, module: &Module, opts: &Options) -> (Assigned, Names) {
    let mut names = Names::new();
    let mut types = vec![String::new(); module.decls.len()];
    let claim = |names: &mut Names, context: Context, raw: &str, pointer: &str| {
        let id = GoPolicy.convert(context, raw, false);
        names
            .claim(&GoPolicy, context, "package", &id, pointer)
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
            // Functions keep the operation's name: a schema with the same name moves aside, as
            // `Body` when it is that operation's request body, otherwise as `Model`.
            let ident = GoPolicy.convert(Context::Type, &raw, false);
            let raw = match model
                .operations
                .iter()
                .find(|op| GoPolicy.convert(Context::Function, &op.id, false) == ident)
            {
                Some(op) => {
                    let body =
                        op.body.iter().flat_map(|b| &b.contents).any(
                            |c| matches!(&c.ty, tsurara_sem::ir::Type::Named(n) if n.id == id),
                        );
                    format!("{raw}{}", if body { "Body" } else { "Model" })
                }
                None => raw,
            };
            types[i] = claim(&mut names, Context::Type, &raw, &pointer);
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
            types[i] = claim(&mut names, Context::Type, &raw, &pointer);
        }
    }
    let mut constants = BTreeMap::new();
    for (i, d) in module.decls.iter().enumerate() {
        if let DeclKind::Enum(e) = &d.kind {
            if enum_base(e).is_some() {
                let consts = e
                    .values
                    .iter()
                    .map(|v| {
                        let raw = format!("{}_{}", types[i], literal_text(v));
                        let pointer = format!("#{}/{}", d.loc.pointer, literal_text(v));
                        claim(&mut names, Context::Constant, &raw, &pointer)
                    })
                    .collect();
                constants.insert(i, consts);
            }
        }
    }
    (
        Assigned {
            types,
            files: Vec::new(),
            constants,
        },
        names,
    )
}

pub(crate) struct Printer<'a> {
    pub(crate) names: &'a Assigned,
    /// Aliases that contain themselves through aliases, printed as defined types.
    pub(crate) recursive: &'a HashSet<usize>,
    pub(crate) warnings: &'a mut Vec<Warning>,
    pub(crate) loc: tsurara_loader::Loc,
    pub(crate) uses_rt: bool,
    pub(crate) uses_json: bool,
    /// Standard library packages the file uses besides `encoding/json`.
    pub(crate) std: std::collections::BTreeSet<&'static str>,
    /// External imports: path and alias.
    pub(crate) imports: BTreeMap<String, Option<String>>,
}

fn printer<'a>(
    names: &'a Assigned,
    recursive: &'a HashSet<usize>,
    warnings: &'a mut Vec<Warning>,
    loc: tsurara_loader::Loc,
) -> Printer<'a> {
    Printer {
        names,
        recursive,
        warnings,
        loc,
        uses_rt: false,
        uses_json: false,
        std: Default::default(),
        imports: BTreeMap::new(),
    }
}

pub fn print(
    model: &Model,
    module: &Module,
    opts: &Options,
    go: &GoOptions,
) -> (Vec<OutputFile>, Vec<Warning>, Manifest) {
    let (mut names, mut scope) = assign(model, module, opts);
    let ops: Vec<crate::operations::OperationNames> = model
        .operations
        .iter()
        .map(|op| crate::operations::claim(op, &mut scope))
        .collect();
    // File names: `tsurara_gen.go` is the package file, then the types, then the operations.
    // A space keeps `_op` apart from a name that ends in a digit (`get_x2_op`).
    let op_raws: Vec<String> = ops.iter().map(|o| format!("{} op", o.function)).collect();
    let mut raws: Vec<&str> = vec!["tsurara"];
    raws.extend(names.types.iter().map(String::as_str));
    raws.extend(op_raws.iter().map(String::as_str));
    let all = allocate(Context::File, &raws);
    names.files = all[1..1 + module.decls.len()].to_vec();
    let op_files = &all[1 + module.decls.len()..];
    let recursive = recursive_aliases(module);
    let mut warnings = Vec::new();
    let mut files = Vec::new();
    let mut manifest = Manifest::new("go");
    manifest.types = in_model_order(
        model,
        module
            .decls
            .iter()
            .enumerate()
            .filter_map(|(i, d)| match d.origin {
                Origin::Schema(name) => Some(TypeEntry {
                    name,
                    export: Export::new(&names.types[i], format!("{}_gen.go", names.files[i])),
                }),
                Origin::Candidate => None,
            })
            .collect(),
    );
    for (i, d) in module.decls.iter().enumerate() {
        let mut p = printer(&names, &recursive, &mut warnings, d.loc.clone());
        let body = p.decl(i, d);
        let imports = p.import_block(&go.runtime_module);
        files.push(OutputFile {
            path: format!("{}_gen.go", names.files[i]),
            contents: format!("{HEADER}\n\npackage {}\n\n{imports}{body}", go.package),
        });
    }
    for (((op, types), op_names), file) in model
        .operations
        .iter()
        .zip(&module.operations)
        .zip(&ops)
        .zip(op_files)
    {
        let mut p = printer(&names, &recursive, &mut warnings, op.loc.clone());
        p.std.extend(["context", "sync", "time"]);
        let body = crate::operations::operation(&mut p, op, types, op_names);
        let imports = p.import_block(&go.runtime_module);
        let path = format!("{file}_gen.go");
        let mut types =
            BTreeMap::from([("options".to_owned(), Export::new(&op_names.options, &path))]);
        if let Some(body) = &op_names.body {
            types.insert("body".to_owned(), Export::new(body, &path));
        }
        manifest.operations.push(OperationEntry {
            helpers: Default::default(),
            operation_id: op.id.clone(),
            kind: OperationKind::Operation,
            function: Some(Export::new(&op_names.function, &path)),
            operation_def: None,
            types,
        });
        files.push(OutputFile {
            path,
            contents: format!("{HEADER}\n\npackage {}\n\n{imports}{body}", go.package),
        });
    }
    if go.mocks {
        let contents = mocks_file(
            model,
            module,
            &names,
            &recursive,
            &ops,
            &mut scope,
            go,
            &mut warnings,
        );
        files.push(OutputFile {
            path: "mocks_gen.go".into(),
            contents,
        });
    }
    let vendored = opts.runtime == tsurara_sem::RuntimeSource::Vendor;
    // A vendored runtime is its own version: no marker.
    let requires = if vendored {
        String::new()
    } else {
        format!(
            "//\n// Requires {} {}\n",
            go.runtime_module,
            crate::RUNTIME_VERSION
        )
    };
    files.push(OutputFile {
        path: "tsurara_gen.go".into(),
        contents: format!(
            "{HEADER}\n\n// Package {0} holds the types and functions tsurara generated from an OpenAPI\n// definition.\n{requires}package {0}\n",
            go.package,
        ),
    });
    if vendored {
        crate::vendor::vendor(&mut files, &go.runtime_module);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    (files, warnings, manifest)
}

/// The declarations an expression names through aliases' own structure.
fn named_decls(e: &Expr, out: &mut Vec<DeclId>) {
    match e {
        Expr::Named { decl, .. } => out.push(*decl),
        Expr::List(t) | Expr::Map(t) | Expr::Nullable(t) => named_decls(t, out),
        Expr::External { schema, .. } => named_decls(schema, out),
        _ => {}
    }
}

/// Aliases that reach themselves through aliases alone: Go type aliases cannot be recursive.
fn recursive_aliases(module: &Module) -> HashSet<usize> {
    let reaches = |start: usize| {
        let mut seen = HashSet::new();
        let mut stack = Vec::new();
        if let DeclKind::Alias(e) = &module.decls[start].kind {
            named_decls(e, &mut stack);
        }
        while let Some(id) = stack.pop() {
            if id.0 == start {
                return true;
            }
            if seen.insert(id.0) {
                if let DeclKind::Alias(e) = &module.decls[id.0].kind {
                    named_decls(e, &mut stack);
                }
            }
        }
        false
    };
    (0..module.decls.len())
        .filter(|&i| matches!(module.decls[i].kind, DeclKind::Alias(_)) && reaches(i))
        .collect()
}

/// A Go interpreted string literal.
pub(crate) fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                write!(out, "\\x{:02x}", c as u32).unwrap();
            }
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{feff}') => {
                write!(out, "\\u{:04x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Doc comment lines. Lines lose their indentation and blank lines are dropped, so that gofmt finds
/// no code blocks, lists or headings to reformat; doubled backquotes and single quotes become the
/// quotation marks gofmt would turn them into.
pub(crate) fn comment(doc: Option<&Doc>, indent: &str) -> String {
    let Some(text) = doc.and_then(|d| d.description.as_deref()) else {
        return String::new();
    };
    let mut out = String::new();
    for line in tsurara_sem::text::lines(text) {
        let line = line
            .replace('\t', " ")
            .replace("``", "\u{201c}")
            .replace("''", "\u{201d}");
        let line = line.trim();
        // gofmt may read a line that starts with `#` as a heading, whose spaces it collapses.
        let line = if line.starts_with('#') {
            line.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            line.to_owned()
        };
        if !line.is_empty() {
            writeln!(out, "{indent}// {line}").unwrap();
        }
    }
    out
}

/// Text of the definition inside a line of a comment: on one line, with the quotation marks gofmt
/// would turn doubled backquotes and single quotes into, and without trailing spaces.
pub(crate) fn comment_inline(text: &str) -> String {
    tsurara_sem::text::single_line(text)
        .replace("``", "\u{201c}")
        .replace("''", "\u{201d}")
        .trim_end()
        .to_owned()
}

/// A line of a block that gofmt aligns: cells, or text that ends the alignment (a comment).
pub(crate) enum Line {
    Cells(Vec<String>),
    Text(String),
}

/// Aligns cells as gofmt's tabwriter does: a column is padded to its widest cell (plus one space)
/// over each run of consecutive lines that have a cell after it; the last cell of a line is not
/// padded.
pub(crate) fn aligned(lines: &[Line], indent: &str) -> String {
    let width = |s: &str| s.chars().count();
    let mut out = String::new();
    let mut start = 0;
    while start < lines.len() {
        if let Line::Text(t) = &lines[start] {
            writeln!(out, "{indent}{t}").unwrap();
            start += 1;
            continue;
        }
        let mut end = start;
        while end < lines.len() && matches!(lines[end], Line::Cells(_)) {
            end += 1;
        }
        let rows: Vec<&Vec<String>> = lines[start..end]
            .iter()
            .map(|l| match l {
                Line::Cells(c) => c,
                Line::Text(_) => unreachable!(),
            })
            .collect();
        let columns = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut widths = vec![vec![0; columns]; rows.len()];
        for j in 0..columns {
            let mut i = 0;
            while i < rows.len() {
                if j + 1 >= rows[i].len() {
                    i += 1;
                    continue;
                }
                let mut k = i;
                let mut w = 0;
                while k < rows.len() && j + 1 < rows[k].len() {
                    w = w.max(width(&rows[k][j]));
                    k += 1;
                }
                for row in widths.iter_mut().take(k).skip(i) {
                    row[j] = w;
                }
                i = k;
            }
        }
        for (r, row) in rows.iter().enumerate() {
            out.push_str(indent);
            for (j, cell) in row.iter().enumerate() {
                if j + 1 < row.len() {
                    out.push_str(cell);
                    out.push_str(&" ".repeat(widths[r][j] - width(cell) + 1));
                } else {
                    out.push_str(cell);
                }
            }
            out.push('\n');
        }
        start = end;
    }
    out
}

/// Whether encoding/json accepts a name in a struct tag.
fn valid_tag(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || "!#$%&()*+-./:;<=>?@[]^_{|}~ ".contains(c))
}

/// A type that is nil when unset already; others get a pointer.
fn nilable(ty: &str) -> bool {
    ty.starts_with('*')
        || ty.starts_with("[]")
        || ty.starts_with("map[")
        || ty == "any"
        || ty == "json.RawMessage"
}

fn pointer(ty: String) -> String {
    if nilable(&ty) { ty } else { format!("*{ty}") }
}

impl Printer<'_> {
    fn rt(&mut self, item: &str) -> String {
        self.uses_rt = true;
        format!("{RT}.{item}")
    }

    fn import_block(&self, runtime_module: &str) -> String {
        let mut std = Vec::new();
        let mut other = Vec::new();
        if self.uses_json {
            std.push(("encoding/json".to_owned(), String::new()));
        }
        std.extend(self.std.iter().map(|p| (p.to_string(), String::new())));
        if self.uses_rt {
            other.push((runtime_module.to_owned(), format!("{RT} ")));
        }
        for (path, alias) in &self.imports {
            let alias = alias.as_ref().map(|a| format!("{a} ")).unwrap_or_default();
            let first = path.split('/').next().unwrap_or_default();
            if first.contains('.') {
                other.push((path.clone(), alias));
            } else {
                std.push((path.clone(), alias));
            }
        }
        if std.is_empty() && other.is_empty() {
            return String::new();
        }
        std.sort();
        std.dedup();
        other.sort();
        other.dedup();
        let mut out = String::from("import (\n");
        for (path, alias) in &std {
            writeln!(out, "\t{alias}{}", quote(path)).unwrap();
        }
        if !std.is_empty() && !other.is_empty() {
            out.push('\n');
        }
        for (path, alias) in &other {
            writeln!(out, "\t{alias}{}", quote(path)).unwrap();
        }
        out.push_str(")\n\n");
        out
    }

    fn decl(&mut self, i: usize, d: &Decl) -> String {
        let name = self.names.types[i].clone();
        let mut out = comment(d.doc.as_ref(), "");
        match &d.kind {
            DeclKind::Record(r) => out.push_str(&self.record(&name, r)),
            DeclKind::Enum(e) => out.push_str(&self.enumeration(i, &name, e)),
            // Named string types keep any value, so only discriminators need a fallback where values
            // are received (`strict_decoding` is off for enums).
            DeclKind::TaggedUnion(t) => out.push_str(&self.tagged(&name, t, d.sites.received)),
            DeclKind::UntaggedUnion(u) => {
                let members: Vec<&Expr> = u.members.iter().collect();
                out.push_str(&self.untagged(&name, &members));
            }
            DeclKind::Alias(e) => {
                let ty = self.ty(e);
                if self.recursive.contains(&i) {
                    writeln!(out, "type {name} {ty}").unwrap();
                } else {
                    writeln!(out, "type {name} = {ty}").unwrap();
                }
            }
        }
        out
    }

    fn record(&mut self, name: &str, r: &Record) -> String {
        let mut raws: Vec<&str> = r.fields.iter().map(|f| f.wire_name.as_str()).collect();
        if r.additional.is_some() {
            raws.push("AdditionalProperties");
        }
        let members = allocate(Context::Member, &raws);
        let mut lines = Vec::new();
        for (f, member) in r.fields.iter().zip(&members) {
            if !valid_tag(&f.wire_name) {
                self.warnings.push(Warning {
                    kind: WarningKind::Unsupported,
                    loc: self.loc.clone(),
                    message: format!(
                        "the property {:?} of `{name}` cannot be named in a Go struct tag; it is left out",
                        f.wire_name
                    ),
                    language: None,
                });
                continue;
            }
            let doc = comment(f.doc.as_ref(), "");
            for l in doc.lines() {
                lines.push(Line::Text(l.to_owned()));
            }
            let (ty, omit) = self.field_type(f);
            let options = if omit { ",omitzero" } else { "" };
            lines.push(Line::Cells(vec![
                member.clone(),
                ty,
                format!("`json:\"{}{options}\"`", f.wire_name),
            ]));
        }
        if let Some(extra) = &r.additional {
            let ty = self.ty(extra);
            lines.push(Line::Cells(vec![
                members.last().expect("additional").clone(),
                format!("map[string]{ty}"),
                "`json:\"-\"`".into(),
            ]));
        }
        let mut out = if lines.is_empty() {
            format!("type {name} struct{{}}\n")
        } else {
            format!("type {name} struct {{\n{}}}\n", aligned(&lines, "\t"))
        };
        if r.additional.is_some() {
            let field = members.last().expect("additional");
            let marshal = self.rt("MarshalWithExtra");
            let unmarshal = self.rt("UnmarshalWithExtra");
            write!(
                out,
                "\nfunc (v {name}) MarshalJSON() ([]byte, error) {{\n\ttype plain {name}\n\treturn {marshal}(plain(v), v.{field})\n}}\n\nfunc (v *{name}) UnmarshalJSON(data []byte) error {{\n\ttype plain {name}\n\treturn {unmarshal}(data, (*plain)(v), &v.{field})\n}}\n"
            )
            .unwrap();
        }
        out
    }

    /// The type of a field and whether it is left out when zero (missing).
    fn field_type(&mut self, f: &Field) -> (String, bool) {
        let boxed = |ty: String| if f.boxed { pointer(ty) } else { ty };
        match (f.presence, &f.ty) {
            (Presence::OptionalNullable, e) => {
                let inner = match e {
                    Expr::Nullable(t) => t.as_ref(),
                    e => e,
                };
                let ty = boxed(self.ty(inner));
                (format!("{}[{ty}]", self.rt("Nullable")), true)
            }
            (Presence::Required, e) => (boxed(self.ty(e)), false),
            (Presence::Nullable, e) => (pointer(self.ty(e)), false),
            (Presence::Optional, e) => (pointer(self.ty(e)), true),
        }
    }

    fn enumeration(&mut self, i: usize, name: &str, e: &EnumDecl) -> String {
        let Some(base) = enum_base(e) else {
            return format!("type {name} = any\n");
        };
        let consts = &self.names.constants[&i];
        let mut lines = Vec::new();
        for (v, c) in e.values.iter().zip(consts) {
            let value = match v {
                Literal::String(s) => quote(s),
                other => literal_text(other),
            };
            lines.push(Line::Cells(vec![
                c.clone(),
                name.to_owned(),
                format!("= {value}"),
            ]));
        }
        format!(
            "type {name} {base}\n\nconst (\n{})\n",
            aligned(&lines, "\t")
        )
    }

    /// Field names for union members: the type's name, or its kind.
    fn member_names(&self, members: &[&Expr], extra: &[&str]) -> Vec<String> {
        let mut raws: Vec<String> = members
            .iter()
            .map(|m| match m {
                Expr::Named { decl, .. } => self.names.types[decl.0].clone(),
                Expr::Scalar(s) => match s.kind {
                    ScalarKind::Integer => "Integer".into(),
                    ScalarKind::Number => "Number".into(),
                    ScalarKind::String => "String".into(),
                },
                Expr::Literal(Literal::String(_)) => "String".into(),
                Expr::Literal(Literal::Number(_)) => "Number".into(),
                Expr::Literal(Literal::Boolean(_)) | Expr::Bool => "Bool".into(),
                Expr::Null => "Null".into(),
                Expr::List(_) | Expr::Tuple { .. } => "List".into(),
                Expr::Map(_) => "Map".into(),
                Expr::Bytes => "File".into(),
                _ => "Value".into(),
            })
            .collect();
        raws.extend(extra.iter().map(|s| s.to_string()));
        let refs: Vec<&str> = raws.iter().map(String::as_str).collect();
        allocate(Context::Member, &refs)
    }

    /// The field of a union member: a pointer (nil when it is not the value), or a flag for null.
    fn member_type(&mut self, e: &Expr) -> String {
        match e {
            Expr::Null => "bool".into(),
            Expr::Nullable(t) => self.member_type(t),
            e => format!("*{}", self.ty(e)),
        }
    }

    fn union_struct(&mut self, name: &str, fields: &[(String, String)]) -> String {
        let lines: Vec<Line> = fields
            .iter()
            .map(|(n, t)| Line::Cells(vec![n.clone(), t.clone()]))
            .collect();
        format!("type {name} struct {{\n{}}}\n", aligned(&lines, "\t"))
    }

    fn tagged(&mut self, name: &str, t: &TaggedUnion, received: bool) -> String {
        let mut members: Vec<&Expr> = t.variants.iter().map(|v| &v.ty).collect();
        if let Some(d) = &t.default {
            members.push(d);
        }
        let unknown = received && t.default.is_none();
        let names = self.member_names(&members, if unknown { &["Unknown"] } else { &[] });
        let mut fields: Vec<(String, String)> = members
            .iter()
            .zip(&names)
            .map(|(m, n)| (n.clone(), self.member_type(m)))
            .collect();
        if unknown {
            self.uses_json = true;
            fields.push((names[members.len()].clone(), "json.RawMessage".into()));
        }
        let mut out = self.union_struct(name, &fields);
        let marshal = self.rt("MarshalUnion");
        let args: Vec<String> = fields.iter().map(|(n, _)| format!("u.{n}")).collect();
        write!(
            out,
            "\nfunc (u {name}) MarshalJSON() ([]byte, error) {{\n\treturn {marshal}({})\n}}\n",
            args.join(", ")
        )
        .unwrap();
        let unmarshal = self.rt("UnmarshalTagged");
        let case = self.rt("TagCase");
        let into = self.rt("Into");
        let fallback = if let Some(n) = t.default.as_ref().map(|_| &names[t.variants.len()]) {
            format!("{into}(&u.{n})")
        } else if unknown {
            format!("{}(&u.{})", self.rt("IntoRaw"), names[members.len()])
        } else {
            "nil".into()
        };
        write!(
            out,
            "\nfunc (u *{name}) UnmarshalJSON(data []byte) error {{\n\t*u = {name}{{}}\n\treturn {unmarshal}(data, {}, []{case}{{\n",
            quote(&t.property)
        )
        .unwrap();
        for (v, n) in t.variants.iter().zip(&names) {
            let values: Vec<String> = v.values.iter().map(|x| quote(x)).collect();
            writeln!(
                out,
                "\t\t{{Values: []string{{{}}}, Decode: {into}(&u.{n})}},",
                values.join(", ")
            )
            .unwrap();
        }
        writeln!(out, "\t}}, {fallback})\n}}").unwrap();
        out
    }

    fn untagged(&mut self, name: &str, members: &[&Expr]) -> String {
        let names = self.member_names(members, &[]);
        let fields: Vec<(String, String)> = members
            .iter()
            .zip(&names)
            .map(|(m, n)| (n.clone(), self.member_type(m)))
            .collect();
        let mut out = self.union_struct(name, &fields);
        let mut encode = Vec::new();
        let mut decode = Vec::new();
        for (m, n) in members.iter().zip(&names) {
            if matches!(m, Expr::Null) {
                encode.push(format!("{}(u.{n})", self.rt("NullMember")));
                decode.push(format!("{}(&u.{n})", self.rt("IntoNull")));
            } else {
                encode.push(format!("u.{n}"));
                decode.push(format!("{}(&u.{n})", self.rt("Into")));
            }
        }
        let marshal = self.rt("MarshalUnion");
        let unmarshal = self.rt("UnmarshalUntagged");
        write!(
            out,
            "\nfunc (u {name}) MarshalJSON() ([]byte, error) {{\n\treturn {marshal}({})\n}}\n\nfunc (u *{name}) UnmarshalJSON(data []byte) error {{\n\t*u = {name}{{}}\n\treturn {unmarshal}(data, {})\n}}\n",
            encode.join(", "),
            decode.join(", ")
        )
        .unwrap();
        out
    }

    /// The Go type of an expression.
    pub(crate) fn ty(&mut self, e: &Expr) -> String {
        match e {
            Expr::Any
            | Expr::Never
            | Expr::Null
            | Expr::InlineRecord(_)
            | Expr::Intersection(_)
            | Expr::Union(_)
            | Expr::Tuple { .. } => match e {
                Expr::Tuple { .. } => "[]any".into(),
                _ => "any".into(),
            },
            Expr::Bool => "bool".into(),
            Expr::Bytes => self.rt("File"),
            Expr::Scalar(s) => match (s.kind, s.format.as_deref()) {
                (ScalarKind::Integer, Some("int32")) => "int32".into(),
                (ScalarKind::Integer, _) => "int64".into(),
                (ScalarKind::Number, _) => "float64".into(),
                (ScalarKind::String, _) => "string".into(),
            },
            Expr::Literal(l) => match l {
                Literal::String(_) => "string".into(),
                Literal::Number(n) if n.is_i64() || n.is_u64() => "int64".into(),
                Literal::Number(_) => "float64".into(),
                Literal::Boolean(_) => "bool".into(),
            },
            Expr::List(t) => format!("[]{}", self.ty(t)),
            Expr::Map(t) => format!("map[string]{}", self.ty(t)),
            Expr::Named { decl, .. } => self.names.types[decl.0].clone(),
            Expr::Nullable(t) => {
                let inner = self.ty(t);
                pointer(inner)
            }
            Expr::External { entries, schema } => match entries.get("go") {
                Some(ext) => match crate::external::check(ext) {
                    Ok(()) => {
                        if let Some(import) = &ext.import {
                            self.imports.insert(
                                import.from.clone(),
                                crate::external::alias(ext).map(str::to_owned),
                            );
                        }
                        ext.code.clone()
                    }
                    Err(why) => {
                        self.warnings.push(Warning {
                            kind: WarningKind::InvalidDefinition,
                            loc: self.loc.clone(),
                            message: format!("{why}; the schema's type is used"),
                            language: None,
                        });
                        self.ty(schema)
                    }
                },
                None => {
                    self.warnings.push(Warning {
                        kind: WarningKind::UnmappedType,
                        loc: self.loc.clone(),
                        message: "no `go` entry; the schema's type is used".into(),
                        language: None,
                    });
                    self.ty(schema)
                }
            },
        }
    }
}

/// A call example for `tsurara inspect operation`: `example.go` in the generated package,
/// which calls the function with the operation's sample values (scalars as constants, other values
/// decoded from JSON into the argument's type) and prints the result.
pub(crate) fn example(
    model: &Model,
    module: &Module,
    opts: &Options,
    go: &GoOptions,
    input: &tsurara_backend::ExampleInput,
) -> Option<tsurara_backend::Example> {
    let base_url = tsurara_backend::example_base_url(model);
    let op = input.operation;
    let index = model.operations.iter().position(|o| o.id == op.id)?;
    let types = module.operations.get(index)?;
    let function = input
        .manifest
        .operations
        .iter()
        .find(|e| e.operation_id == op.id)?
        .function
        .as_ref()?;
    let (names, _) = assign(model, module, opts);
    let recursive = recursive_aliases(module);
    let mut warnings = Vec::new();
    let mut p = printer(&names, &recursive, &mut warnings, op.loc.clone());
    let mut decodes = false;
    let mut value = |p: &mut Printer, e: &Expr, v: &serde_json::Value| -> String {
        let scalar = matches!(e, Expr::Bool | Expr::Scalar(_))
            && matches!(
                v,
                serde_json::Value::String(_)
                    | serde_json::Value::Number(_)
                    | serde_json::Value::Bool(_)
            );
        if scalar && !matches!(e, Expr::Scalar(s) if s.format.is_some()) {
            return v.to_string();
        }
        decodes = true;
        let json = serde_json::to_string(&v.to_string()).expect("strings serialize");
        format!("decodeExample[{}]({json})", p.ty(e))
    };
    let mut args = vec!["context.Background()".to_owned(), "c".to_owned()];
    for (i, q) in op.params.iter().enumerate().filter(|(_, q)| q.required) {
        let sample = input
            .samples
            .params
            .iter()
            .find(|s| s.name == q.name && s.location == q.location)?;
        args.push(value(&mut p, &types.params[i], &sample.value));
    }
    if let Some(body) = op.body.as_ref().filter(|b| b.required) {
        // A body with several media types is a struct of fields; leave it to the caller.
        let sample = input
            .samples
            .body
            .as_ref()
            .filter(|_| body.contents.len() == 1)?;
        args.push(value(&mut p, &types.body[0], &sample.value));
    }
    args.push("nil".into());
    p.uses_rt = true;
    p.std.extend(["context", "fmt"]);
    if decodes {
        p.uses_json = true;
    }
    let imports = p.import_block(&go.runtime_module);
    let helper = if decodes {
        "// decodeExample decodes a sample value into the argument's type.\nfunc decodeExample[T any](s string) T {\n\tvar v T\n\tif err := json.Unmarshal([]byte(s), &v); err != nil {\n\t\tpanic(err)\n\t}\n\treturn v\n}\n\n"
    } else {
        ""
    };
    let name = format!("Example{}", function.identifier);
    let code = format!(
        "package {}\n\n{imports}{helper}// {name} calls {} with sample values.\nfunc {name}() {{\n\tc := {RT}.NewClient(\"{base_url}\")\n\tres, err := {}({})\n\tif err != nil {{\n\t\tfmt.Println(err)\n\t\treturn\n\t}}\n\tfmt.Println(res.Status, res.Data)\n}}\n",
        go.package,
        function.identifier,
        function.identifier,
        args.join(", ")
    );
    Some(tsurara_backend::Example {
        file: "example.go".into(),
        code,
        note: None,
    })
}

/// `mocks_gen.go` (`go.mocks`, design.md 18.7.1), built with the tag `tsurara_mocks`:
/// `Mock{Type}()` decodes each declaration from its sample value, and per operation there is a
/// constructor per documented status and `{Function}Route(respond)` for `tsuraratest`; `MockRoutes()`
/// answers every operation with its first documented success. Names are claimed after every other
/// name of the package, so they never move a generated name.
#[allow(clippy::too_many_arguments)]
fn mocks_file(
    model: &Model,
    module: &Module,
    names: &Assigned,
    recursive: &HashSet<usize>,
    ops: &[crate::operations::OperationNames],
    scope: &mut Names,
    go: &GoOptions,
    warnings: &mut Vec<Warning>,
) -> String {
    use tsurara_sem::operation::{BodyKind, Status};
    let (samples, _) = tsurara_sample::samples(model);
    let sample_of: HashMap<_, _> = samples.types.iter().map(|(id, v)| (*id, v)).collect();
    let writable: HashSet<_> = model
        .schemas
        .iter()
        .filter(|d| d.writable)
        .map(|d| d.id)
        .collect();
    let mut claim = |raw: &str, pointer: &str| {
        let id = GoPolicy.convert(Context::Function, raw, false);
        scope
            .claim(&GoPolicy, Context::Function, "package", &id, pointer)
            .name
    };
    let quote = |s: &str| serde_json::to_string(s).expect("strings serialize");
    let decode = claim("mockDecode", "#/mocks/decode");
    let mut body = String::new();
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
        let f = claim(&format!("Mock{ty}"), &format!("#{}/mock", d.loc.pointer));
        write!(
            body,
            "\n// {f} returns a {ty} with the sample values of the definition.\nfunc {f}() {ty} {{\n\treturn {decode}[{ty}]({})\n}}\n",
            quote(&sample.to_string())
        )
        .unwrap();
    }
    let mut defaults = Vec::new();
    for ((op, types), op_names) in model.operations.iter().zip(&module.operations).zip(ops) {
        let function = &op_names.function;
        let mut p = printer(names, recursive, warnings, op.loc.clone());
        let mut first_success = None;
        let index = model.operations.iter().position(|o| o.id == op.id);
        let sample = index.and_then(|i| samples.operations.get(i));
        for (r, exprs) in op.responses.iter().zip(&types.responses) {
            let (suffix, status_param, status) = match r.status {
                Status::Code(c) => (c.to_string(), String::new(), c.to_string()),
                Status::Range(d) => (
                    format!("{d}XX"),
                    "status int, ".to_owned(),
                    "status".to_owned(),
                ),
                Status::Default => (
                    "Default".to_owned(),
                    "status int, ".to_owned(),
                    "status".to_owned(),
                ),
            };
            let ctor = claim(
                &format!("{function}Mock{suffix}"),
                &format!("#{}/mock/{suffix}", op.loc.pointer),
            );
            let content = r.contents.first().zip(exprs.first());
            match content {
                Some((c, e)) if c.body_kind != BodyKind::None => {
                    let (ty, value) = match c.body_kind {
                        BodyKind::Json | BodyKind::Auto => {
                            (p.ty(e), format!("tsuraratest.JSON({status}, body)"))
                        }
                        BodyKind::Text => (
                            "string".to_owned(),
                            format!("tsuraratest.Text({status}, {}, body)", quote(&c.media_type)),
                        ),
                        _ => (
                            "[]byte".to_owned(),
                            format!(
                                "tsuraratest.Text({status}, {}, string(body))",
                                quote(&c.media_type)
                            ),
                        ),
                    };
                    write!(
                        body,
                        "\n// {ctor} is a {} response of {function}.\nfunc {ctor}({status_param}body {ty}) *tsuraratest.Response {{\n\treturn {value}\n}}\n",
                        r.status.key()
                    )
                    .unwrap();
                    if first_success.is_none() && r.status.is_success() == Some(true) {
                        let key = r.status.key();
                        let found = sample.and_then(|s| {
                            s.responses
                                .iter()
                                .find(|x| x.status == key && x.media_type == c.media_type)
                        });
                        let default_status = match r.status {
                            Status::Code(_) => String::new(),
                            Status::Range(d) => format!("{}, ", u16::from(d) * 100),
                            Status::Default => "200, ".to_owned(),
                        };
                        let value = match (c.body_kind, found) {
                            (BodyKind::Json | BodyKind::Auto, Some(s)) => {
                                Some(format!("{decode}[{ty}]({})", quote(&s.value.to_string())))
                            }
                            (BodyKind::Text, Some(s)) => {
                                Some(quote(s.value.as_str().unwrap_or(&s.value.to_string())))
                            }
                            (BodyKind::Json | BodyKind::Auto | BodyKind::Text, None) => None,
                            _ => Some("nil".to_owned()),
                        };
                        if let Some(v) = value {
                            first_success = Some(format!("{ctor}({default_status}{v})"));
                        }
                    }
                }
                _ => {
                    write!(
                        body,
                        "\n// {ctor} is a {} response of {function} without a body.\nfunc {ctor}({}) *tsuraratest.Response {{\n\treturn &tsuraratest.Response{{Status: {status}}}\n}}\n",
                        r.status.key(),
                        status_param.trim_end_matches(", ")
                    )
                    .unwrap();
                    if first_success.is_none() && r.status.is_success() == Some(true) {
                        let default_status = match r.status {
                            Status::Code(_) => String::new(),
                            Status::Range(d) => format!("{}", u16::from(d) * 100),
                            Status::Default => "200".to_owned(),
                        };
                        first_success = Some(format!("{ctor}({default_status})"));
                    }
                }
            }
        }
        let route = claim(
            &format!("{function}Route"),
            &format!("#{}/mock/route", op.loc.pointer),
        );
        let fallback = match &first_success {
            Some(d) => format!(
                "\tif respond == nil {{\n\t\trespond = func(*tsuraratest.Request) *tsuraratest.Response {{\n\t\t\treturn {d}\n\t\t}}\n\t}}\n"
            ),
            None => String::new(),
        };
        write!(
            body,
            "\n// {route} answers {} with respond{}.\nfunc {route}(respond func(*tsuraratest.Request) *tsuraratest.Response) tsuraratest.Route {{\n{fallback}\treturn tsuraratest.Path({}, {}, func(r *tsuraratest.Request) (*tsuraratest.Response, error) {{\n\t\treturn respond(r), nil\n\t}})\n}}\n",
            comment_inline(&format!("{} {}", op.method, op.path)),
            if first_success.is_some() {
                " (nil: its first documented success with the sample values)"
            } else {
                ""
            },
            quote(&op.method),
            quote(&op.path)
        )
        .unwrap();
        if first_success.is_some() {
            defaults.push(format!("{route}(nil)"));
        }
    }
    let routes = claim("MockRoutes", "#/mocks/routes");
    let list = if defaults.is_empty() {
        "nil".to_owned()
    } else {
        format!(
            "[]tsuraratest.Route{{\n\t\t{},\n\t}}",
            defaults.join(",\n\t\t")
        )
    };
    format!(
        "{HEADER}\n\n//go:build tsurara_mocks\n\npackage {}\n\nimport (\n\t\"encoding/json\"\n\n\t\"{}/tsuraratest\"\n)\n\n// {decode} decodes a sample value of the definition.\nfunc {decode}[T any](text string) T {{\n\tvar v T\n\tif err := json.Unmarshal([]byte(text), &v); err != nil {{\n\t\tpanic(err)\n\t}}\n\treturn v\n}}\n{body}\n// {routes} answers every operation with its first documented success and the sample values.\nfunc {routes}() []tsuraratest.Route {{\n\treturn {list}\n}}\n",
        go.package, go.runtime_module
    )
}

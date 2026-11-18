//! Zod schemas for response validation (`typescript.validators: zod`, design.md 8.7).
//!
//! One file per declaration under `zod/` (`zod/Pet.ts` exports `PetSchema`), and `zod/operations.ts`
//! with `responseSchemas`, the schemas per operation id, status and media type that
//! `validateResponses` in `@tsurara/client` takes. Only the Zod files import `zod`; the types and call
//! functions never do. The mapping never checks more than the definition says: constraints that the
//! TypeScript types approximate (string and number constraints, `oneOf` exclusivity, long tuples)
//! are left unchecked, and external types are `z.unknown()`.

use crate::HEADER;
use crate::options::TypeScriptOptions;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use tsurara_backend::OutputFile;
use tsurara_sem::Model;
use tsurara_sem::ir::{Constraints, Doc, Literal, Object, Scalar, ScalarKind, Type};
use tsurara_sem::naming::NameId;
use tsurara_sem::operation::BodyKind;
use tsurara_sem::print::{property_key, string_literal};
use tsurara_sem::{Warning, WarningKind};

struct Printer<'a> {
    names: &'a HashMap<NameId, String>,
    /// Declarations the current file refers to.
    refs: BTreeSet<String>,
    warnings: &'a mut Vec<Warning>,
    /// `typescript.validatorConstraints`.
    constraints: bool,
    /// Where the schema being printed is, for warnings.
    at: Option<tsurara_loader::Loc>,
}

/// The Zod files. `model` is the model before TypeScript's lowering, `lowered` the copy with the
/// names TypeScript assigned.
pub fn emit(
    model: &Model,
    lowered: &Model,
    opts: &TypeScriptOptions,
    operations: bool,
    warnings: &mut Vec<Warning>,
) -> Vec<OutputFile> {
    let names: HashMap<NameId, String> = lowered
        .schemas
        .iter()
        .map(|d| (d.id, d.name.clone()))
        .collect();
    let ext = opts.import_extension.suffix();
    let mut files = Vec::new();
    for decl in model.schemas.iter().filter(|d| !d.writable) {
        let mut p = Printer {
            names: &names,
            refs: BTreeSet::new(),
            warnings,
            constraints: opts.validator_constraints,
            at: None,
        };
        p.at = Some(decl.loc.clone());
        let expr = p.constrained(&decl.ty, decl.doc.as_ref());
        let name = &names[&decl.id];
        p.refs.remove(name);
        let mut code = format!("{HEADER}\n\nimport {{ z }} from 'zod';\n");
        for r in &p.refs {
            writeln!(code, "import {{ {r}Schema }} from './{r}{ext}';").unwrap();
        }
        write!(code, "\nexport const {name}Schema: z.ZodType = {expr};\n").unwrap();
        files.push(OutputFile {
            path: format!("zod/{name}.ts"),
            contents: code,
        });
    }

    // Response schemas per operation.
    let mut p = Printer {
        names: &names,
        refs: BTreeSet::new(),
        warnings,
        constraints: opts.validator_constraints,
        at: None,
    };
    let mut entries = String::new();
    for op in &model.operations {
        let mut statuses = Vec::new();
        for r in &op.responses {
            let mut media = Vec::new();
            for c in &r.contents {
                if matches!(c.body_kind, BodyKind::Json | BodyKind::Auto) {
                    media.push(format!(
                        "{}: {}",
                        string_literal(&c.media_type),
                        p.schema(&c.ty)
                    ));
                }
            }
            if !media.is_empty() {
                statuses.push(format!(
                    "    {}: {{ {} }},",
                    property_key(&r.status.key()),
                    media.join(", ")
                ));
            }
        }
        if !statuses.is_empty() {
            writeln!(
                entries,
                "  {}: {{\n{}\n  }},",
                property_key(&op.id),
                statuses.join("\n")
            )
            .unwrap();
        }
    }
    let mut code = format!(
        "{HEADER}\n\nimport type {{ ResponseSchemas }} from '{}';\nimport {{ z }} from 'zod';\n",
        opts.client_module
    );
    for r in &p.refs {
        writeln!(code, "import {{ {r}Schema }} from './{r}{ext}';").unwrap();
    }
    write!(
        code,
        "\n/** Response schemas per operation id, status and media type, for `validateResponses`. */\nexport const responseSchemas: ResponseSchemas = {{\n{entries}}};\n"
    )
    .unwrap();
    // `z` is used by inline schemas; keep the import valid when there are none.
    if !code.contains("z.") {
        code = code.replace("import { z } from 'zod';\n", "");
    }
    // Without operations there is nothing to validate, and the file would import the
    // runtime.
    if operations {
        files.push(OutputFile {
            path: "zod/operations.ts".into(),
            contents: code,
        });
    }

    // Message schemas per event channel.
    if !model.channels.is_empty() {
        let mut p = Printer {
            names: &names,
            refs: BTreeSet::new(),
            warnings,
            constraints: opts.validator_constraints,
            at: None,
        };
        let mut entries = String::new();
        for c in &model.channels {
            if let Some(m) = c
                .contents
                .iter()
                .find(|m| matches!(m.body_kind, BodyKind::Json | BodyKind::Auto))
            {
                writeln!(entries, "  {}: {},", property_key(&c.name), p.schema(&m.ty)).unwrap();
            }
        }
        let mut code = format!(
            "{HEADER}\n\nimport type {{ ChannelSchemas }} from '{}';\nimport {{ z }} from 'zod';\n",
            opts.client_module
        );
        for r in &p.refs {
            writeln!(code, "import {{ {r}Schema }} from './{r}{ext}';").unwrap();
        }
        write!(
            code,
            "\n/** Message schemas per channel name, for `validateChannels`. */\nexport const channelSchemas: ChannelSchemas = {{\n{entries}}};\n"
        )
        .unwrap();
        if !code.contains("z.") {
            code = code.replace("import { z } from 'zod';\n", "");
        }
        files.push(OutputFile {
            path: "zod/channels.ts".into(),
            contents: code,
        });
    }
    files
}

impl Printer<'_> {
    fn schema(&mut self, t: &Type) -> String {
        match t {
            Type::Unknown
            | Type::Undefined
            | Type::Ref(_)
            | Type::Builtin(_)
            | Type::External(_) => "z.unknown()".into(),
            Type::Never => "z.never()".into(),
            Type::Null => "z.null()".into(),
            Type::String => "z.string()".into(),
            Type::Number => "z.number()".into(),
            Type::Boolean => "z.boolean()".into(),
            // Binary values are not JSON; they are never validated.
            Type::Binary => "z.unknown()".into(),
            Type::Scalar(s) => match (s.kind, s.format.as_deref()) {
                (ScalarKind::String, Some("uuid")) => "z.uuid()".into(),
                (ScalarKind::String, Some("email")) => "z.email()".into(),
                (ScalarKind::String, Some("date-time")) => {
                    "z.iso.datetime({ offset: true })".into()
                }
                // Formats that may reject values servers send, only with the constraints.
                (ScalarKind::String, Some(f)) if self.constraints => match f {
                    "uri" => "z.url()".into(),
                    "ipv4" => "z.ipv4()".into(),
                    "ipv6" => "z.ipv6()".into(),
                    "date" => "z.iso.date()".into(),
                    "time" => "z.iso.time()".into(),
                    "duration" => "z.iso.duration()".into(),
                    _ => "z.string()".into(),
                },
                (ScalarKind::String, _) => "z.string()".into(),
                (ScalarKind::Integer, _) => "z.number().int()".into(),
                (ScalarKind::Number, _) => "z.number()".into(),
            },
            Type::Literal(l) => format!("z.literal({})", literal(l)),
            Type::Enum(e) => {
                let base = match e.values.as_slice() {
                    [] => "z.never()".to_owned(),
                    [one] => format!("z.literal({})", literal(one)),
                    values if values.iter().all(|v| matches!(v, Literal::String(_))) => format!(
                        "z.enum([{}])",
                        values.iter().map(literal).collect::<Vec<_>>().join(", ")
                    ),
                    values => format!(
                        "z.union([{}])",
                        values
                            .iter()
                            .map(|v| format!("z.literal({})", literal(v)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                if e.nullable {
                    format!("{base}.nullable()")
                } else {
                    base
                }
            }
            Type::Named(n) => match self.names.get(&n.id) {
                Some(name) => {
                    self.refs.insert(name.clone());
                    // Lazy, so that cycles and the order of module evaluation do not matter.
                    format!("z.lazy(() => {name}Schema)")
                }
                None => "z.unknown()".into(),
            },
            Type::Inline(i) => self.schema(&i.ty),
            Type::Array(t) => format!("z.array({})", self.schema(t)),
            Type::Tuple(t) => {
                if t.rest.is_none() && t.required == t.items.len() {
                    let items: Vec<String> = t.items.iter().map(|i| self.schema(i)).collect();
                    format!("z.tuple([{}])", items.join(", "))
                } else {
                    // Optional elements and rest elements: checked as an array, never stricter.
                    "z.array(z.unknown())".into()
                }
            }
            Type::Union(ts) => self.union(ts),
            Type::Choice(c) => self.union(&c.members),
            Type::Intersection(ts) => {
                let mut parts = ts.iter().map(|t| self.schema(t));
                let first = parts.next().unwrap_or_else(|| "z.unknown()".into());
                parts.fold(first, |acc, p| format!("z.intersection({acc}, {p})"))
            }
            Type::Tag(t) => {
                let values: Vec<String> = t.values.iter().map(|v| string_literal(v)).collect();
                format!(
                    "z.object({{ {}: z.enum([{}]) }})",
                    property_key(&t.property),
                    values.join(", ")
                )
            }
            Type::Object(o) => self.object(o),
            Type::Mapped(m) => {
                self.warnings.push(Warning {
                    kind: WarningKind::UnmappedType,
                    loc: m.loc.clone(),
                    message: "an external type is not validated by the Zod schemas (z.unknown())"
                        .into(),
                    language: Some("typescript".into()),
                });
                "z.unknown()".into()
            }
        }
    }

    /// `ty` with the constraints of its schema (`doc`), when `validatorConstraints` is on.
    fn constrained(&mut self, ty: &Type, doc: Option<&Doc>) -> String {
        match doc {
            Some(doc) if self.constraints => {
                let c = doc.constraints();
                self.apply(ty, &c)
            }
            _ => self.schema(ty),
        }
    }

    fn apply(&mut self, ty: &Type, c: &Constraints) -> String {
        match ty {
            Type::Inline(i) => self.apply(&i.ty, c),
            // A nullable value: the constraints apply to the other members.
            Type::Union(ts) => self.union_with(ts, c),
            Type::Choice(ch) => self.union_with(&ch.members, c),
            Type::String
            | Type::Scalar(Scalar {
                kind: ScalarKind::String,
                ..
            }) => {
                let mut s = self.schema(ty);
                if let Some(n) = c.min_length {
                    write!(s, ".min({n})").unwrap();
                }
                if let Some(n) = c.max_length {
                    write!(s, ".max({n})").unwrap();
                }
                if let Some(p) = &c.pattern {
                    // Patterns are ECMA-262 by the specification; one JavaScript cannot compile with
                    // the `u` flag would throw when the module loads, so it is left out.
                    if regress::Regex::with_flags(p, "u").is_ok() {
                        write!(s, ".regex(new RegExp({}, 'u'))", string_literal(p)).unwrap();
                    } else if let Some(at) = &self.at {
                        self.warnings.push(Warning {
                            kind: WarningKind::Unsupported,
                            loc: at.clone(),
                            message: format!(
                                "the pattern {} is not a JavaScript regular expression; the Zod schema does not check it",
                                string_literal(p)
                            ),
                            language: Some("typescript".into()),
                        });
                    }
                }
                s
            }
            Type::Number
            | Type::Scalar(Scalar {
                kind: ScalarKind::Integer | ScalarKind::Number,
                ..
            }) => {
                let mut s = self.schema(ty);
                for (method, bound) in [
                    ("gte", &c.minimum),
                    ("lte", &c.maximum),
                    ("gt", &c.exclusive_minimum),
                    ("lt", &c.exclusive_maximum),
                    ("multipleOf", &c.multiple_of),
                ] {
                    if let Some(n) = bound {
                        write!(s, ".{method}({n})").unwrap();
                    }
                }
                s
            }
            Type::Array(_) => {
                let mut s = self.schema(ty);
                if let Some(n) = c.min_items {
                    write!(s, ".min({n})").unwrap();
                }
                if let Some(n) = c.max_items {
                    write!(s, ".max({n})").unwrap();
                }
                if c.unique_items {
                    s.push_str(".refine((items) => new Set(items.map((item) => JSON.stringify(item))).size === items.length, 'items must be unique')");
                }
                s
            }
            Type::Object(_) => {
                let mut s = self.schema(ty);
                if let Some(n) = c.min_properties {
                    write!(
                        s,
                        ".refine((o) => Object.keys(o).length >= {n}, 'at least {n} properties')"
                    )
                    .unwrap();
                }
                if let Some(n) = c.max_properties {
                    write!(
                        s,
                        ".refine((o) => Object.keys(o).length <= {n}, 'at most {n} properties')"
                    )
                    .unwrap();
                }
                s
            }
            _ => self.schema(ty),
        }
    }

    fn union_with(&mut self, ts: &[Type], c: &Constraints) -> String {
        let members: Vec<String> = ts
            .iter()
            .map(|t| match t {
                Type::Null => self.schema(t),
                _ => self.apply(t, c),
            })
            .collect();
        match members.as_slice() {
            [] => "z.never()".into(),
            [one] => one.clone(),
            _ => format!("z.union([{}])", members.join(", ")),
        }
    }

    fn union(&mut self, ts: &[Type]) -> String {
        let members: Vec<String> = ts.iter().map(|t| self.schema(t)).collect();
        match members.as_slice() {
            [] => "z.never()".into(),
            [one] => one.clone(),
            _ => format!("z.union([{}])", members.join(", ")),
        }
    }

    fn object(&mut self, o: &Object) -> String {
        let props: Vec<String> = o
            .properties
            .iter()
            .map(|p| {
                let s = self.constrained(&p.ty, p.doc.as_ref());
                let s = if p.optional {
                    format!("{s}.optional()")
                } else {
                    s
                };
                format!("{}: {s}", property_key(&p.name))
            })
            .collect();
        let base = if props.is_empty() {
            "z.object({})".to_owned()
        } else {
            format!("z.object({{ {} }})", props.join(", "))
        };
        match o.index.as_deref() {
            // Properties that are not listed are allowed, as in JSON Schema.
            None => base,
            Some(Type::Never) => format!("{base}.strict()"),
            Some(t) => format!("{base}.catchall({})", self.schema(t)),
        }
    }
}

fn literal(l: &Literal) -> String {
    match l {
        Literal::String(s) => string_literal(s),
        Literal::Number(n) => n.to_string(),
        Literal::Boolean(b) => b.to_string(),
    }
}

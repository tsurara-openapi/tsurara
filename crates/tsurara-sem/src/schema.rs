//! Converts JSON Schema (OpenAPI 3.0 / 3.1 / 3.2) to the IR (design.md 6.2 to 6.5).

use crate::ir::{
    Choice, Discriminator, Doc, Enum, External, Import, Inline, Literal, Mapped, NameRef, Object,
    Property, Scalar, ScalarKind, Tag, Tuple, Type,
};
use crate::naming::{NameId, NameKind, NameRequest};
use crate::options::Options;
use crate::warning::{Warning, WarningKind};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use tsurara_loader::{DocumentSet, Loc, SpecVersion};
use url::Url;

/// Context in which a type is used. Changes how readOnly/writeOnly are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Response. Drops writeOnly properties.
    Read,
    /// Request. Drops readOnly properties and points references to split schemas at the `Writable` type.
    Write,
}

/// Declaration of a named type.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct TypeDecl {
    pub id: NameId,
    /// The name the definition asks for, before any language's rules (the components key, a title,
    /// or a derived name). `Writable` declarations share it with the declaration they split from.
    pub preferred_name: String,
    /// The identifier a backend assigned. Empty in the model; backends fill it in their own copy.
    pub name: String,
    pub ty: Type,
    pub doc: Option<Doc>,
    /// Location of the original schema.
    pub loc: Loc,
    /// Whether this is the `Writable` type.
    pub writable: bool,
}

#[derive(Debug, Clone)]
struct NamedSchema {
    id: NameId,
    preferred: String,
    writable: Option<NameId>,
}

/// Keywords that only annotate and do not affect the type. Used to judge `$ref` with sibling keys.
const ANNOTATIONS: &[&str] = &[
    "description",
    "title",
    "summary",
    "deprecated",
    "readOnly",
    "writeOnly",
    "example",
    "examples",
    "default",
    "$comment",
    "externalDocs",
    "xml",
    "$id",
    "$anchor",
    "$dynamicAnchor",
    "$defs",
    "$schema",
    "discriminator",
    "format",
];

pub struct Converter<'a> {
    pub set: &'a DocumentSet,
    pub opts: &'a Options,
    pub version: SpecVersion,
    /// Name requests in the order they were made (design.md 18.4.6).
    pub names: Vec<NameRequest>,
    pub warnings: Vec<Warning>,
    named: IndexMap<Loc, NamedSchema>,
    queue: VecDeque<(Loc, Mode)>,
    declared: HashSet<(Loc, Mode)>,
    pub decls: Vec<TypeDecl>,
    visibility: HashMap<Loc, bool>,
    dynamic_names: HashMap<Loc, Vec<String>>,
    /// Dynamic scope. URLs of the schema resources being converted, outermost first.
    scope: Vec<Url>,
    /// Reference targets being expanded for `$dynamicRef`.
    expanding: HashSet<Loc>,
    /// Named types being converted (used to detect discriminator cycles).
    current: Vec<Loc>,
    /// The operation being converted and its preferred name, the owner of its inline schemas.
    pub operation_owner: Option<(Loc, String)>,
    warned: HashSet<(Loc, WarningKind)>,
    /// Locations whose `overrides` entry is being applied; their own schema is converted as the fallback.
    overriding: HashSet<Loc>,
    /// Whether the schema being converted is a root: a declaration, or an HTTP body that is not JSON.
    /// A binary string inside a JSON value warns; at a root it does not (design.md 6.4).
    at_root: bool,
    /// Set by `convert_root` for the next schema converted.
    body_root: bool,
    /// Set by `convert_multipart` for the next object: its properties are multipart part values.
    part_props: bool,
    /// Named object schemas whose only uses are as multipart request bodies (`find_multipart_only`).
    /// Their declarations are converted with part values, and the bodies refer to them.
    multipart_only: HashSet<Loc>,
}

impl<'a> Converter<'a> {
    pub fn new(set: &'a DocumentSet, opts: &'a Options, version: SpecVersion) -> Self {
        Self {
            set,
            opts,
            version,
            names: Vec::new(),
            warnings: Vec::new(),
            named: IndexMap::new(),
            queue: VecDeque::new(),
            declared: HashSet::new(),
            decls: Vec::new(),
            visibility: HashMap::new(),
            dynamic_names: HashMap::new(),
            scope: Vec::new(),
            expanding: HashSet::new(),
            current: Vec::new(),
            operation_owner: None,
            warned: HashSet::new(),
            overriding: HashSet::new(),
            at_root: false,
            body_root: false,
            part_props: false,
            multipart_only: HashSet::new(),
        }
    }

    /// Finds the schemas referenced only from multipart request bodies (design.md 6.4). Every `$ref`
    /// and discriminator mapping in every document counts as a use, so a schema also used anywhere
    /// else (a JSON body, another schema, even an unused one) stays a JSON type.
    pub fn find_multipart_only(&mut self) {
        let mut multipart = HashSet::new();
        let mut other = HashSet::new();
        let docs: Vec<(Url, &Value)> = self.set.documents().map(|(u, v)| (u.clone(), v)).collect();
        for (url, value) in docs {
            self.collect_uses(&Loc::root(url), value, &mut multipart, &mut other);
        }
        self.multipart_only = multipart
            .into_iter()
            .filter(|loc| !other.contains(loc))
            .filter(|loc| {
                // Only plain objects: their properties are what `part_props` applies to.
                matches!(self.set.get(loc), Ok(Value::Object(m))
                    if !m.contains_key("$ref") && m.get("properties").is_some_and(Value::is_object))
            })
            .collect();
    }

    fn collect_uses(
        &self,
        loc: &Loc,
        value: &Value,
        multipart: &mut HashSet<Loc>,
        other: &mut HashSet<Loc>,
    ) {
        match value {
            Value::Object(map) => {
                let target = match map.get("$ref") {
                    Some(Value::String(r)) => self.set.resolve_ref(loc, r).ok(),
                    _ => None,
                };
                if let Some(target) = target {
                    if is_multipart_body_schema(loc) {
                        multipart.insert(target);
                    } else {
                        other.insert(target);
                    }
                }
                if let Some(Value::Object(disc)) = map.get("discriminator") {
                    let mapped = disc
                        .get("mapping")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flat_map(|m| m.values())
                        .chain(disc.get("defaultMapping"));
                    for v in mapped.filter_map(Value::as_str) {
                        if let Some(t) = self.mapping_target(loc, v) {
                            other.insert(t);
                        }
                    }
                }
                for (k, v) in map {
                    self.collect_uses(&loc.join(k), v, multipart, other);
                }
            }
            Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    self.collect_uses(&loc.join(&i.to_string()), v, multipart, other);
                }
            }
            _ => {}
        }
    }

    pub fn warn(&mut self, kind: WarningKind, loc: &Loc, message: impl Into<String>) {
        if self.warned.insert((loc.clone(), kind)) {
            self.warnings.push(Warning {
                kind,
                loc: loc.clone(),
                message: message.into(),
                language: None,
            });
        }
    }

    /// Names the root's `components.schemas` in document order.
    pub fn register_components(&mut self) -> Vec<Loc> {
        let root = self.set.root_loc().join("components").join("schemas");
        let keys: Vec<String> = match self.set.get(&root) {
            Ok(Value::Object(m)) => m.keys().cloned().collect(),
            _ => Vec::new(),
        };
        let mut locs = Vec::new();
        for key in keys {
            let loc = root.join(&key);
            self.name_for(&loc);
            locs.push(loc);
        }
        locs
    }

    /// Requests a name for a declaration or anything else that needs one.
    pub fn request_name(&mut self, kind: NameKind, loc: &Loc) -> NameId {
        let id = NameId(self.names.len() as u32);
        self.names.push(NameRequest {
            id,
            kind,
            loc: loc.clone(),
            in_root: loc.doc == *self.set.root_url(),
        });
        id
    }

    /// The name request of the schema at `loc`. Requests one if it has none yet.
    fn name_for(&mut self, loc: &Loc) -> NameId {
        if let Some(n) = self.named.get(loc) {
            return n.id;
        }
        let (preferred, preserve) = self.raw_name(loc);
        let id = self.request_name(
            NameKind::Schema {
                preferred: preferred.clone(),
                preserve,
            },
            loc,
        );
        self.named.insert(
            loc.clone(),
            NamedSchema {
                id,
                preferred,
                writable: None,
            },
        );
        id
    }

    /// The string the name is based on, and whether to use it unchanged.
    fn raw_name(&self, loc: &Loc) -> (String, bool) {
        let tokens: Vec<String> = loc.tokens().collect();
        let n = tokens.len();
        if n >= 3 && tokens[n - 3] == "components" && tokens[n - 2] == "schemas" {
            return (tokens[n - 1].clone(), true);
        }
        if n >= 2 && matches!(tokens[n - 2].as_str(), "$defs" | "definitions") {
            return (tokens[n - 1].clone(), true);
        }
        const SKIP: &[&str] = &[
            "schema",
            "items",
            "additionalProperties",
            "content",
            "properties",
            "allOf",
            "oneOf",
            "anyOf",
            "not",
            "$defs",
            "definitions",
            "components",
            "schemas",
            "prefixItems",
            "patternProperties",
            "itemSchema",
            "requestBody",
            "responses",
            "parameters",
        ];
        for t in tokens.iter().rev() {
            if SKIP.contains(&t.as_str()) || t.contains('/') || t.parse::<u64>().is_ok() {
                continue;
            }
            return (t.clone(), false);
        }
        let stem = loc
            .doc
            .path_segments()
            .and_then(|mut s| s.next_back())
            .map(|f| f.split('.').next().unwrap_or(f).to_owned())
            .unwrap_or_default();
        if stem.is_empty() {
            ("Schema".to_owned(), false)
        } else {
            (stem, false)
        }
    }

    /// Returns a reference to the named type and queues its declaration.
    pub fn named_ref(&mut self, loc: &Loc, mode: Mode) -> Type {
        let id = self.name_for(loc);
        let mode = self.effective_mode(loc, mode);
        self.enqueue(loc, mode);
        let preferred = self.named[loc].preferred.clone();
        Type::Named(match mode {
            Mode::Read => NameRef {
                id,
                hint: preferred,
            },
            Mode::Write => NameRef {
                id: self.writable_name(loc),
                hint: format!("{preferred}Writable"),
            },
        })
    }

    /// For schemas that need no split, reads `Write` as `Read`.
    fn effective_mode(&mut self, loc: &Loc, mode: Mode) -> Mode {
        if mode == Mode::Write && self.opts.split_visibility && self.has_visibility(loc) {
            Mode::Write
        } else {
            Mode::Read
        }
    }

    fn writable_name(&mut self, loc: &Loc) -> NameId {
        let entry = self.named.get(loc).expect("named before").clone();
        if let Some(w) = entry.writable {
            return w;
        }
        let id = self.request_name(NameKind::Writable { of: entry.id }, loc);
        if let Some(n) = self.named.get_mut(loc) {
            n.writable = Some(id);
        }
        id
    }

    /// Queues a declaration. For a split schema, queuing `Read` also queues `Write`.
    pub fn enqueue(&mut self, loc: &Loc, mode: Mode) {
        self.name_for(loc);
        if self.declared.insert((loc.clone(), mode)) {
            self.queue.push_back((loc.clone(), mode));
        }
        if mode == Mode::Read && self.effective_mode(loc, Mode::Write) == Mode::Write {
            self.writable_name(loc);
            if self.declared.insert((loc.clone(), Mode::Write)) {
                self.queue.push_back((loc.clone(), Mode::Write));
            }
        }
    }

    /// Converts all queued declarations.
    pub fn drain(&mut self) {
        while let Some((loc, mode)) = self.queue.pop_front() {
            let decl = self.convert_decl(&loc, mode);
            self.decls.push(decl);
        }
    }

    fn convert_decl(&mut self, loc: &Loc, mode: Mode) -> TypeDecl {
        let id = match mode {
            Mode::Read => self.name_for(loc),
            Mode::Write => self.writable_name(loc),
        };
        let preferred_name = self.named[loc].preferred.clone();
        let base = self.set.base_uri(loc);
        let saved_scope = std::mem::replace(&mut self.scope, vec![base]);
        self.current.push(loc.clone());
        self.part_props = self.multipart_only.contains(loc);
        let ty = self.convert_inner(loc, mode, true);
        self.part_props = false;
        self.current.pop();
        self.scope = saved_scope;
        let doc = self.set.get(loc).ok().and_then(schema_doc);
        TypeDecl {
            id,
            preferred_name,
            name: String::new(),
            ty,
            doc,
            loc: loc.clone(),
            writable: mode == Mode::Write,
        }
    }

    /// Converts the schema at `loc` inline.
    pub fn convert(&mut self, loc: &Loc, mode: Mode) -> Type {
        self.convert_inner(loc, mode, false)
    }

    /// Converts the schema of an HTTP body that is not JSON. Binary strings at its root do not warn.
    pub fn convert_root(&mut self, loc: &Loc, mode: Mode) -> Type {
        self.body_root = true;
        let ty = self.convert(loc, mode);
        self.body_root = false;
        ty
    }

    /// Converts the schema of a multipart body. Its top-level properties are part values: binary
    /// strings (and arrays of them) become `Blob | File`, following `$ref`s. A named schema used only
    /// by multipart bodies is declared with part values and referred to; one also used elsewhere is
    /// expanded inline when it has binary parts, so that the named type stays a JSON type
    /// (design.md 6.4).
    pub fn convert_multipart(&mut self, loc: &Loc, mode: Mode) -> Type {
        let Ok((target, value)) = self.set.deref_all(loc) else {
            return self.convert(loc, mode);
        };
        if self.multipart_only.contains(&target) {
            return self.convert(loc, mode);
        }
        let keys: Vec<String> = match value.get("properties") {
            Some(Value::Object(props)) => props.keys().cloned().collect(),
            _ => Vec::new(),
        };
        let has_binary = keys.iter().any(|k| {
            self.part_binary_type(&target.join("properties").join(k))
                .is_some()
        });
        if !has_binary {
            return self.convert(loc, mode);
        }
        self.part_props = true;
        let ty = self.convert_unnamed(&target, mode, false);
        self.part_props = false;
        self.mark_candidate(loc, ty)
    }

    /// The member names of an `enum` (`x-enum-varnames`, or NSwag's `x-enumNames`): one distinct
    /// string per value. Names that do not fit the values are ignored with a warning.
    fn enum_names(
        &mut self,
        loc: &Loc,
        map: &Map<String, Value>,
        values: &[Value],
    ) -> Option<Vec<String>> {
        let (key, names) = ["x-enum-varnames", "x-enumNames"]
            .into_iter()
            .find_map(|k| map.get(k).map(|v| (k, v)))?;
        let names: Option<Vec<String>> = names.as_array().and_then(|names| {
            names
                .iter()
                .map(|n| n.as_str().map(str::to_owned))
                .collect()
        });
        let problem = match &names {
            None => Some("is not a list of strings".to_owned()),
            Some(names) if names.len() != values.len() => Some(format!(
                "has {} names for {} values",
                names.len(),
                values.len()
            )),
            Some(names) => {
                // Names of `null` and of repeated values are dropped with them; the rest must differ.
                let mut seen: Vec<&Value> = Vec::new();
                let mut kept: Vec<&str> = Vec::new();
                for (v, n) in values.iter().zip(names) {
                    if v.is_null() || seen.contains(&v) {
                        continue;
                    }
                    seen.push(v);
                    kept.push(n);
                }
                let distinct: std::collections::BTreeSet<&str> = kept.iter().copied().collect();
                (distinct.len() != kept.len()).then(|| "names a value twice".to_owned())
            }
        };
        if let Some(problem) = problem {
            self.warn(
                WarningKind::InvalidDefinition,
                &loc.join(key),
                format!("`{key}` {problem}; the members are named after their values"),
            );
            return None;
        }
        names
    }

    /// Whether the schema at `loc` (following `$ref`s) is a binary string, as the root of a body.
    pub fn is_binary_root(&self, loc: &Loc) -> bool {
        let Ok((_, Value::Object(map))) = self.set.deref_all(loc) else {
            return false;
        };
        let ty = map.get("type").and_then(Value::as_str);
        let format = map.get("format").and_then(Value::as_str);
        (ty == Some("string") && is_binary_string(map))
            || (ty.is_none() && format == Some("binary"))
    }

    /// The type of a multipart part value that is binary, or `None` to convert it as JSON.
    fn part_binary_type(&self, loc: &Loc) -> Option<Type> {
        let (target, value) = self.set.deref_all(loc).ok()?;
        let map = value.as_object()?;
        let ty = map.get("type").and_then(Value::as_str);
        let format = map.get("format").and_then(Value::as_str);
        if (ty == Some("string") && is_binary_string(map))
            || (ty.is_none() && format == Some("binary"))
        {
            return Some(binary_type());
        }
        let is_array = ty == Some("array") || (ty.is_none() && map.contains_key("items"));
        if is_array && map.get("items").is_some_and(Value::is_object) {
            return self
                .part_binary_type(&target.join("items"))
                .map(Type::array);
        }
        None
    }

    fn convert_inner(&mut self, loc: &Loc, mode: Mode, decl_root: bool) -> Type {
        // An inline schema that is referenced by `$ref` from elsewhere uses the named type.
        // A schema that refers to itself during conversion (inline self-reference) is also replaced by its name here.
        if !decl_root && self.named.contains_key(loc) {
            return self.named_ref(loc, mode);
        }
        let ty = self.convert_unnamed(loc, mode, decl_root);
        if !decl_root && self.named.contains_key(loc) {
            return self.named_ref(loc, mode);
        }
        // `allOf` members merge into their owner, so they are not candidates of their own.
        if decl_root || loc.parent().and_then(|p| p.last_token()).as_deref() == Some("allOf") {
            return ty;
        }
        self.mark_candidate(loc, ty)
    }

    /// Wraps an inline object, enum or union (also the non-null member of a nullable one) as a
    /// hoisting candidate (design.md 18.4.2).
    fn mark_candidate(&self, loc: &Loc, ty: Type) -> Type {
        let candidate = |t: &Type| matches!(t, Type::Object(_) | Type::Enum(_) | Type::Choice(_));
        let wrap = |t: Type| {
            Type::Inline(Box::new(Inline {
                preferred_name: self.candidate_name(loc),
                loc: loc.clone(),
                ty: t,
            }))
        };
        match ty {
            t if candidate(&t) => wrap(t),
            Type::Union(ts)
                if ts.iter().filter(|t| !matches!(t, Type::Null)).count() == 1
                    && ts.iter().any(candidate) =>
            {
                Type::Union(
                    ts.into_iter()
                        .map(|t| if candidate(&t) { wrap(t) } else { t })
                        .collect(),
                )
            }
            other => other,
        }
    }

    /// The preferred name of a hoisting candidate: the owner's preferred name (the declaration or
    /// operation being converted) followed by the path from the owner (design.md 18.4.2).
    fn candidate_name(&self, loc: &Loc) -> String {
        let mut tokens: Vec<String> = loc.tokens().collect();
        // A parameter is named after its `name`, not its index, so that reordering parameters does
        // not rename it.
        for i in 0..tokens.len().saturating_sub(1) {
            if tokens[i] == "parameters" && tokens[i + 1].parse::<usize>().is_ok() {
                let mut param = Loc::root(loc.doc.clone());
                for t in &tokens[..i + 2] {
                    param = param.join(t);
                }
                if let Ok((_, v)) = self.set.deref_all(&param) {
                    if let Some(name) = v.get("name").and_then(Value::as_str) {
                        tokens[i + 1] = name.to_owned();
                    }
                }
            }
        }
        let owners = self
            .current
            .iter()
            .rev()
            .filter_map(|l| self.named.get(l).map(|n| (l, n.preferred.clone())))
            .chain(self.operation_owner.iter().map(|(l, n)| (l, n.clone())));
        for (owner, name) in owners {
            let prefix: Vec<String> = owner.tokens().collect();
            if owner.doc == loc.doc && tokens.starts_with(&prefix) {
                return name + &path_name(&tokens[prefix.len()..]);
            }
        }
        tsurara_naming::pascal_case(&self.raw_name(loc).0)
    }

    fn convert_unnamed(&mut self, loc: &Loc, mode: Mode, decl_root: bool) -> Type {
        let root = decl_root || std::mem::take(&mut self.body_root);
        let saved = std::mem::replace(&mut self.at_root, root);
        let ty = self.convert_unnamed_at(loc, mode, decl_root);
        self.at_root = saved;
        ty
    }

    fn convert_unnamed_at(&mut self, loc: &Loc, mode: Mode, decl_root: bool) -> Type {
        let pointer = format!("#{}", loc.pointer);
        if loc.doc == *self.set.root_url() && !self.overriding.contains(loc) {
            if let Some(entries) = self.opts.type_overrides.get(&pointer) {
                let entries = entries.clone();
                self.overriding.insert(loc.clone());
                let schema = self.convert_unnamed_at(loc, mode, decl_root);
                self.overriding.remove(loc);
                return mapped(entries, schema, loc.clone());
            }
        }
        let value = match self.set.get(loc) {
            Ok(v) => v,
            Err(e) => {
                self.warn(WarningKind::UnresolvedRef, loc, e.to_string());
                return Type::Unknown;
            }
        };
        match value {
            Value::Bool(true) => Type::Unknown,
            Value::Bool(false) => Type::Never,
            Value::Object(map) => {
                let map = map.clone();
                self.convert_object_schema(loc, &map, mode, decl_root)
            }
            _ => {
                self.warn(
                    WarningKind::InvalidDefinition,
                    loc,
                    "a schema must be an object or a boolean",
                );
                Type::Unknown
            }
        }
    }

    fn convert_object_schema(
        &mut self,
        loc: &Loc,
        map: &Map<String, Value>,
        mode: Mode,
        decl_root: bool,
    ) -> Type {
        if let Some(t) = map.get("x-tsurara-type") {
            let mut rest = map.clone();
            rest.remove("x-tsurara-type");
            match parse_language_map(t) {
                Some(mut entries) => {
                    for ext in entries.values_mut() {
                        if let Some(import) = &mut ext.import {
                            import.from =
                                rebase_specifier(&self.opts.external_import_base, &import.from);
                        }
                    }
                    let schema = self.convert_object_schema(loc, &rest, mode, decl_root);
                    return Type::Mapped(Mapped {
                        entries,
                        schema: Box::new(schema),
                        loc: loc.join("x-tsurara-type"),
                    });
                }
                None => {
                    self.warn(
                        WarningKind::InvalidDefinition,
                        &loc.join("x-tsurara-type"),
                        "x-tsurara-type must map languages to types, such as `{ typescript: Uuid }` or `{ typescript: { type, import: { from, name } } }`; it was ignored",
                    );
                    return self.convert_object_schema(loc, &rest, mode, decl_root);
                }
            }
        }
        if let Some(Value::String(r)) = map.get("$ref") {
            let target = self.ref_target(loc, r);
            let ref_ty = match target {
                Some(target) => self.ref_type(&target, mode),
                None => Type::Unknown,
            };
            let assertions: Vec<&String> = map
                .keys()
                .filter(|k| {
                    *k != "$ref" && !ANNOTATIONS.contains(&k.as_str()) && !k.starts_with("x-")
                })
                .collect();
            if self.version < SpecVersion::V3_1 {
                for key in assertions {
                    self.warn(
                        WarningKind::IgnoredSibling,
                        &loc.join(key),
                        format!("`{key}` next to `$ref` is ignored in OpenAPI 3.0; wrap the reference in `allOf` to apply it"),
                    );
                }
                return ref_ty;
            }
            if assertions.is_empty() {
                return ref_ty;
            }
            let mut rest = map.clone();
            rest.remove("$ref");
            let rest_ty = self.convert_keywords(loc, &rest, mode, decl_root);
            return Type::intersection(vec![ref_ty, rest_ty]);
        }
        if let Some(Value::String(r)) = map.get("$dynamicRef") {
            let target = self.dynamic_ref_target(loc, r);
            let ref_ty = match target {
                // Dynamic anchors usually stand in for type arguments, so outside components
                // they are expanded without a name.
                Some(target)
                    if !self.is_component(&target) && self.expanding.insert(target.clone()) =>
                {
                    let ty = self.convert_unnamed(&target, mode, false);
                    self.expanding.remove(&target);
                    ty
                }
                Some(target) => self.ref_type(&target, mode),
                None => Type::Unknown,
            };
            let mut rest = map.clone();
            rest.remove("$dynamicRef");
            let has_assertions = rest
                .keys()
                .any(|k| !ANNOTATIONS.contains(&k.as_str()) && !k.starts_with("x-"));
            if !has_assertions {
                return ref_ty;
            }
            let rest_ty = self.convert_keywords(loc, &rest, mode, decl_root);
            return Type::intersection(vec![ref_ty, rest_ty]);
        }
        self.convert_keywords(loc, map, mode, decl_root)
    }

    fn ref_target(&mut self, from: &Loc, reference: &str) -> Option<Loc> {
        match self.set.resolve_ref(from, reference) {
            Ok(target) => Some(target),
            Err(e) => {
                self.warn(WarningKind::UnresolvedRef, from, e.to_string());
                None
            }
        }
    }

    /// Statically resolves `$dynamicRef` using the dynamic scope. Searches from the outermost scope for a
    /// resource with a `$dynamicAnchor` of the same name.
    fn dynamic_ref_target(&mut self, from: &Loc, reference: &str) -> Option<Loc> {
        let initial = self.ref_target(from, reference)?;
        let name = reference.rsplit_once('#').map(|(_, a)| a).unwrap_or("");
        let is_dynamic = !name.is_empty()
            && !name.starts_with('/')
            && self
                .set
                .get(&initial)
                .ok()
                .and_then(|v| v.get("$dynamicAnchor"))
                .and_then(Value::as_str)
                == Some(name);
        if !is_dynamic {
            return Some(initial);
        }
        let found = self
            .scope
            .iter()
            .find_map(|url| self.set.dynamic_anchor(url, name).cloned());
        let target = found.unwrap_or(initial);
        self.warn(
            WarningKind::DynamicRef,
            from,
            format!(
                "$dynamicRef `{reference}` was resolved statically to {}",
                target
            ),
        );
        Some(target)
    }

    /// Type of a reference target. It always goes through a named type, so cycles always pass through a name.
    fn ref_type(&mut self, target: &Loc, mode: Mode) -> Type {
        // Follows chains of bare `$ref`s before naming, but keeps aliases in components as they are.
        let target = if self.is_component(target) {
            target.clone()
        } else {
            match self.set.deref(target) {
                Ok((loc, _)) => loc,
                Err(e) => {
                    self.warn(WarningKind::UnresolvedRef, target, e.to_string());
                    return Type::Unknown;
                }
            }
        };
        if self.needs_dynamic_expansion(&target) {
            if self.expanding.insert(target.clone()) {
                let base = self.set.base_uri(&target);
                self.scope.push(base);
                let ty = self.convert_unnamed(&target, mode, false);
                self.scope.pop();
                self.expanding.remove(&target);
                return ty;
            }
            self.warn(
                WarningKind::DynamicRef,
                &target,
                "a recursive $dynamicRef could not be expanded; the default resolution is used",
            );
        }
        match self.set.get(&target) {
            Ok(Value::Bool(true)) => return Type::Unknown,
            Ok(Value::Bool(false)) => return Type::Never,
            _ => {}
        }
        self.named_ref(&target, mode)
    }

    fn is_component(&self, loc: &Loc) -> bool {
        let tokens: Vec<String> = loc.tokens().collect();
        tokens.len() == 3 && tokens[0] == "components" && tokens[1] == "schemas"
    }

    /// Whether the target contains `$dynamicRef` and the current scope has an anchor of that name.
    fn needs_dynamic_expansion(&mut self, target: &Loc) -> bool {
        let names = self.dynamic_ref_names(target);
        if names.is_empty() {
            return false;
        }
        let own = self.set.base_uri(target);
        self.scope.iter().any(|url| {
            *url != own
                && names
                    .iter()
                    .any(|n| self.set.dynamic_anchor(url, n).is_some())
        })
    }

    /// Anchor names of `$dynamicRef`s used in the subtree at `loc` (references are not followed).
    fn dynamic_ref_names(&mut self, loc: &Loc) -> Vec<String> {
        if let Some(v) = self.dynamic_names.get(loc) {
            return v.clone();
        }
        let mut out = Vec::new();
        if let Ok(v) = self.set.get(loc) {
            collect_dynamic_refs(v, &mut out);
        }
        out.sort();
        out.dedup();
        self.dynamic_names.insert(loc.clone(), out.clone());
        out
    }

    /// Converts the keywords other than `$ref`.
    fn convert_keywords(
        &mut self,
        loc: &Loc,
        map: &Map<String, Value>,
        mode: Mode,
        decl_root: bool,
    ) -> Type {
        let types: Vec<String> = match map.get("type") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        let nullable = map.get("nullable").and_then(Value::as_bool) == Some(true)
            || types.iter().any(|t| t == "null");
        let has_composition = ["allOf", "oneOf", "anyOf"]
            .iter()
            .any(|k| map.contains_key(*k));

        let mut parts: Vec<Type> = Vec::new();
        let mut extra_required: Vec<String> = Vec::new();

        if let Some(c) = map.get("const") {
            parts.push(literal_type(c).unwrap_or(Type::Unknown));
        } else if let Some(Value::Array(values)) = map.get("enum") {
            let has_object = values.iter().any(|v| v.is_object() || v.is_array());
            if has_object {
                if let Some(t) =
                    self.base_type(loc, map, &types, mode, has_composition, &mut extra_required)
                {
                    parts.push(t);
                }
            } else {
                let names = self.enum_names(loc, map, values);
                parts.push(enum_type(values, names));
            }
        } else if let Some(t) =
            self.base_type(loc, map, &types, mode, has_composition, &mut extra_required)
        {
            parts.push(t);
        }

        if let Some(Value::Array(members)) = map.get("allOf") {
            for (i, member) in members.iter().enumerate() {
                let mloc = loc.join("allOf").join(&i.to_string());
                if let Some(Value::Array(req)) = member.get("required") {
                    let own_props = member.get("properties");
                    for r in req.iter().filter_map(Value::as_str) {
                        if own_props.and_then(|p| p.get(r)).is_none() {
                            extra_required.push(r.to_owned());
                        }
                    }
                }
                let ty = if decl_root {
                    self.allof_member_for_decl(&mloc, mode)
                } else {
                    self.convert(&mloc, mode)
                };
                parts.push(ty);
            }
            if decl_root {
                if let Some(tag) = self.inherited_discriminator(loc, members) {
                    parts.push(tag);
                }
            }
        }
        for key in ["oneOf", "anyOf"] {
            if let Some(Value::Array(members)) = map.get(key) {
                let union = self.convert_union(loc, key, members, map.get("discriminator"), mode);
                parts.push(union);
            }
        }
        if map.contains_key("not") && parts.is_empty() {
            self.warn(
                WarningKind::Unsupported,
                &loc.join("not"),
                "`not` cannot be represented and was dropped; no other keyword constrains the value",
            );
        } else if map.contains_key("not") {
            self.warn(
                WarningKind::Unsupported,
                &loc.join("not"),
                "`not` cannot be represented and was dropped",
            );
        }
        for key in ["if", "dependentSchemas"] {
            if map.contains_key(key) {
                self.warn(
                    WarningKind::Unsupported,
                    &loc.join(key),
                    format!("`{key}` cannot be represented and was dropped"),
                );
            }
        }

        let mut ty = if parts.is_empty() {
            Type::Unknown
        } else {
            Type::intersection(parts)
        };
        if !extra_required.is_empty() {
            ty = self.apply_extra_required(ty, &extra_required, mode);
        }
        if nullable && ty != Type::Unknown {
            ty = ty.nullable();
        }
        ty
    }

    /// Builds a type from `type` and related keywords. `None` for schemas with only composition keywords.
    fn base_type(
        &mut self,
        loc: &Loc,
        map: &Map<String, Value>,
        types: &[String],
        mode: Mode,
        has_composition: bool,
        extra_required: &mut Vec<String>,
    ) -> Option<Type> {
        let explicit: Vec<&str> = types
            .iter()
            .map(String::as_str)
            .filter(|t| *t != "null")
            .collect();
        if !explicit.is_empty() {
            let ts: Vec<Type> = explicit
                .iter()
                .map(|t| self.type_for(loc, map, t, mode, true, has_composition, extra_required))
                .collect();
            return Some(Type::union(ts));
        }
        if !types.is_empty() {
            // Only `type: null`.
            return Some(Type::Null);
        }
        let object_keys = ["properties", "additionalProperties", "patternProperties"];
        if object_keys.iter().any(|k| map.contains_key(*k)) {
            return Some(self.type_for(
                loc,
                map,
                "object",
                mode,
                false,
                has_composition,
                extra_required,
            ));
        }
        if map.contains_key("required") {
            if let Some(Value::Array(req)) = map.get("required") {
                extra_required.extend(req.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            return None;
        }
        if map.contains_key("items") || map.contains_key("prefixItems") {
            return Some(self.type_for(
                loc,
                map,
                "array",
                mode,
                false,
                has_composition,
                extra_required,
            ));
        }
        if let Some(format) = map.get("format").and_then(Value::as_str) {
            if let Some(entries) = self.opts.type_mappings.get(format) {
                let schema = if format == "binary" {
                    Type::Scalar(Scalar::new(ScalarKind::String, Some(format)))
                } else {
                    Type::Unknown
                };
                return Some(mapped(entries.clone(), schema, loc.join("format")));
            }
            if format == "binary" {
                self.warn_binary_in_json(loc);
                return Some(Type::Scalar(Scalar::new(ScalarKind::String, Some(format))));
            }
        }
        None
    }

    /// `format: binary` describes raw bytes, which a JSON value cannot hold (design.md 6.4).
    fn warn_binary_in_json(&mut self, loc: &Loc) {
        if !self.at_root {
            self.warn(
                WarningKind::Unsupported,
                &loc.join("format"),
                "`format: binary` inside a JSON value; the value is a JSON string, so the type is `string`",
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn type_for(
        &mut self,
        loc: &Loc,
        map: &Map<String, Value>,
        ty: &str,
        mode: Mode,
        explicit: bool,
        has_composition: bool,
        extra_required: &mut Vec<String>,
    ) -> Type {
        let ty = self.type_for_unmapped(
            loc,
            map,
            ty,
            mode,
            explicit,
            has_composition,
            extra_required,
        );
        match map.get("format").and_then(Value::as_str) {
            Some(format) => match self.opts.type_mappings.get(format) {
                Some(entries) => mapped(entries.clone(), ty, loc.join("format")),
                None => ty,
            },
            None => ty,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn type_for_unmapped(
        &mut self,
        loc: &Loc,
        map: &Map<String, Value>,
        ty: &str,
        mode: Mode,
        explicit: bool,
        has_composition: bool,
        extra_required: &mut Vec<String>,
    ) -> Type {
        match ty {
            "string" => {
                // Inside JSON a string is always a JSON string, whatever its media type
                // (design.md 6.4). Part values and bodies are handled by `convert_multipart` and
                // `operation.rs`.
                let format = map.get("format").and_then(Value::as_str);
                if format == Some("binary") {
                    self.warn_binary_in_json(loc);
                }
                Type::Scalar(Scalar::new(ScalarKind::String, format))
            }
            "integer" => Type::Scalar(Scalar::new(
                ScalarKind::Integer,
                map.get("format").and_then(Value::as_str),
            )),
            "number" => Type::Scalar(Scalar::new(
                ScalarKind::Number,
                map.get("format").and_then(Value::as_str),
            )),
            "boolean" => Type::Boolean,
            "null" => Type::Null,
            "array" => self.array_type(loc, map, mode),
            "object" => self.object_type(loc, map, mode, explicit, has_composition, extra_required),
            other => {
                self.warn(
                    WarningKind::InvalidDefinition,
                    &loc.join("type"),
                    format!("unknown type `{other}`"),
                );
                Type::Unknown
            }
        }
    }

    fn array_type(&mut self, loc: &Loc, map: &Map<String, Value>, mode: Mode) -> Type {
        let count = |key: &str| map.get(key).and_then(Value::as_u64).map(|n| n as usize);
        let min = count("minItems").unwrap_or(0);
        let max = count("maxItems");
        if max.is_some_and(|max| min > max) {
            return Type::Never;
        }
        let prefix = match map.get("prefixItems") {
            Some(Value::Array(p)) => Some(("prefixItems", p.len())),
            _ => match map.get("items") {
                // Draft 4 array-form items.
                Some(Value::Array(p)) => Some(("items", p.len())),
                _ => None,
            },
        };
        let Some((key, len)) = prefix else {
            return match map.get("items") {
                Some(_) => self.convert(&loc.join("items"), mode).array(),
                None => Type::Unknown.array(),
            };
        };
        let rest_key = if key == "prefixItems" {
            "items"
        } else {
            "additionalItems"
        };
        let closed = matches!(map.get(rest_key), Some(Value::Bool(false)));
        if closed && min > len {
            return Type::Never;
        }
        // Elements beyond maxItems can never be present (design.md 6.4).
        let len = max.map_or(len, |max| len.min(max));
        let items: Vec<Type> = (0..len)
            .map(|i| self.convert(&loc.join(key).join(&i.to_string()), mode))
            .collect();
        let rest = if closed || max.is_some_and(|max| max <= len) {
            None
        } else {
            match map.get(rest_key) {
                Some(Value::Bool(true)) | None => Some(Box::new(Type::Unknown)),
                Some(_) => Some(Box::new(self.convert(&loc.join(rest_key), mode))),
            }
        };
        // minItems beyond the prefix and maxItems beyond it with a rest type are approximated with
        // the rest element type; the constraints stay in JSDoc.
        Type::Tuple(Tuple {
            items,
            required: min.min(len),
            rest,
        })
    }

    fn object_type(
        &mut self,
        loc: &Loc,
        map: &Map<String, Value>,
        mode: Mode,
        explicit: bool,
        has_composition: bool,
        extra_required: &mut Vec<String>,
    ) -> Type {
        let required: Vec<String> = match map.get("required") {
            Some(Value::Array(r)) => r
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        let parts = std::mem::take(&mut self.part_props);
        let mut properties = Vec::new();
        if let Some(Value::Object(props)) = map.get("properties") {
            for name in props.keys() {
                let ploc = loc.join("properties").join(name);
                let (read_only, write_only) = self.property_visibility(&ploc);
                if self.opts.split_visibility {
                    if mode == Mode::Read && write_only {
                        continue;
                    }
                    if mode == Mode::Write && read_only {
                        continue;
                    }
                }
                let ty = match parts.then(|| self.part_binary_type(&ploc)).flatten() {
                    Some(ty) => ty,
                    None => self.convert(&ploc, mode),
                };
                let doc = self.set.get(&ploc).ok().and_then(schema_doc);
                properties.push(
                    Property::new(name.clone(), ty)
                        .optional(!required.contains(name))
                        .with_doc(doc),
                );
            }
        }
        for r in &required {
            if !properties.iter().any(|p| &p.name == r)
                && !map.get("properties").is_some_and(|p| p.get(r).is_some())
            {
                extra_required.push(r.clone());
            }
        }
        let has_props = map
            .get("properties")
            .is_some_and(|p| p.as_object().is_some_and(|m| !m.is_empty()));
        let additional = map.get("additionalProperties");
        let mut index_types: Vec<Type> = Vec::new();
        let mut closed = false;
        match additional {
            None => {}
            Some(Value::Bool(true)) => index_types.push(Type::Unknown),
            Some(Value::Bool(false)) => closed = true,
            Some(_) => index_types.push(self.convert(&loc.join("additionalProperties"), mode)),
        }
        if let Some(Value::Object(patterns)) = map.get("patternProperties") {
            for p in patterns.keys() {
                index_types.push(self.convert(&loc.join("patternProperties").join(p), mode));
            }
        }
        // The map value as the definition says. TypeScript widens it with the property types when
        // it lowers the object (design.md 6.4, 18.4.1).
        let index = if !index_types.is_empty() {
            Some(Box::new(Type::union(index_types)))
        } else if closed && !has_props {
            Some(Box::new(Type::Never))
        } else if !closed
            && !has_props
            && explicit
            && !has_composition
            && !map.contains_key("patternProperties")
        {
            Some(Box::new(Type::Unknown))
        } else {
            None
        };
        if properties.is_empty() && index.is_none() {
            return if has_composition || !explicit {
                Type::Unknown
            } else {
                Type::Object(Object::default())
            };
        }
        Type::Object(Object { properties, index })
    }

    /// readOnly and writeOnly of a property. If the property is a `$ref`, the target is checked too.
    fn property_visibility(&self, ploc: &Loc) -> (bool, bool) {
        let flag = |v: &Value, k: &str| v.get(k).and_then(Value::as_bool) == Some(true);
        let Ok(v) = self.set.get(ploc) else {
            return (false, false);
        };
        let mut ro = flag(v, "readOnly");
        let mut wo = flag(v, "writeOnly");
        if !ro && !wo && v.get("$ref").is_some() {
            if let Ok((_, target)) = self.set.deref_all(ploc) {
                ro = flag(target, "readOnly");
                wo = flag(target, "writeOnly");
            }
        }
        (ro, wo)
    }

    /// Whether the schema (including reference targets) has readOnly / writeOnly properties.
    ///
    /// References can form cycles, so the answer is the least fixed point over every schema reachable
    /// from `loc`: a schema has visibility if it has it directly or any schema it reaches has it.
    /// Results are recorded only after the fixed point is reached (design.md 6.5).
    pub fn has_visibility(&mut self, loc: &Loc) -> bool {
        if let Some(&v) = self.visibility.get(loc) {
            return v;
        }
        let mut nodes: Vec<Loc> = Vec::new();
        let mut index: HashMap<Loc, usize> = HashMap::new();
        let mut direct = Vec::new();
        let mut edges: Vec<Vec<usize>> = Vec::new();
        let mut stack = vec![loc.clone()];
        index.insert(loc.clone(), 0);
        nodes.push(loc.clone());
        while let Some(current) = stack.pop() {
            let i = index[&current];
            let (d, succs) = self.visibility_node(&current);
            if direct.len() <= i {
                direct.resize(i + 1, false);
                edges.resize(i + 1, Vec::new());
            }
            direct[i] = d;
            for succ in succs {
                let j = match index.get(&succ) {
                    Some(&j) => j,
                    None => {
                        let j = nodes.len();
                        index.insert(succ.clone(), j);
                        nodes.push(succ.clone());
                        stack.push(succ);
                        j
                    }
                };
                edges[i].push(j);
            }
        }
        direct.resize(nodes.len(), false);
        edges.resize(nodes.len(), Vec::new());
        // Schemas already decided keep their recorded result.
        let mut result: Vec<bool> = nodes
            .iter()
            .zip(&direct)
            .map(|(n, &d)| self.visibility.get(n).copied().unwrap_or(d))
            .collect();
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..nodes.len() {
                if !result[i] && edges[i].iter().any(|&j| result[j]) {
                    result[i] = true;
                    changed = true;
                }
            }
        }
        for (n, r) in nodes.into_iter().zip(result) {
            self.visibility.insert(n, r);
        }
        self.visibility[loc]
    }

    /// Whether the schema at `loc` has readOnly / writeOnly properties itself, and the schemas whose
    /// visibility it inherits. Already decided schemas have no successors to visit.
    fn visibility_node(&self, loc: &Loc) -> (bool, Vec<Loc>) {
        if self.visibility.contains_key(loc) {
            return (self.visibility[loc], Vec::new());
        }
        let Ok(Value::Object(map)) = self.set.get(loc) else {
            return (false, Vec::new());
        };
        let mut direct = false;
        let mut succs = Vec::new();
        if let Some(Value::String(r)) = map.get("$ref") {
            if let Ok(target) = self.set.resolve_ref(loc, r) {
                succs.push(target);
            }
        }
        if let Some(Value::Object(props)) = map.get("properties") {
            for name in props.keys() {
                let ploc = loc.join("properties").join(name);
                let (ro, wo) = self.property_visibility(&ploc);
                direct |= ro || wo;
                succs.push(ploc);
            }
        }
        for key in ["allOf", "oneOf", "anyOf", "prefixItems"] {
            if let Some(Value::Array(items)) = map.get(key) {
                for i in 0..items.len() {
                    succs.push(loc.join(key).join(&i.to_string()));
                }
            }
        }
        for key in ["items", "additionalProperties"] {
            if map.get(key).is_some_and(Value::is_object) {
                succs.push(loc.join(key));
            }
        }
        (direct, succs)
    }

    /// allOf members of a named type. If the parent lists this schema in `oneOf`, expands only the part
    /// without `oneOf` instead of referencing the parent (to avoid a type cycle).
    fn allof_member_for_decl(&mut self, mloc: &Loc, mode: Mode) -> Type {
        let Some(me) = self.current.last().cloned() else {
            return self.convert(mloc, mode);
        };
        let Ok(Value::Object(member)) = self.set.get(mloc) else {
            return self.convert(mloc, mode);
        };
        let Some(Value::String(r)) = member.get("$ref") else {
            return self.convert(mloc, mode);
        };
        let r = r.clone();
        let Some(parent) = self.ref_target(mloc, &r) else {
            return Type::Unknown;
        };
        let parent = self.set.deref(&parent).map(|(l, _)| l).unwrap_or(parent);
        if !self.lists_member(&parent, &me) {
            return self.convert(mloc, mode);
        }
        let Ok(Value::Object(pmap)) = self.set.get(&parent) else {
            return Type::Unknown;
        };
        let mut pmap = pmap.clone();
        for k in ["oneOf", "anyOf", "discriminator"] {
            pmap.remove(k);
        }
        self.convert_keywords(&parent, &pmap, mode, false)
    }

    /// Whether the oneOf / anyOf of `parent` references `member`.
    fn lists_member(&self, parent: &Loc, member: &Loc) -> bool {
        let Ok(Value::Object(pmap)) = self.set.get(parent) else {
            return false;
        };
        for key in ["oneOf", "anyOf"] {
            if let Some(Value::Array(items)) = pmap.get(key) {
                for (i, item) in items.iter().enumerate() {
                    if let Some(Value::String(r)) = item.get("$ref") {
                        let iloc = parent.join(key).join(&i.to_string());
                        if let Ok(t) = self.set.resolve_ref(&iloc, r) {
                            let t = self.set.deref(&t).map(|(l, _)| l).unwrap_or(t);
                            if t == *member {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Literal added to a child when the child references its parent in allOf and the parent has a discriminator.
    fn inherited_discriminator(&mut self, loc: &Loc, members: &[Value]) -> Option<Type> {
        let me = self.current.last()?.clone();
        if me != *loc {
            return None;
        }
        for (i, m) in members.iter().enumerate() {
            let Some(Value::String(r)) = m.get("$ref") else {
                continue;
            };
            let mloc = loc.join("allOf").join(&i.to_string());
            let Ok(parent) = self.set.resolve_ref(&mloc, r) else {
                continue;
            };
            let parent = self.set.deref(&parent).map(|(l, _)| l).unwrap_or(parent);
            let Ok(pv) = self.set.get(&parent) else {
                continue;
            };
            let Some(disc) = pv.get("discriminator").and_then(Value::as_object) else {
                continue;
            };
            let Some(prop) = disc.get("propertyName").and_then(Value::as_str) else {
                continue;
            };
            let disc = disc.clone();
            let prop = prop.to_owned();
            let values = self.discriminator_values(&parent, &disc, &me);
            if values.is_empty() {
                continue;
            }
            return Some(Type::Tag(Tag {
                property: prop,
                values,
            }));
        }
        None
    }

    /// The discriminator values that point to `target`. If none in `mapping` or `defaultMapping`,
    /// the schema name is used.
    fn discriminator_values(
        &self,
        disc_loc: &Loc,
        disc: &Map<String, Value>,
        target: &Loc,
    ) -> Vec<String> {
        let mut values = Vec::new();
        let mut mentioned = false;
        if let Some(Value::Object(mapping)) = disc.get("mapping") {
            for (value, r) in mapping {
                let Some(r) = r.as_str() else { continue };
                if let Some(t) = self.mapping_target(disc_loc, r) {
                    if t == *target {
                        values.push(value.clone());
                        mentioned = true;
                    }
                }
            }
        }
        if mentioned {
            return values;
        }
        if let Some(Value::String(d)) = disc.get("defaultMapping") {
            if self.mapping_target(disc_loc, d).as_ref() == Some(target) {
                return Vec::new();
            }
        }
        match target.last_token() {
            Some(name) => vec![name],
            None => Vec::new(),
        }
    }

    /// Turns a mapping value into a location. A value with neither `#` nor `/` is a schema name.
    fn mapping_target(&self, from: &Loc, value: &str) -> Option<Loc> {
        let reference = if !value.contains('#') && !value.contains('/') && !value.contains('.') {
            format!("#/components/schemas/{value}")
        } else {
            value.to_owned()
        };
        let base_from = if reference.starts_with("#/components/") {
            self.set.root_loc()
        } else {
            from.clone()
        };
        let t = self.set.resolve_ref(&base_from, &reference).ok()?;
        Some(self.set.deref(&t).map(|(l, _)| l).unwrap_or(t))
    }

    fn convert_union(
        &mut self,
        loc: &Loc,
        key: &str,
        members: &[Value],
        discriminator: Option<&Value>,
        mode: Mode,
    ) -> Type {
        let disc = discriminator
            .and_then(Value::as_object)
            .filter(|d| d.get("propertyName").and_then(Value::as_str).is_some())
            .cloned();
        let mut out = Vec::with_capacity(members.len());
        let mut values_per_member = Vec::with_capacity(members.len());
        let mut targets = Vec::new();
        for (i, member) in members.iter().enumerate() {
            let mloc = loc.join(key).join(&i.to_string());
            out.push(self.convert(&mloc, mode));
            let Some(disc) = &disc else { continue };
            let target = match member.get("$ref").and_then(Value::as_str) {
                Some(r) => self
                    .set
                    .resolve_ref(&mloc, r)
                    .ok()
                    .map(|t| self.set.deref(&t).map(|(l, _)| l).unwrap_or(t)),
                None => None,
            };
            let values = match target {
                Some(target) => {
                    targets.push(target.clone());
                    self.discriminator_values(loc, disc, &target)
                }
                // An inline member has no name: the values its tag property allows select it.
                None => {
                    let property = disc["propertyName"].as_str().expect("checked");
                    member
                        .get("properties")
                        .and_then(|p| p.get(property))
                        .map(tag_values)
                        .unwrap_or_default()
                }
            };
            values_per_member.push(values);
        }
        let discriminator = disc.map(|disc| {
            let default = match disc.get("defaultMapping") {
                Some(Value::String(d)) => self
                    .mapping_target(loc, d)
                    .filter(|t| !targets.contains(t))
                    .map(|t| Box::new(self.ref_type(&t, mode))),
                _ => None,
            };
            Discriminator {
                property: disc["propertyName"].as_str().expect("checked").to_owned(),
                values: values_per_member,
                default,
            }
        });
        Type::Choice(Choice {
            members: out,
            exclusive: key == "oneOf",
            discriminator,
        })
    }

    /// Applies properties made required only by allOf members or by `required` alone.
    fn apply_extra_required(&mut self, ty: Type, names: &[String], mode: Mode) -> Type {
        let mut members = match ty {
            Type::Intersection(m) => m,
            other => vec![other],
        };
        let refs: Vec<NameId> = members
            .iter()
            .filter_map(|m| match m {
                Type::Named(n) => Some(n.id),
                _ => None,
            })
            .collect();
        let obj_idx = members.iter().position(|m| matches!(m, Type::Object(_)));
        let mut added = Vec::new();
        for name in names {
            // An inherited discriminator property is already required by its tag.
            if members
                .iter()
                .any(|m| matches!(m, Type::Tag(t) if &t.property == name))
            {
                continue;
            }
            if let Some(i) = obj_idx {
                if let Type::Object(o) = &mut members[i] {
                    if let Some(p) = o.properties.iter_mut().find(|p| &p.name == name) {
                        p.optional = false;
                        continue;
                    }
                }
            }
            // If it is a property of a named type, looks up that type and adds the property as required.
            let found = refs.iter().find_map(|r| {
                let loc = self.loc_of_name(*r)?;
                find_property(self.set, &loc, name, 0)
            });
            if let Some(ploc) = found {
                let ty = self.convert(&ploc, mode);
                added.push(Property::new(name.clone(), ty));
            }
        }
        if !added.is_empty() {
            members.push(Type::Object(Object {
                properties: added,
                index: None,
            }));
        }
        Type::intersection(members)
    }

    fn loc_of_name(&self, id: NameId) -> Option<Loc> {
        self.named
            .iter()
            .find_map(|(loc, n)| (n.id == id || n.writable == Some(id)).then(|| loc.clone()))
    }

    /// Locations whose named types are treated as already declared (in document order).
    pub fn named_locs(&self) -> Vec<Loc> {
        self.named.keys().cloned().collect()
    }
}

/// Locations that define the property `name` in the schema at `loc` (following allOf and `$ref`).
fn find_property(set: &DocumentSet, loc: &Loc, name: &str, depth: usize) -> Option<Loc> {
    if depth > 16 {
        return None;
    }
    let (loc, value) = set.deref_all(loc).ok()?;
    if value.get("properties").and_then(|p| p.get(name)).is_some() {
        return Some(loc.join("properties").join(name));
    }
    if let Some(Value::Array(members)) = value.get("allOf") {
        for i in 0..members.len() {
            if let Some(found) = find_property(
                set,
                &loc.join("allOf").join(&i.to_string()),
                name,
                depth + 1,
            ) {
                return Some(found);
            }
        }
    }
    None
}

fn collect_dynamic_refs(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("$dynamicRef") {
                if let Some((_, name)) = r.rsplit_once('#') {
                    if !name.is_empty() && !name.starts_with('/') {
                        out.push(name.to_owned());
                    }
                }
            }
            for (k, v) in map {
                if matches!(
                    k.as_str(),
                    "example" | "examples" | "default" | "const" | "enum"
                ) {
                    continue;
                }
                collect_dynamic_refs(v, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_dynamic_refs(v, out)),
        _ => {}
    }
}

/// Whether a `type: string` schema describes raw bytes when it is not inside JSON.
fn is_binary_string(map: &Map<String, Value>) -> bool {
    let format = map.get("format").and_then(Value::as_str);
    let encoding = map.get("contentEncoding").and_then(Value::as_str);
    format == Some("binary")
        || (map.contains_key("contentMediaType")
            && !matches!(encoding, Some("base64" | "base64url")))
}

fn binary_type() -> Type {
    Type::Binary
}

/// An `enum` whose values are all literals or `null`, with the member names for the values, if
/// there are as many.
fn enum_type(values: &[Value], names: Option<Vec<String>>) -> Type {
    let mut e = Enum {
        values: Vec::new(),
        nullable: false,
        names: Vec::new(),
    };
    for (i, v) in values.iter().enumerate() {
        let lit = match v {
            Value::String(s) => Literal::String(s.clone()),
            Value::Number(n) => Literal::Number(n.clone()),
            Value::Bool(b) => Literal::Boolean(*b),
            _ => {
                e.nullable = true;
                continue;
            }
        };
        if !e.values.contains(&lit) {
            e.values.push(lit);
            if let Some(names) = &names {
                e.names.push(names[i].clone());
            }
        }
    }
    Type::Enum(e)
}

fn literal_type(v: &Value) -> Option<Type> {
    match v {
        Value::String(s) => Some(Type::Literal(Literal::String(s.clone()))),
        Value::Number(n) => Some(Type::Literal(Literal::Number(n.clone()))),
        Value::Bool(b) => Some(Type::Literal(Literal::Boolean(*b))),
        Value::Null => Some(Type::Null),
        _ => None,
    }
}

/// The part of a candidate's preferred name that comes from the path below its owner.
fn path_name(tokens: &[String]) -> String {
    use tsurara_naming::pascal_case;
    let mut out = String::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        let next = tokens.get(i + 1).map(String::as_str);
        match t {
            "properties" | "headers" => {
                if let Some(n) = next {
                    out.push_str(&pascal_case(n));
                }
                i += 2;
                continue;
            }
            "items" | "itemSchema" => out.push_str("Item"),
            "additionalProperties" => out.push_str("Value"),
            "patternProperties" => {
                out.push_str("Value");
                i += 2;
                continue;
            }
            "prefixItems" | "oneOf" | "anyOf" => {
                if t == "prefixItems" {
                    out.push_str("Item");
                }
                if let Some(n) = next.and_then(|n| n.parse::<usize>().ok()) {
                    out.push_str(&(n + 1).to_string());
                }
                i += 2;
                continue;
            }
            "requestBody" => out.push_str("Body"),
            "responses" => {
                if let Some(n) = next {
                    out.push_str(&pascal_case(n));
                }
                i += 2;
                continue;
            }
            "parameters" => {
                if let Some(n) = next {
                    out.push_str(&pascal_case(n));
                }
                i += 2;
                continue;
            }
            "content" => {
                i += 2;
                continue;
            }
            "schema" => {}
            other if other.parse::<usize>().is_ok() => out.push_str(other),
            other => out.push_str(&pascal_case(other)),
        }
        i += 1;
    }
    out
}

/// A mapped type from the configuration's entries per language.
fn mapped(entries: std::collections::BTreeMap<String, External>, schema: Type, loc: Loc) -> Type {
    Type::Mapped(Mapped {
        entries,
        schema: Box::new(schema),
        loc,
    })
}

/// Value of `x-tsurara-type`: a map from language to a type. The flat form (a type without a language)
/// is not accepted.
fn parse_language_map(v: &Value) -> Option<std::collections::BTreeMap<String, External>> {
    let Value::Object(m) = v else { return None };
    if m.is_empty() || m.contains_key("type") {
        return None;
    }
    m.iter()
        .map(|(lang, t)| Some((lang.clone(), parse_external(t)?)))
        .collect()
}

/// One language's entry of `x-tsurara-type`. A string or `{ type, import: { from, name } }`.
pub fn parse_external(v: &Value) -> Option<External> {
    match v {
        Value::String(s) => Some(External {
            code: s.clone(),
            import: None,
        }),
        Value::Object(m) => {
            let code = m.get("type")?.as_str()?.to_owned();
            let import = match m.get("import") {
                Some(Value::Object(i)) => Some(Import {
                    from: i.get("from")?.as_str()?.to_owned(),
                    name: i
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&code)
                        .to_owned(),
                }),
                _ => None,
            };
            Some(External { code, import })
        }
        _ => None,
    }
}

/// Rewrites a relative specifier written against `base` (relative to the output root) so that it is
/// relative to the output root. Non-relative specifiers are returned as is.
fn rebase_specifier(base: &str, from: &str) -> String {
    if base.is_empty() || !from.starts_with('.') {
        return from.to_owned();
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in base.split('/').chain(from.split('/')) {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|p| *p != "..") => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if joined.starts_with("../") {
        joined
    } else {
        format!("./{joined}")
    }
}

/// JSDoc of a schema. Emits description, deprecated, default, example, format, and the various constraints.
pub fn schema_doc(value: &Value) -> Option<Doc> {
    let map = value.as_object()?;
    let mut doc = Doc::default();
    let title = map.get("title").and_then(Value::as_str);
    let description = map.get("description").and_then(Value::as_str);
    doc.description = match (title, description) {
        (Some(t), Some(d)) if t != d => Some(format!("{t}\n\n{d}")),
        (_, Some(d)) => Some(d.to_owned()),
        (Some(t), None) => Some(t.to_owned()),
        (None, None) => None,
    };
    if map.get("deprecated").and_then(Value::as_bool) == Some(true) {
        doc.tag("deprecated", "");
    }
    if let Some(f) = map.get("format").and_then(Value::as_str) {
        doc.tag("format", f);
    }
    for key in [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "minLength",
        "maxLength",
        "pattern",
        "minItems",
        "maxItems",
        "uniqueItems",
        "minProperties",
        "maxProperties",
        "contentMediaType",
        "contentEncoding",
    ] {
        if let Some(v) = map.get(key) {
            doc.tag(key, v.clone());
        }
    }
    if let Some(v) = map.get("default") {
        doc.tag("default", v.clone());
    }
    if let Some(v) = map.get("example") {
        doc.tag("example", v.clone());
    } else if let Some(Value::Array(examples)) = map.get("examples") {
        if let Some(first) = examples.first() {
            doc.tag("example", first.clone());
        }
    }
    doc.into_option()
}

/// Whether `loc` is the schema of a multipart media type of a request body
/// (`…/requestBody/content/multipart~1…/schema` or `/components/requestBodies/{name}/content/…`).
fn is_multipart_body_schema(loc: &Loc) -> bool {
    let tokens: Vec<String> = loc.tokens().collect();
    let n = tokens.len();
    n >= 4
        && tokens[n - 1] == "schema"
        && tokens[n - 2].to_ascii_lowercase().starts_with("multipart/")
        && tokens[n - 3] == "content"
        && (tokens[n - 4] == "requestBody" || (n >= 5 && tokens[n - 5] == "requestBodies"))
}

/// The string values a tag property schema allows: its `const`, or its `enum`.
fn tag_values(schema: &Value) -> Vec<String> {
    let strings = |values: &[Value]| -> Option<Vec<String>> {
        values
            .iter()
            .map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    match (schema.get("const"), schema.get("enum")) {
        (Some(c), _) => strings(std::slice::from_ref(c)).unwrap_or_default(),
        (None, Some(Value::Array(e))) => strings(e).unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::rebase_specifier;

    #[test]
    fn rebase_specifier_joins_relative_paths() {
        assert_eq!(rebase_specifier("", "./brand.js"), "./brand.js");
        assert_eq!(rebase_specifier("../..", "./brand.js"), "../../brand.js");
        assert_eq!(
            rebase_specifier("../..", "../lib/brand.js"),
            "../../../lib/brand.js"
        );
        assert_eq!(rebase_specifier("types", "./brand.js"), "./types/brand.js");
        assert_eq!(rebase_specifier("types", "../brand.js"), "./brand.js");
        assert_eq!(rebase_specifier("../..", "@acme/brand"), "@acme/brand");
    }
}

//! Intermediate representation (IR) close to TypeScript types.
//!
//! Conditional, mapped, and indexed access types are deliberately not representable. Type computation
//! is done in the generator, not left to the TypeScript type system (design.md 6.7).

use serde_json::Number;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Type {
    Unknown,
    Never,
    Null,
    Undefined,
    String,
    Number,
    Boolean,
    /// An integer, number or string from a schema, with its `format`. Backends lower it to their
    /// own types; TypeScript prints `number` or `string` (design.md 18.4.1).
    Scalar(Scalar),
    /// A schema mapped to an external type per language (`x-tsurara-type`, `typeMappings`,
    /// `overrides`). A backend without an entry uses `schema` (design.md 18.4.4).
    Mapped(Mapped),
    /// An inline object, enum or union, with what a backend needs to name it if it hoists it
    /// (design.md 18.4.2). TypeScript writes it inline.
    Inline(Box<Inline>),
    /// A raw byte value: a multipart part value or an HTTP body that is not JSON (design.md 6.4).
    /// TypeScript lowers it to `Blob | File`.
    Binary,
    /// `enum` with literal values. TypeScript lowers it to a union of literals (design.md 18.4.1).
    Enum(Enum),
    Literal(Literal),
    /// Reference to a named type by its final identifier. Only backends produce it, when they resolve
    /// `Named`.
    Ref(String),
    /// Reference to a declaration by its name request (design.md 18.4.2).
    Named(NameRef),
    Array(Box<Type>),
    Tuple(Tuple),
    Union(Vec<Type>),
    Intersection(Vec<Type>),
    /// `oneOf` or `anyOf` as written: members are not flattened or deduplicated, because that is not
    /// exact for `oneOf` (design.md 18.4.1). TypeScript lowers it to a union.
    Choice(Choice),
    /// The values a discriminator property takes in a schema that inherits the discriminator from its
    /// parent through `allOf`. TypeScript lowers it to `{ property: 'value' }`.
    Tag(Tag),
    Object(Object),
    /// A type defined outside the generator. Used by `x-tsurara-type` and `typeMappings`.
    External(External),
    /// A type from `@tsurara/client` or a TypeScript built-in (`Blob`, `ReadableStream<Uint8Array>`, ...).
    /// The name is a fixed string and takes no type arguments.
    #[serde(skip)]
    Builtin(std::borrow::Cow<'static, str>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Inline {
    /// Derived from the owner's preferred name and the path to the schema (`PetOwner`).
    pub preferred_name: String,
    /// Where the schema is written. Its JSON Pointer is the key of `naming.overrides`.
    pub loc: tsurara_loader::Loc,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct NameRef {
    pub id: crate::naming::NameId,
    /// The preferred name, used only to order members deterministically inside the model.
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Mapped {
    /// External type per language (`typescript`, `rust`, ...).
    pub entries: std::collections::BTreeMap<String, External>,
    /// The type the schema has without the mapping.
    pub schema: Box<Type>,
    /// Where the mapping is written, for warnings.
    pub loc: tsurara_loader::Loc,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub members: Vec<Type>,
    /// `oneOf` is exclusive, `anyOf` is not.
    pub exclusive: bool,
    pub discriminator: Option<Discriminator>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Discriminator {
    pub property: String,
    /// The values that select each member, in member order. Empty for a member that is not a
    /// reference or has no value.
    pub values: Vec<Vec<String>>,
    /// The `defaultMapping` target (3.2), when it is not one of the members.
    pub default: Option<Box<Type>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub property: String,
    pub values: Vec<String>,
}

impl Tag {
    /// `{ property: 'a' | 'b' }`.
    pub fn object(&self) -> Type {
        tag_object(&self.property, &self.values)
    }
}

fn tag_object(property: &str, values: &[String]) -> Type {
    Type::Object(Object {
        properties: vec![Property::new(
            property,
            Type::union(values.iter().cloned().map(Type::string_literal).collect()),
        )],
        index: None,
    })
}

impl Choice {
    /// The TypeScript union: `({ petType: 'cat' } & Cat) | ...`, followed by the default member.
    pub fn union(&self) -> Type {
        let mut out = Vec::with_capacity(self.members.len() + 1);
        for (i, m) in self.members.iter().enumerate() {
            match &self.discriminator {
                Some(d) if d.values.get(i).is_some_and(|v| !v.is_empty()) => {
                    out.push(Type::intersection(vec![
                        tag_object(&d.property, &d.values[i]),
                        m.clone(),
                    ]))
                }
                _ => out.push(m.clone()),
            }
        }
        if let Some(def) = self
            .discriminator
            .as_ref()
            .and_then(|d| d.default.as_deref())
        {
            out.push(def.clone());
        }
        Type::union(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Enum {
    /// The values in definition order, without duplicates and without `null`.
    pub values: Vec<Literal>,
    /// Whether `null` is one of the values.
    pub nullable: bool,
    /// Member names for the values, in the same order (`x-enum-varnames` or `x-enumNames`), or
    /// empty. Backends turn them into identifiers instead of the values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
}

impl Enum {
    /// The union of literal types TypeScript uses for the enum.
    pub fn literal_union(&self) -> Type {
        let mut members: Vec<Type> = self.values.iter().cloned().map(Type::Literal).collect();
        if self.nullable {
            members.push(Type::Null);
        }
        Type::union(members)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Scalar {
    pub kind: ScalarKind,
    /// `int64`, `double`, `date-time`, `uuid`, ...
    pub format: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum ScalarKind {
    Integer,
    Number,
    String,
}

impl Scalar {
    pub fn new(kind: ScalarKind, format: Option<&str>) -> Self {
        Self {
            kind,
            format: format.map(str::to_owned),
        }
    }

    /// The TypeScript type: `number` or `string`.
    pub fn base(&self) -> Type {
        match self.kind {
            ScalarKind::Integer | ScalarKind::Number => Type::Number,
            ScalarKind::String => Type::String,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Literal {
    String(String),
    Number(Number),
    Boolean(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Tuple {
    pub items: Vec<Type>,
    /// Number of leading elements that are required. The rest of `items` are optional (`T?`).
    pub required: usize,
    /// Type of the rest elements. `None` makes the length fixed.
    pub rest: Option<Box<Type>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Object {
    pub properties: Vec<Property>,
    /// Value type of the index signature `[key: string]: T`.
    pub index: Option<Box<Type>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Property {
    pub name: String,
    pub ty: Type,
    pub optional: bool,
    pub readonly: bool,
    pub doc: Option<Doc>,
}

impl Property {
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
        Self {
            name: name.into(),
            ty,
            optional: false,
            readonly: false,
            doc: None,
        }
    }

    pub fn optional(mut self, optional: bool) -> Self {
        self.optional = optional;
        self
    }

    pub fn with_doc(mut self, doc: Option<Doc>) -> Self {
        self.doc = doc;
        self
    }
}

/// An external type. With `import`, files that use the type get an `import type`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct External {
    pub code: String,
    pub import: Option<Import>,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Import {
    /// Module specifier. If it starts with `.`, it is relative to the output root.
    pub from: String,
    pub name: String,
}

/// Documentation of a declaration or property: a description and tags such as `deprecated`,
/// `default`, `example`, `format` and constraints (design.md 18.4.5).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    pub description: Option<String>,
    /// Tags with their values as written in the definition (`""` for tags without a value, such as
    /// `deprecated`). Printers render them with `Doc::tag_text`.
    pub tags: Vec<(String, serde_json::Value)>,
}

/// The validation keywords of a schema (`Doc::constraints`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Constraints {
    pub minimum: Option<serde_json::Number>,
    pub maximum: Option<serde_json::Number>,
    pub exclusive_minimum: Option<serde_json::Number>,
    pub exclusive_maximum: Option<serde_json::Number>,
    pub multiple_of: Option<serde_json::Number>,
    pub min_length: Option<u64>,
    pub max_length: Option<u64>,
    pub pattern: Option<String>,
    pub min_items: Option<u64>,
    pub max_items: Option<u64>,
    pub unique_items: bool,
    pub min_properties: Option<u64>,
    pub max_properties: Option<u64>,
}

impl std::hash::Hash for Doc {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.description.hash(state);
        for (name, value) in &self.tags {
            name.hash(state);
            value.to_string().hash(state);
        }
    }
}

impl Doc {
    pub fn is_empty(&self) -> bool {
        self.description
            .as_deref()
            .is_none_or(|d| d.trim().is_empty())
            && self.tags.is_empty()
    }

    pub fn tag(&mut self, name: &str, value: impl Into<serde_json::Value>) {
        self.tags.push((name.to_owned(), value.into()));
    }

    /// A tag's value as text: `default` and `example` are JSON (strings are quoted, as in the
    /// definition); other strings are shown as they are.
    pub fn tag_text(name: &str, value: &serde_json::Value) -> String {
        match value {
            serde_json::Value::String(s) if !matches!(name, "default" | "example") => s.clone(),
            other => other.to_string(),
        }
    }

    pub fn into_option(self) -> Option<Self> {
        if self.is_empty() { None } else { Some(self) }
    }

    /// The validation keywords among the tags, typed. The tags keep them as written for
    /// JSDoc; this is the view validators print from. OpenAPI 3.0's boolean `exclusiveMinimum` and
    /// `exclusiveMaximum` are folded into the bounds, so `exclusive_minimum` is always a number.
    pub fn constraints(&self) -> Constraints {
        let get = |name: &str| self.tags.iter().find(|(n, _)| n == name).map(|(_, v)| v);
        let number = |name: &str| get(name).and_then(|v| v.as_number()).cloned();
        let count = |name: &str| get(name).and_then(serde_json::Value::as_u64);
        let mut c = Constraints {
            minimum: number("minimum"),
            maximum: number("maximum"),
            exclusive_minimum: number("exclusiveMinimum"),
            exclusive_maximum: number("exclusiveMaximum"),
            multiple_of: number("multipleOf"),
            min_length: count("minLength"),
            max_length: count("maxLength"),
            pattern: get("pattern").and_then(|v| v.as_str()).map(str::to_owned),
            min_items: count("minItems"),
            max_items: count("maxItems"),
            unique_items: get("uniqueItems").and_then(serde_json::Value::as_bool) == Some(true),
            min_properties: count("minProperties"),
            max_properties: count("maxProperties"),
        };
        if get("exclusiveMinimum").and_then(serde_json::Value::as_bool) == Some(true) {
            c.exclusive_minimum = c.minimum.take();
        }
        if get("exclusiveMaximum").and_then(serde_json::Value::as_bool) == Some(true) {
            c.exclusive_maximum = c.maximum.take();
        }
        c
    }

    /// Merges two JSDocs. The description of `self` wins, and tags are added without duplicates.
    pub fn merge(mut self, other: Doc) -> Doc {
        if self.description.is_none() {
            self.description = other.description;
        }
        for t in other.tags {
            if !self.tags.contains(&t) {
                self.tags.push(t);
            }
        }
        self
    }
}

impl Type {
    /// A type from `@tsurara/client` or a TypeScript built-in. Only backends produce it.
    pub fn builtin(name: &'static str) -> Self {
        Type::Builtin(std::borrow::Cow::Borrowed(name))
    }

    pub fn string_literal(s: impl Into<String>) -> Self {
        Type::Literal(Literal::String(s.into()))
    }

    pub fn union(types: Vec<Type>) -> Self {
        crate::normalize::union(types)
    }

    pub fn intersection(types: Vec<Type>) -> Self {
        crate::normalize::intersection(types)
    }

    pub fn nullable(self) -> Self {
        Type::union(vec![self, Type::Null])
    }

    pub fn array(self) -> Self {
        Type::Array(Box::new(self))
    }

    /// Collects the named types referenced in the type.
    pub fn collect_refs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Type::Ref(name) => out.push(name),
            Type::Array(t) => t.collect_refs(out),
            Type::Tuple(t) => {
                t.items.iter().for_each(|i| i.collect_refs(out));
                if let Some(r) = &t.rest {
                    r.collect_refs(out);
                }
            }
            Type::Union(ts) | Type::Intersection(ts) => ts.iter().for_each(|t| t.collect_refs(out)),
            Type::Inline(i) => i.ty.collect_refs(out),
            Type::Choice(c) => {
                c.members.iter().for_each(|t| t.collect_refs(out));
                if let Some(d) = c.discriminator.as_ref().and_then(|d| d.default.as_deref()) {
                    d.collect_refs(out);
                }
            }
            Type::Object(o) => {
                o.properties.iter().for_each(|p| p.ty.collect_refs(out));
                if let Some(i) = &o.index {
                    i.collect_refs(out);
                }
            }
            _ => {}
        }
    }

    /// Collects the external imports used in the type.
    pub fn collect_imports<'a>(&'a self, out: &mut Vec<&'a Import>) {
        match self {
            Type::External(e) => {
                if let Some(i) = &e.import {
                    out.push(i);
                }
            }
            Type::Array(t) => t.collect_imports(out),
            Type::Tuple(t) => {
                t.items.iter().for_each(|i| i.collect_imports(out));
                if let Some(r) = &t.rest {
                    r.collect_imports(out);
                }
            }
            Type::Union(ts) | Type::Intersection(ts) => {
                ts.iter().for_each(|t| t.collect_imports(out))
            }
            Type::Inline(i) => i.ty.collect_imports(out),
            Type::Choice(c) => {
                c.members.iter().for_each(|t| t.collect_imports(out));
                if let Some(d) = c.discriminator.as_ref().and_then(|d| d.default.as_deref()) {
                    d.collect_imports(out);
                }
            }
            Type::Object(o) => {
                o.properties.iter().for_each(|p| p.ty.collect_imports(out));
                if let Some(i) = &o.index {
                    i.collect_imports(out);
                }
            }
            _ => {}
        }
    }

    /// Rewrites every type in the type, children first. Used by the IR stage of plugins.
    pub fn map(self, f: &mut impl FnMut(Type) -> Type) -> Type {
        let t = match self {
            Type::Array(t) => Type::Array(Box::new(t.map(f))),
            Type::Tuple(t) => Type::Tuple(Tuple {
                items: t.items.into_iter().map(|i| i.map(f)).collect(),
                required: t.required,
                rest: t.rest.map(|r| Box::new(r.map(f))),
            }),
            Type::Union(ts) => Type::Union(ts.into_iter().map(|t| t.map(f)).collect()),
            Type::Intersection(ts) => {
                Type::Intersection(ts.into_iter().map(|t| t.map(f)).collect())
            }
            Type::Inline(i) => Type::Inline(Box::new(Inline {
                ty: i.ty.map(f),
                ..*i
            })),
            Type::Mapped(m) => Type::Mapped(Mapped {
                schema: Box::new(m.schema.map(f)),
                ..m
            }),
            Type::Choice(c) => Type::Choice(Choice {
                members: c.members.into_iter().map(|t| t.map(f)).collect(),
                exclusive: c.exclusive,
                discriminator: c.discriminator.map(|d| Discriminator {
                    default: d.default.map(|t| Box::new(t.map(f))),
                    ..d
                }),
            }),
            Type::Object(o) => Type::Object(Object {
                properties: o
                    .properties
                    .into_iter()
                    .map(|p| Property {
                        ty: p.ty.map(f),
                        ..p
                    })
                    .collect(),
                index: o.index.map(|i| Box::new(i.map(f))),
            }),
            other => other,
        };
        f(t)
    }
}

//! The lowered type IR (design.md 18.5.1): declarations and type expressions that can express every
//! construct some language has. Which constructs remain after lowering depends on the capabilities.

use std::collections::BTreeMap;
use tsurara_loader::Loc;
use tsurara_sem::ir::{Doc, External, Literal, Scalar};
use tsurara_sem::naming::NameId;

/// The lowered declarations of a model.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Module {
    pub decls: Vec<Decl>,
    /// The types of each operation of the model (`Model::operations`, in order).
    pub operations: Vec<OperationTypes>,
}

/// What an operation's call function takes and returns, lowered like declarations; inline schemas
/// in them are hoisted as candidates. Each list follows its model counterpart.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct OperationTypes {
    /// Per parameter (`Operation::params`). Sent.
    pub params: Vec<Expr>,
    /// Per request media type (`Body::contents`): `Bytes` for binary bodies. Sent.
    pub body: Vec<Expr>,
    /// Per response (`Operation::responses`) and media type: the value its body is read as (a string
    /// for text, `Bytes` for binary). Received.
    pub responses: Vec<Vec<Expr>>,
    /// Per response and media type: whether the body is read item by item, `responses` then
    /// holding the type of one item (an event, or a JSON value) instead of the body's.
    pub sequences: Vec<Vec<Option<Sequence>>>,
}

/// A response body read item by item (design.md 8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Sequence {
    /// Server-Sent Events: each item is an event object with `event`, `data`, `id` and `retry`.
    Events,
    /// One JSON value per line.
    JsonLines,
    /// One JSON value per record starting with U+001E.
    JsonSeq,
}

impl Sequence {
    /// The `parseAs` of the OperationDef.
    pub fn parse_as(self) -> &'static str {
        match self {
            Sequence::Events => "events",
            Sequence::JsonLines => "jsonLines",
            Sequence::JsonSeq => "jsonSeq",
        }
    }
}

/// Index of a declaration in `Module::decls`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct DeclId(pub usize);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Decl {
    /// The name the definition asks for; the backend's naming policy turns it into an identifier.
    pub preferred_name: String,
    /// Where the name comes from: a schema's name request, or a hoisted inline schema.
    pub origin: Origin,
    pub loc: Loc,
    pub kind: DeclKind,
    pub doc: Option<Doc>,
    /// Whether values of this declaration are sent, received, or both (design.md 18.5.3, pass 5).
    pub sites: Sites,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// A declaration of the model; its name is claimed with this request.
    Schema(NameId),
    /// An inline schema hoisted into a declaration (design.md 18.4.2).
    Candidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Sites {
    pub sent: bool,
    pub received: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum DeclKind {
    Record(Record),
    Enum(EnumDecl),
    TaggedUnion(TaggedUnion),
    /// Members decoded by trying them in order (with a runtime helper where the language needs one).
    UntaggedUnion(UntaggedUnion),
    Alias(Expr),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub fields: Vec<Field>,
    /// A base declaration, when the language has inheritance (pass 2).
    pub base: Option<DeclId>,
    /// Values of properties the record does not list (`additionalProperties`), as an extra map that
    /// is flattened when serialized (pass 11).
    pub additional: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub wire_name: String,
    pub ty: Expr,
    pub presence: Presence,
    /// The field needs indirection (`Box`), because the record contains itself through it (pass 10).
    pub boxed: bool,
    pub doc: Option<Doc>,
}

/// Whether a field may be missing and whether it may be `null` (pass 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Presence {
    Required,
    Optional,
    Nullable,
    /// Missing and `null` are different values; the language needs a wrapper to tell them apart (Q3).
    OptionalNullable,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct EnumDecl {
    pub values: Vec<Literal>,
    /// Where values are received, unlisted values are kept in a fallback (pass 5). Where they are
    /// sent, the enum is closed; references say which (`Expr::Named::site`).
    pub open: bool,
    pub repr: EnumRepr,
    /// Member names for the values, in the same order (`x-enum-varnames`), or empty: then members
    /// are named after their values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
}

impl EnumDecl {
    /// The name of the member for value `i`, if the definition gives one.
    pub fn name(&self, i: usize) -> Option<&str> {
        self.names.get(i).map(String::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum EnumRepr {
    /// A native enum of the language.
    Native,
    /// Named constants of the base type, for languages without such enums.
    Constants,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct TaggedUnion {
    pub property: String,
    pub variants: Vec<Variant>,
    /// The `defaultMapping` member.
    pub default: Option<Box<Expr>>,
    /// Unlisted discriminator values are kept in a fallback where values are received (pass 6).
    pub open: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub values: Vec<String>,
    pub ty: Expr,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct UntaggedUnion {
    pub members: Vec<Expr>,
    /// The discriminator, when there is one but the language has no tagged unions.
    pub discriminator: Option<String>,
}

/// How a reference uses a declaration: values going out, coming in, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Site {
    Sent,
    Received,
    Both,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Expr {
    Any,
    Never,
    Null,
    Bool,
    Scalar(Scalar),
    Bytes,
    List(Box<Expr>),
    Map(Box<Expr>),
    Tuple {
        items: Vec<Expr>,
        required: usize,
        rest: Option<Box<Expr>>,
    },
    Named {
        decl: DeclId,
        site: Site,
    },
    /// Per language external types, and the type to use without an entry.
    External {
        entries: BTreeMap<String, External>,
        schema: Box<Expr>,
    },
    Nullable(Box<Expr>),
    InlineRecord(Record),
    Union(Vec<Expr>),
    Intersection(Vec<Expr>),
    Literal(Literal),
}

impl Module {
    pub fn decl(&self, id: DeclId) -> &Decl {
        &self.decls[id.0]
    }
}

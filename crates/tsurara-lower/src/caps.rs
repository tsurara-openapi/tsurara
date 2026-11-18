//! What a language can express (design.md 18.5.2).

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Inheritance {
    None,
    Single,
    Multiple,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Enums {
    /// String- and integer-valued enums.
    Any,
    IntegerOnly,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub inline_records: bool,
    pub structural_unions: bool,
    pub tagged_unions: bool,
    pub intersections: bool,
    pub inheritance: Inheritance,
    pub literal_types: bool,
    pub enums: Enums,
    pub tuples: bool,
    /// Whether decoding fails on an enum or discriminator value the definition does not list, so that
    /// received values need a fallback (pass 5).
    pub strict_decoding: bool,
    pub nullable_primitives: bool,
    pub recursion_by_value: bool,
}

impl Capabilities {
    /// Everything, as TypeScript has it.
    pub const ALL: Capabilities = Capabilities {
        inline_records: true,
        structural_unions: true,
        tagged_unions: true,
        intersections: true,
        inheritance: Inheritance::Multiple,
        literal_types: true,
        enums: Enums::Any,
        tuples: true,
        strict_decoding: false,
        nullable_primitives: true,
        recursion_by_value: true,
    };

    /// Very little, like Go: no inline records, unions, inheritance, tuples, literals or
    /// intersections, and decoding that fails on unknown values.
    pub const MINIMAL: Capabilities = Capabilities {
        inline_records: false,
        structural_unions: false,
        tagged_unions: false,
        intersections: false,
        inheritance: Inheritance::None,
        literal_types: false,
        enums: Enums::Any,
        tuples: false,
        strict_decoding: true,
        nullable_primitives: true,
        recursion_by_value: true,
    };
}

//! TypeScript naming (design.md 6.6, 18.6): the policy, and the replay of the model's name requests.

use crate::options::{FunctionCase, NameCase, TypeScriptOptions};
use std::collections::{BTreeMap, HashMap};
use tsurara_naming::{
    Context, Names, NamingPolicy, camel_case, function_name, function_name_from_path, pascal_case,
    type_name, type_prefix,
};
use tsurara_sem::naming::{NameId, NameKind};
use tsurara_sem::operation::MediaContent;
use tsurara_sem::{Model, Options, Warning, WarningKind};

/// Reserved words, and names with special meaning in TypeScript type positions.
const RESERVED: &[&str] = &[
    "abstract",
    "any",
    "as",
    "asserts",
    "async",
    "await",
    "bigint",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "constructor",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "global",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "module",
    "namespace",
    "never",
    "new",
    "null",
    "number",
    "object",
    "of",
    "package",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "return",
    "satisfies",
    "set",
    "static",
    "string",
    "super",
    "switch",
    "symbol",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unique",
    "unknown",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "accessor",
    "out",
    "arguments",
    "eval",
];

/// TypeScript globals referenced by generated code, and commonly used built-in names.
/// Also includes the names imported from `@tsurara/client`.
const GLOBALS: &[&str] = &[
    "Array",
    "ArrayBuffer",
    "ArrayBufferView",
    "AsyncIterable",
    "Awaited",
    "BigInt",
    "Blob",
    "Boolean",
    "Date",
    "Error",
    "Exclude",
    "Extract",
    "File",
    "FormData",
    "Function",
    "Headers",
    "Iterable",
    "JSON",
    "Map",
    "Math",
    "NonNullable",
    "Number",
    "Object",
    "Omit",
    "Partial",
    "Pick",
    "Promise",
    "ReadableStream",
    "Readonly",
    "Record",
    "RegExp",
    "Request",
    "Required",
    "Response",
    "ReturnType",
    "Set",
    "String",
    "Symbol",
    "URL",
    "URLSearchParams",
    "Uint8Array",
    "AbortSignal",
    "WeakMap",
    "WeakSet",
    "globalThis",
    // @tsurara/client
    "Client",
    "ClientOptions",
    "RequestOptions",
    "OperationDef",
    "TransportFailure",
    "ApiError",
    "Result",
    "Middleware",
    // Generated client.ts
    "BaseUrl",
    "servers",
    "operations",
    "Paths",
];

/// TypeScript's naming rules: reserved words and the globals generated code uses get the suffix
/// `Model`; schema names are kept when they are identifiers, unless PascalCase is requested.
pub struct TypeScriptPolicy {
    pub pascal_schemas: bool,
}

impl NamingPolicy for TypeScriptPolicy {
    fn convert(&self, context: Context, raw: &str, preserve: bool) -> String {
        match context {
            Context::Type => type_name(raw, !preserve || self.pascal_schemas),
            Context::Function => function_name(raw),
            _ => raw.to_owned(),
        }
    }

    fn is_reserved(&self, _: Context, ident: &str) -> bool {
        RESERVED.contains(&ident) || GLOBALS.contains(&ident)
    }

    fn reserved_suffix(&self, _: Context) -> &str {
        "Model"
    }
}

/// Names assigned to the model's requests.
#[derive(Debug, Default)]
pub struct Assigned {
    names: HashMap<NameId, String>,
    prefixes: HashMap<NameId, String>,
    /// Type names derived from operations, by template (`{P}Result`).
    type_names: HashMap<NameId, BTreeMap<String, String>>,
}

impl Assigned {
    pub fn name(&self, id: NameId) -> &str {
        &self.names[&id]
    }

    /// The type name prefix of an operation (`GetPetById`).
    pub fn prefix(&self, id: NameId) -> &str {
        &self.prefixes[&id]
    }

    /// The type names derived from an operation, by template (`{P}Result` → `GetPetResult`).
    pub fn type_names(&self, id: NameId) -> BTreeMap<String, String> {
        self.type_names.get(&id).cloned().unwrap_or_default()
    }
}

const OPERATION_TYPES: &[&str] = &[
    "{}Def",
    "{P}Data",
    "{P}Responses",
    "{P}Errors",
    "{P}Result",
    "{P}ResponseHeaders",
];
/// Claimed only by operations with typed event streams, so that other operations do not move a
/// schema named `{P}Event` aside.
const EVENT_TYPE: &str = "{P}Event";
/// Claimed only by operations with typed JSON Lines or JSON text sequences, like `EVENT_TYPE`.
const ITEM_TYPE: &str = "{P}Item";
const WEBHOOK_TYPES: &[&str] = &["{P}WebhookRequest", "{P}WebhookResponses"];
const CALLBACK_TYPES: &[&str] = &["{P}CallbackRequest", "{P}CallbackResponses"];
/// The types of an event channel: the message and the topic parameters.
const CHANNEL_TYPES: &[&str] = &["{P}Message", "{P}TopicParams"];

/// Claims the names the model requests, in order. Names that had to change are reported as warnings,
/// including derived type names that moved aside because a schema has them (design.md 6.6).
pub fn assign(model: &Model, common: &Options, ts: &TypeScriptOptions) -> (Assigned, Vec<Warning>) {
    let policy = TypeScriptPolicy {
        pascal_schemas: ts.schema_name_case == NameCase::Pascal,
    };
    let mut names = Names::new();
    let with = |has: fn(&MediaContent) -> bool| -> std::collections::HashSet<NameId> {
        model
            .operations
            .iter()
            .filter(|o| o.responses.iter().any(|r| r.contents.iter().any(has)))
            .map(|o| o.name_id)
            .collect()
    };
    let with_events = with(|c| c.events.is_some());
    let with_items = with(|c| c.items.is_some());
    let mut out = Assigned::default();
    let mut warnings = Vec::new();
    for r in &model.names {
        let pointer = format!("#{}", r.loc.pointer);
        // `typescript.naming.overrides` are final identifiers; common `naming.overrides` are preferred
        // names that still go through the policy.
        let fixed = ts.name_overrides.get(&pointer).cloned();
        let preferred = common.name_overrides.get(&pointer).cloned();
        let mut renamed = |changed: Option<String>, what: &str, name: &str| {
            if let Some(reason) = changed {
                warnings.push(Warning {
                    kind: WarningKind::Renamed,
                    loc: r.loc.clone(),
                    message: format!("{reason}; the {what} is named `{name}`"),
                    language: None,
                });
            }
        };
        let (name, operation) = match &r.kind {
            NameKind::Schema {
                preferred: raw,
                preserve,
            } => match fixed.filter(|_| r.in_root) {
                Some(name) => {
                    names.reserve("schemas", &name, &pointer);
                    (name, false)
                }
                None => {
                    let (raw, preserve) = match preferred.filter(|_| r.in_root) {
                        Some(p) => (p, true),
                        None => (raw.clone(), *preserve),
                    };
                    let ident = policy.convert(Context::Type, &raw, preserve);
                    let ident = format!("{}{ident}", ts.type_prefix);
                    let c = names.claim(&policy, Context::Type, "schemas", &ident, &pointer);
                    renamed(c.changed, "type", &c.name);
                    (c.name, false)
                }
            },
            NameKind::Writable { of } => {
                let ident = format!("{}Writable", out.name(*of));
                let c = names.claim(&policy, Context::Type, "schemas", &ident, &pointer);
                (c.name, false)
            }
            NameKind::Operation {
                operation_id,
                method,
                path,
            } => {
                let raw = fixed.or(preferred).unwrap_or_else(|| match operation_id {
                    Some(id) if ts.function_name_case == FunctionCase::Camel => {
                        function_name(&tsurara_naming::camel_case(id))
                    }
                    Some(id) => function_name(id),
                    None => function_name_from_path(method, path),
                });
                let mut templates = OPERATION_TYPES.to_vec();
                if with_events.contains(&r.id) {
                    templates.push(EVENT_TYPE);
                }
                if with_items.contains(&r.id) {
                    templates.push(ITEM_TYPE);
                }
                let (c, derived) = names.claim_group(
                    &policy,
                    Context::Function,
                    "operations",
                    &raw,
                    &templates,
                    &pointer,
                );
                renamed(c.changed, "operation", &c.name);
                let mut types = BTreeMap::new();
                for (template, d) in templates.iter().zip(derived) {
                    renamed(d.changed, "type", &d.name);
                    types.insert((*template).to_owned(), d.name);
                }
                out.type_names.insert(r.id, types);
                (c.name, true)
            }
            NameKind::Webhook { name } => {
                let raw = fixed.or(preferred).unwrap_or_else(|| {
                    let base = camel_case(name);
                    if base.is_empty() {
                        "webhook".to_owned()
                    } else {
                        function_name(&base)
                    }
                });
                let (c, derived) = names.claim_group(
                    &policy,
                    Context::Function,
                    "webhooks",
                    &raw,
                    WEBHOOK_TYPES,
                    &pointer,
                );
                renamed(c.changed, "operation", &c.name);
                let mut types = BTreeMap::new();
                for (template, d) in WEBHOOK_TYPES.iter().zip(derived) {
                    renamed(d.changed, "type", &d.name);
                    types.insert((*template).to_owned(), d.name);
                }
                out.type_names.insert(r.id, types);
                (c.name, true)
            }
            NameKind::Channel { name } => {
                let raw = fixed.or(preferred).unwrap_or_else(|| {
                    let base = camel_case(name);
                    if base.is_empty() {
                        "channel".to_owned()
                    } else {
                        function_name(&base)
                    }
                });
                let (c, derived) = names.claim_group(
                    &policy,
                    Context::Function,
                    "channels",
                    &raw,
                    CHANNEL_TYPES,
                    &pointer,
                );
                renamed(c.changed, "channel", &c.name);
                let mut types = BTreeMap::new();
                for (template, d) in CHANNEL_TYPES.iter().zip(derived) {
                    renamed(d.changed, "type", &d.name);
                    types.insert((*template).to_owned(), d.name);
                }
                out.type_names.insert(r.id, types);
                (c.name, true)
            }
            NameKind::Callback {
                parent,
                label,
                method,
            } => {
                let raw = format!(
                    "{}{}{}",
                    camel_case(out.prefix(*parent)),
                    pascal_case(label),
                    pascal_case(&method.to_lowercase())
                );
                let (c, derived) = names.claim_group(
                    &policy,
                    Context::Function,
                    "operations",
                    &raw,
                    CALLBACK_TYPES,
                    &pointer,
                );
                renamed(c.changed, "operation", &c.name);
                let mut types = BTreeMap::new();
                for (template, d) in CALLBACK_TYPES.iter().zip(derived) {
                    renamed(d.changed, "type", &d.name);
                    types.insert((*template).to_owned(), d.name);
                }
                out.type_names.insert(r.id, types);
                (c.name, true)
            }
        };
        if operation {
            out.prefixes.insert(r.id, type_prefix(&name));
        }
        out.names.insert(r.id, name);
    }
    (out, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsurara_naming::{Names, camel_case, pascal_case, type_name, words};

    const P: TypeScriptPolicy = TypeScriptPolicy {
        pascal_schemas: false,
    };

    #[test]
    fn word_splitting() {
        assert_eq!(words("getPetByID"), ["get", "Pet", "By", "ID"]);
        assert_eq!(words("HTTPServer"), ["HTTP", "Server"]);
        assert_eq!(words("pet-store_v2"), ["pet", "store", "v2"]);
        assert_eq!(pascal_case("pet-store"), "PetStore");
        assert_eq!(camel_case("Get Pet By Id"), "getPetById");
    }

    #[test]
    fn type_names() {
        assert_eq!(type_name("Pet", false), "Pet");
        assert_eq!(type_name("pet_store", false), "pet_store");
        assert_eq!(type_name("pet_store", true), "PetStore");
        assert_eq!(type_name("Foo.Bar", false), "FooBar");
        assert_eq!(type_name("1Password", false), "_1Password");
        assert_eq!(type_name("---", false), "Schema");
        assert_eq!(type_name("ペット", false), "ペット");
    }

    #[test]
    fn function_names() {
        assert_eq!(function_name("getPetById"), "getPetById");
        assert_eq!(function_name("get-pet-by-id"), "getPetById");
        assert_eq!(function_name("pets.list"), "petsList");
        assert_eq!(
            function_name_from_path("GET", "/pets/{petId}/photos"),
            "getPetsByPetIdPhotos"
        );
        assert_eq!(function_name_from_path("post", "/"), "post");
        assert_eq!(type_prefix("getPetById"), "GetPetById");
    }

    #[test]
    fn globals_and_reserved_words_get_a_suffix() {
        let mut n = Names::new();
        let c = n.claim(&P, Context::Type, "schemas", "Date", "");
        assert_eq!(c.name, "DateModel");
        assert!(c.changed.is_some());
        assert_eq!(
            n.claim(&P, Context::Type, "schemas", "Error", "").name,
            "ErrorModel"
        );
        assert_eq!(
            n.claim(&P, Context::Type, "schemas", "ErrorModel", "").name,
            "ErrorModel2"
        );
        assert_eq!(
            n.claim(&P, Context::Function, "operations", "delete", "")
                .name,
            "deleteModel"
        );
    }

    #[test]
    fn case_insensitive_collisions() {
        let mut n = Names::new();
        assert_eq!(n.claim(&P, Context::Type, "schemas", "Pet", "").name, "Pet");
        let c = n.claim(&P, Context::Type, "schemas", "pet", "");
        assert_eq!(c.name, "pet2");
        assert!(c.changed.is_some());
        // Across kinds, a difference only in case is not a collision.
        assert_eq!(
            n.claim(&P, Context::Function, "operations", "pet", "").name,
            "pet"
        );
    }

    #[test]
    fn groups() {
        let mut n = Names::new();
        n.claim(
            &P,
            Context::Type,
            "schemas",
            "GetPetData",
            "#/components/schemas/GetPetData",
        );
        // A derived name taken by a schema moves aside; the function and the other names stay.
        let (c, derived) = n.claim_group(
            &P,
            Context::Function,
            "operations",
            "getPet",
            &["{}Def", "{P}Data"],
            "#/paths/~1pet/get",
        );
        assert_eq!(c.name, "getPet");
        assert_eq!(derived[0].name, "getPetDef");
        assert_eq!(derived[0].changed, None);
        assert_eq!(derived[1].name, "GetPetOperationData");
        assert!(
            derived[1]
                .changed
                .as_deref()
                .is_some_and(|r| r.contains("#/components/schemas/GetPetData")),
            "{:?}",
            derived[1].changed
        );
        let (c, derived) = n.claim_group(
            &P,
            Context::Function,
            "operations",
            "listPets",
            &["{}Def", "{P}Data"],
            "#/paths/~1pets/get",
        );
        assert_eq!(c.name, "listPets");
        let names: Vec<&str> = derived.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["listPetsDef", "ListPetsData"]);
        // A collision of the base name still appends a number.
        let (c, derived) = n.claim_group(
            &P,
            Context::Function,
            "operations",
            "listPets",
            &["{}Def"],
            "#/paths/~1pets/post",
        );
        assert_eq!(c.name, "listPets2");
        assert_eq!(derived[0].name, "listPets2Def");
    }
}

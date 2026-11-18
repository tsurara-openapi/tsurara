//! Python naming (design.md 18.6): PascalCase classes, snake_case members and modules, `_` after
//! keywords.

use tsurara_naming::{Context, Names, NamingPolicy, pascal_case, words};

/// Keywords and soft keywords, which cannot be (or should not be) member names.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "match", "case", "type", "_",
];

/// Names a generated module uses itself; declarations get `Model` instead.
const MODULE_NAMES: &[&str] = &[
    "Any",
    "Enum",
    "Literal",
    "Never",
    "TypeAlias",
    "TYPE_CHECKING",
    "annotations",
    "dataclass",
    "field",
    "File",
    "Missing",
    "MISSING",
    "TYPE_DEFS",
];

pub struct PythonPolicy;

/// Whether `name` is a Python identifier.
pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

pub fn snake_case(raw: &str) -> String {
    words(raw)
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join("_")
}

fn start(name: String, fallback: &str) -> String {
    if name.is_empty() {
        fallback.to_owned()
    } else if name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{name}")
    } else {
        name
    }
}

impl NamingPolicy for PythonPolicy {
    fn convert(&self, context: Context, raw: &str, preserve: bool) -> String {
        match context {
            Context::Type if preserve && is_identifier(raw) => raw.to_owned(),
            Context::Type => start(pascal_case(raw), "Schema"),
            Context::Constant => {
                let name = words(raw)
                    .iter()
                    .map(|w| w.to_uppercase())
                    .collect::<Vec<_>>()
                    .join("_");
                if name.is_empty() {
                    "EMPTY".to_owned()
                } else if name.starts_with(|c: char| c.is_ascii_digit()) {
                    format!("VALUE_{name}")
                } else {
                    name
                }
            }
            _ => start(snake_case(raw), "value"),
        }
    }

    fn is_reserved(&self, context: Context, ident: &str) -> bool {
        match context {
            Context::Type => KEYWORDS.contains(&ident) || MODULE_NAMES.contains(&ident),
            Context::Constant => false,
            _ => KEYWORDS.contains(&ident),
        }
    }

    fn reserved_suffix(&self, context: Context) -> &str {
        match context {
            Context::Type => "Model",
            _ => "_",
        }
    }
}

/// Allocates names in one scope (a module, a class, an enum).
pub fn allocate(context: Context, raws: &[(&str, bool)]) -> Vec<String> {
    let mut names = Names::new();
    raws.iter()
        .map(|(raw, preserve)| {
            let ident = PythonPolicy.convert(context, raw, *preserve);
            names
                .claim(&PythonPolicy, context, "scope", &ident, raw)
                .name
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_are_snake_case_without_keywords_and_without_collisions() {
        assert_eq!(
            allocate(
                Context::Member,
                &[
                    ("createdAt", false),
                    ("created_at", false),
                    ("class", false),
                    ("type", false)
                ]
            ),
            ["created_at", "created_at2", "class_", "type_"]
        );
        assert_eq!(
            allocate(
                Context::Type,
                &[("pet", false), ("Pet", true), ("Any", true), ("2fa", true)]
            ),
            ["Pet", "Pet2", "AnyModel", "_2fa"]
        );
        assert_eq!(
            allocate(
                Context::Constant,
                &[
                    ("available", false),
                    ("it's", false),
                    ("1", false),
                    ("value", false)
                ]
            ),
            ["AVAILABLE", "IT_S", "VALUE_1", "VALUE"]
        );
    }
}

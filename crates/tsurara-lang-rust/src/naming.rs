//! Rust naming (docs/languages/rust.md): PascalCase types and variants, snake_case fields, modules
//! and functions, SCREAMING_SNAKE_CASE constants.

use tsurara_naming::{Context, Names, NamingPolicy, pascal_case, words};

/// Strict and reserved keywords of Rust 2024.
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
];

/// Keywords that cannot be raw identifiers.
const NOT_RAW: &[&str] = &["crate", "self", "Self", "super", "_"];

/// Type names generated code uses itself.
const TYPE_NAMES: &[&str] = &[
    "Option",
    "Some",
    "None",
    "Result",
    "Ok",
    "Err",
    "String",
    "Vec",
    "Box",
    "BTreeMap",
    "Nullable",
    "File",
    "Value",
    "Serialize",
    "Deserialize",
    "Debug",
    "Clone",
    "PartialEq",
    "Eq",
    "Hash",
    "Copy",
    "Default",
    "Client",
    "Self",
];

pub struct RustPolicy;

fn snake(raw: &str) -> String {
    let s = words(raw)
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join("_");
    if s.is_empty() {
        "value".into()
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{s}")
    } else {
        s
    }
}

/// Module file names: rustc loads `mod x;` from a file only when `x` is ASCII, so other characters
/// become `u` and their code point in hex, as separate words.
fn ascii(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            out.push_str(&format!("u{:x}_", c as u32));
        }
    }
    out.trim_end_matches('_').to_owned()
}

fn pascal(raw: &str, fallback: &str) -> String {
    let s = pascal_case(raw);
    if s.is_empty() {
        fallback.into()
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        format!("V{s}")
    } else {
        s
    }
}

impl NamingPolicy for RustPolicy {
    fn convert(&self, context: Context, raw: &str, _preserve: bool) -> String {
        match context {
            Context::Type => pascal(raw, "Schema"),
            Context::Constant => {
                let s = words(raw)
                    .iter()
                    .map(|w| w.to_uppercase())
                    .collect::<Vec<_>>()
                    .join("_");
                if s.is_empty() {
                    "EMPTY".into()
                } else if s.starts_with(|c: char| c.is_ascii_digit()) {
                    format!("V_{s}")
                } else {
                    s
                }
            }
            Context::File => ascii(&snake(raw)),
            _ => snake(raw),
        }
    }

    fn is_reserved(&self, context: Context, ident: &str) -> bool {
        match context {
            Context::Type => KEYWORDS.contains(&ident) || TYPE_NAMES.contains(&ident),
            // Fields and functions use raw identifiers for keywords (`r#type`), except these.
            Context::Member | Context::Function | Context::Parameter => NOT_RAW.contains(&ident),
            Context::File => KEYWORDS.contains(&ident),
            Context::Constant => false,
        }
    }

    fn reserved_suffix(&self, context: Context) -> &str {
        match context {
            Context::Type => "Model",
            _ => "_",
        }
    }
}

/// A field or function name as written: keywords as raw identifiers.
pub fn ident(name: &str) -> String {
    if KEYWORDS.contains(&name) && !NOT_RAW.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_owned()
    }
}

/// Allocates names in one scope.
pub fn allocate(context: Context, raws: &[&str]) -> Vec<String> {
    let mut names = Names::new();
    raws.iter()
        .map(|raw| {
            let id = RustPolicy.convert(context, raw, false);
            names.claim(&RustPolicy, context, "scope", &id, raw).name
        })
        .collect()
}

/// Enum variant names: PascalCase, numbered when they collide. Variants live in their enum's
/// namespace, so type names such as `String` are fine; only `Self` is not.
pub fn allocate_variants(raws: &[&str]) -> Vec<String> {
    let mut taken = std::collections::HashSet::new();
    raws.iter()
        .map(|raw| {
            let mut base = pascal(raw, "Value");
            if base == "Self" {
                base.push_str("Model");
            }
            let mut name = base.clone();
            let mut n = 2;
            while !taken.insert(name.clone()) {
                name = format!("{base}{n}");
                n += 1;
            }
            name
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_ascii() {
        // rustc loads a module from a file only when its name is ASCII.
        assert_eq!(
            RustPolicy.convert(Context::File, "S3日本語", false),
            "s3_u65e5_u672c_u8a9e"
        );
        assert_eq!(
            RustPolicy.convert(Context::File, "日本", false),
            "u65e5_u672c"
        );
        assert_eq!(
            RustPolicy.convert(Context::Type, "S3日本語", false),
            "S3日本語"
        );
    }
}

//! A naming policy as data, for backends outside this repository that cannot implement
//! `NamingPolicy` in Rust: they describe their rules in `describe`, and the CLI allocates names
//! with this policy. The vocabulary covers what the built-in policies need; Go's policy is
//! expressible (a test compares them), and rules beyond it stay Rust-only.

use crate::{Context, NamingPolicy, words};
use std::collections::BTreeMap;

/// The casing of a context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Case {
    /// `PetStore`
    Pascal,
    /// `petStore`
    Camel,
    /// `pet_store`
    Snake,
    /// `PET_STORE`
    ScreamingSnake,
    /// `pet-store`
    Kebab,
    /// The preferred name with invalid characters replaced by `_`.
    Preserve,
}

/// The rules of one context.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ContextRules {
    pub case: Case,
    /// Prefix of a name that would start with a digit (`V` in Go, `_` in many languages).
    #[serde(default = "underscore")]
    pub digit_prefix: String,
    /// Prefix of a name that does not start with an upper-case letter after casing (Go's `X`, so
    /// that every name is exported). Empty: none.
    #[serde(default)]
    pub upper_prefix: String,
    /// The name of a preferred name with no word in it.
    #[serde(default = "value")]
    pub fallback: String,
    /// Characters outside ASCII become `u` and their code point in hex (`u65e5`), for file names.
    #[serde(default)]
    pub ascii: bool,
    /// Words reserved in this context: keywords, globals, members the language defines.
    #[serde(default)]
    pub reserved: Vec<String>,
    /// Appended to a reserved name.
    #[serde(default = "underscore")]
    pub reserved_suffix: String,
}

fn underscore() -> String {
    "_".into()
}

fn value() -> String {
    "value".into()
}

/// A naming policy as data: rules per context (`type`, `function`, `member`, `parameter`,
/// `constant`, `file`; a missing context uses `type`'s rules), and initialisms written in capitals
/// by the Pascal and camel cases (`ID`, `URL`; `IDs` for the plural).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct DataPolicy {
    pub contexts: BTreeMap<String, ContextRules>,
    #[serde(default)]
    pub initialisms: Vec<String>,
}

/// The key of a context in `DataPolicy::contexts`.
pub fn context_key(context: Context) -> &'static str {
    match context {
        Context::Type => "type",
        Context::Function => "function",
        Context::Member => "member",
        Context::Parameter => "parameter",
        Context::Constant => "constant",
        Context::File => "file",
    }
}

impl DataPolicy {
    fn rules(&self, context: Context) -> Option<&ContextRules> {
        self.contexts
            .get(context_key(context))
            .or_else(|| self.contexts.get("type"))
    }

    fn word(&self, w: &str, first: bool, upper_first: bool) -> String {
        let upper = w.to_uppercase();
        if !self.initialisms.is_empty() && (upper_first || !first) {
            if self.initialisms.contains(&upper) {
                return upper;
            }
            let plural = upper
                .strip_suffix('S')
                .filter(|u| w.ends_with('s') && self.initialisms.iter().any(|i| i == u));
            if let Some(u) = plural {
                return format!("{u}s");
            }
        }
        let lower = w.to_lowercase();
        if upper_first || !first {
            let mut chars = lower.chars();
            let mut out = String::new();
            if let Some(c) = chars.next() {
                out.extend(c.to_uppercase());
            }
            // Keep the rest of the word as written, as `words` splits at case changes.
            out.push_str(&w[w.chars().next().map_or(0, char::len_utf8)..]);
            out
        } else {
            lower
        }
    }
}

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

impl NamingPolicy for DataPolicy {
    fn convert(&self, context: Context, raw: &str, _preserve: bool) -> String {
        let Some(rules) = self.rules(context) else {
            return raw.to_owned();
        };
        let ws = words(raw);
        let joined = |sep: &str, f: &dyn Fn(&str) -> String| {
            ws.iter().map(|w| f(w)).collect::<Vec<_>>().join(sep)
        };
        let mut out = match rules.case {
            Case::Pascal => ws
                .iter()
                .map(|w| self.word(w, false, true))
                .collect::<String>(),
            Case::Camel => ws
                .iter()
                .enumerate()
                .map(|(i, w)| self.word(w, i == 0, false))
                .collect::<String>(),
            Case::Snake => joined("_", &|w| w.to_lowercase()),
            Case::ScreamingSnake => joined("_", &|w| w.to_uppercase()),
            Case::Kebab => joined("-", &|w| w.to_lowercase()),
            Case::Preserve => raw
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect(),
        };
        if rules.ascii {
            out = ascii(&out);
        }
        match out.chars().next() {
            None => rules.fallback.clone(),
            Some(c) if c.is_ascii_digit() => format!("{}{out}", rules.digit_prefix),
            Some(c) if !rules.upper_prefix.is_empty() && !c.is_uppercase() => {
                format!("{}{out}", rules.upper_prefix)
            }
            Some(_) => out,
        }
    }

    fn is_reserved(&self, context: Context, ident: &str) -> bool {
        self.rules(context)
            .is_some_and(|r| r.reserved.iter().any(|w| w == ident))
    }

    fn reserved_suffix(&self, context: Context) -> &str {
        self.rules(context)
            .map_or("_", |r| r.reserved_suffix.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(case: Case) -> DataPolicy {
        DataPolicy {
            contexts: BTreeMap::from([(
                "type".to_owned(),
                ContextRules {
                    case,
                    digit_prefix: "_".into(),
                    upper_prefix: String::new(),
                    fallback: "Value".into(),
                    ascii: false,
                    reserved: vec!["Object".into()],
                    reserved_suffix: "Model".into(),
                },
            )]),
            initialisms: vec!["ID".into(), "URL".into()],
        }
    }

    #[test]
    fn cases() {
        let c = |case, raw| policy(case).convert(Context::Type, raw, false);
        assert_eq!(c(Case::Pascal, "pet_id"), "PetID");
        assert_eq!(c(Case::Camel, "PetId"), "petID");
        assert_eq!(c(Case::Camel, "id"), "id");
        assert_eq!(c(Case::Snake, "photoUrls"), "photo_urls");
        assert_eq!(c(Case::ScreamingSnake, "petId"), "PET_ID");
        assert_eq!(c(Case::Kebab, "PetStore"), "pet-store");
        assert_eq!(c(Case::Preserve, "a-b"), "a_b");
        assert_eq!(c(Case::Pascal, "photoUrls"), "PhotoURLs");
        assert_eq!(c(Case::Pascal, "1st"), "_1st");
        assert_eq!(c(Case::Pascal, "--"), "Value");
        assert!(policy(Case::Pascal).is_reserved(Context::Member, "Object"));
    }
}

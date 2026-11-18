//! Go names (docs/languages/go.md): every type, field, constant and function is exported
//! PascalCase with Go's initialisms (`PetID`, `GetPetByID`); file names are ASCII snake_case.

use tsurara_naming::{Context, Names, NamingPolicy, words};

/// The initialisms golint knows: words written in capitals.
const INITIALISMS: &[&str] = &[
    "ACL", "API", "ASCII", "CPU", "CSS", "DNS", "EOF", "GUID", "HTML", "HTTP", "HTTPS", "ID", "IP",
    "JSON", "LHS", "QPS", "RAM", "RHS", "RPC", "SLA", "SMTP", "SQL", "SSH", "TCP", "TLS", "TTL",
    "UDP", "UI", "UID", "UUID", "URI", "URL", "UTF8", "VM", "XML", "XMPP", "XSRF", "XSS",
];

/// Methods generated types have; a field with the same name would collide.
const METHODS: &[&str] = &["MarshalJSON", "UnmarshalJSON"];

pub struct GoPolicy;

/// Exported PascalCase: initialisms in capitals, `V` before a digit, `X` before a first letter that
/// has no upper case (a field must start with an upper-case letter to be exported).
pub fn exported(raw: &str, fallback: &str) -> String {
    let mut out = String::new();
    for w in words(raw) {
        let upper = w.to_uppercase();
        let plural = upper
            .strip_suffix('S')
            .filter(|u| w.ends_with('s') && INITIALISMS.contains(u));
        if INITIALISMS.contains(&upper.as_str()) {
            out.push_str(&upper);
        } else if let Some(u) = plural {
            // `IDs`, `URLs`
            out.push_str(u);
            out.push('s');
        } else {
            let mut chars = w.chars();
            if let Some(first) = chars.next() {
                out.extend(first.to_uppercase());
                out.push_str(chars.as_str());
            }
        }
    }
    match out.chars().next() {
        None => fallback.to_owned(),
        Some(c) if c.is_ascii_digit() => format!("V{out}"),
        Some(c) if !c.is_uppercase() => format!("X{out}"),
        Some(_) => out,
    }
}

fn snake(raw: &str) -> String {
    let s = words(raw)
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join("_");
    if s.is_empty() { "value".into() } else { s }
}

/// File names are ASCII: other characters become `u` and their code point in hex.
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

impl NamingPolicy for GoPolicy {
    fn convert(&self, context: Context, raw: &str, _preserve: bool) -> String {
        match context {
            Context::File => ascii(&snake(raw)),
            Context::Constant => exported(raw, "Empty"),
            _ => exported(raw, "Value"),
        }
    }

    fn is_reserved(&self, context: Context, ident: &str) -> bool {
        match context {
            Context::Member => METHODS.contains(&ident),
            _ => false,
        }
    }

    fn reserved_suffix(&self, context: Context) -> &str {
        match context {
            Context::Member => "Field",
            Context::File => "_",
            _ => "Type",
        }
    }
}

/// Allocates names in one scope, in order.
pub fn allocate(context: Context, raws: &[&str]) -> Vec<String> {
    let mut names = Names::new();
    raws.iter()
        .map(|raw| {
            let id = GoPolicy.convert(context, raw, false);
            names.claim(&GoPolicy, context, "scope", &id, raw).name
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_exported_with_initialisms() {
        let c = |raw| GoPolicy.convert(Context::Type, raw, false);
        assert_eq!(c("petId"), "PetID");
        assert_eq!(c("get_pet_by_id"), "GetPetByID");
        assert_eq!(c("URLPath"), "URLPath");
        assert_eq!(c("api-key"), "APIKey");
        assert_eq!(c("photoUrls"), "PhotoURLs");
        assert_eq!(c("ids"), "IDs");
        assert_eq!(c("status"), "Status");
        assert_eq!(c("1st"), "V1st");
        assert_eq!(c("日本語"), "X日本語");
        assert_eq!(c("S日本語"), "S日本語");
        assert_eq!(c("--"), "Value");
        assert_eq!(GoPolicy.convert(Context::File, "PetID", false), "pet_id");
        assert_eq!(
            GoPolicy.convert(Context::File, "S日本語", false),
            "s_u65e5_u672c_u8a9e"
        );
        assert_eq!(
            allocate(Context::Member, &["MarshalJSON", "x"]),
            ["MarshalJSONField", "X"]
        );
    }
}

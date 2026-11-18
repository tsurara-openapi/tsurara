//! Name allocation shared by all backends (design.md 6.6, 18.6). A backend replays the name requests of
//! the model in model order and claims identifiers through `Names`, with its own `NamingPolicy`.
//!
//! Published for backends outside this repository, with one version for all tsurara crates,
//! released with the CLI and moving with `semVersion`. Stable: `Names`, `NamingPolicy`, `Context`, the
//! casing helpers, and `data::DataPolicy` (the policy external backends describe as data).

pub mod data;

use std::collections::{HashMap, HashSet};

/// What an identifier names (design.md 18.6.1). Reserved words and suffixes depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    Type,
    Function,
    Member,
    Parameter,
    Constant,
    File,
}

/// A language's naming rules. The allocation algorithm in `Names` is shared.
pub trait NamingPolicy {
    /// Converts a preferred name for a context (casing, invalid characters).
    fn convert(&self, context: Context, raw: &str, preserve: bool) -> String;
    /// Words reserved in a context: keywords, globals, and members the language defines.
    fn is_reserved(&self, context: Context, ident: &str) -> bool;
    /// Suffix for reserved names (`Model` for TypeScript types).
    fn reserved_suffix(&self, context: Context) -> &str;
}

/// Name assignment. Names are decided in call order, so the same definition always yields the same names.
#[derive(Debug, Default)]
pub struct Names {
    exact: HashSet<String>,
    /// Lower-cased names per folding scope. Names differing only in case collide within a scope
    /// (an output directory, so that file names do not clash on case-insensitive file systems).
    folded: HashMap<String, HashSet<String>>,
    /// JSON Pointer of what each name was assigned to. Used in collision messages.
    owners: HashMap<String, String>,
}

/// Result of assigning a name. `changed` is the reason when it differs from the requested name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    pub name: String,
    pub changed: Option<String>,
}

impl Names {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_free(&self, scope: &str, name: &str) -> bool {
        !self.exact.contains(name)
            && !self
                .folded
                .get(scope)
                .is_some_and(|s| s.contains(&name.to_lowercase()))
    }

    fn take(&mut self, scope: &str, name: &str, owner: &str) {
        self.owners
            .entry(name.to_owned())
            .or_insert_with(|| owner.to_owned());
        self.exact.insert(name.to_owned());
        self.folded
            .entry(scope.to_owned())
            .or_default()
            .insert(name.to_lowercase());
    }

    /// Assigns the identifier `ident` (already converted). Reserved words get the policy's suffix, and a
    /// number is appended if it still collides.
    pub fn claim(
        &mut self,
        policy: &dyn NamingPolicy,
        context: Context,
        scope: &str,
        ident: &str,
        owner: &str,
    ) -> Claimed {
        self.claim_group(policy, context, scope, ident, &[], owner)
            .0
    }

    /// Assigns a base name together with names derived from it (such as `{}Data`).
    /// In each element of `derived`, `{}` is replaced by the base name and `{P}` by the base name
    /// with its first letter capitalized. A collision of the base name appends a number. A derived
    /// name that is already taken (usually by a schema) moves aside instead: `Operation` goes before
    /// its suffix (`{P}OperationResult`), then a number if that is taken too, as kubb does. The base
    /// name and the other derived names stay, so adding a schema never renames the function
    /// (design.md 6.6).
    #[allow(clippy::too_many_arguments)]
    pub fn claim_group(
        &mut self,
        policy: &dyn NamingPolicy,
        context: Context,
        scope: &str,
        ident: &str,
        derived: &[&str],
        owner: &str,
    ) -> (Claimed, Vec<Claimed>) {
        let mut reason = None;
        let mut base = ident.to_owned();
        if policy.is_reserved(context, &base) {
            reason = Some(format!("`{ident}` is a reserved or global name"));
            base.push_str(policy.reserved_suffix(context));
        }
        let mut candidate = base.clone();
        let mut n = 2;
        while !self.is_free(scope, &candidate) {
            candidate = format!("{base}{n}");
            n += 1;
        }
        if candidate != base && reason.is_none() {
            reason = Some(format!("`{ident}` is already used"));
        }
        self.take(scope, &candidate, owner);
        let mut names = Vec::with_capacity(derived.len());
        for template in derived {
            let wanted = expand(template, &candidate);
            let mut name = wanted.clone();
            let mut changed = None;
            if self.exact.contains(&name) {
                let aside = expand(&with_operation(template), &candidate);
                name = aside.clone();
                let mut n = 2;
                while self.exact.contains(&name) {
                    name = format!("{aside}{n}");
                    n += 1;
                }
                changed = Some(format!(
                    "`{wanted}` is already used by {}",
                    self.owners
                        .get(&wanted)
                        .map_or("another name", String::as_str)
                ));
            }
            self.owners.insert(name.clone(), owner.to_owned());
            self.exact.insert(name.clone());
            names.push(Claimed { name, changed });
        }
        (
            Claimed {
                name: candidate,
                changed: reason,
            },
            names,
        )
    }

    /// Reserves a name given by an override as is.
    pub fn reserve(&mut self, scope: &str, name: &str, owner: &str) {
        self.take(scope, name, owner);
    }

    pub fn is_taken(&self, name: &str) -> bool {
        self.exact.contains(name)
    }
}

/// `{P}Result` → `{P}OperationResult`: the template with `Operation` after the base name.
fn with_operation(template: &str) -> String {
    for base in ["{P}", "{}"] {
        if let Some(rest) = template.strip_prefix(base) {
            return format!("{base}Operation{rest}");
        }
    }
    format!("{template}Operation")
}

fn expand(template: &str, base: &str) -> String {
    template
        .replace("{P}", &type_prefix(base))
        .replace("{}", base)
}

/// Whether the name is a valid identifier. Unicode letters are allowed.
pub fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// Splits a name into words, at non-alphanumeric characters and at `camelCase` boundaries.
pub fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(&prev) = current.chars().last().as_ref() {
            let next = chars.get(i + 1).copied();
            let boundary = (prev.is_lowercase() && c.is_uppercase())
                || (prev.is_uppercase()
                    && c.is_uppercase()
                    && next.is_some_and(|n| n.is_lowercase()));
            if boundary {
                words.push(std::mem::take(&mut current));
            }
        }
        current.push(c);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn lower_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn pascal_case(name: &str) -> String {
    words(name).iter().map(|w| capitalize(w)).collect()
}

pub fn camel_case(name: &str) -> String {
    lower_first(&pascal_case(name))
}

/// Prefixes names starting with a digit with `_`. Uses `fallback` if the name is empty.
fn ensure_start(name: String, fallback: &str) -> String {
    if name.is_empty() {
        fallback.to_owned()
    } else if name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("_{name}")
    } else {
        name
    }
}

/// Builds a type name from a schema name. If `pascal` is false, valid identifiers are kept as is.
pub fn type_name(raw: &str, pascal: bool) -> String {
    if !pascal && is_identifier(raw) {
        return raw.to_owned();
    }
    ensure_start(pascal_case(raw), "Schema")
}

/// Builds a function name from an `operationId`. Valid identifiers are kept as is.
pub fn function_name(raw: &str) -> String {
    if is_identifier(raw) {
        return raw.to_owned();
    }
    ensure_start(camel_case(raw), "operation")
}

/// Builds a function name from the method and path for an operation without `operationId`.
/// `GET /pets/{petId}/photos` becomes `getPetsByPetIdPhotos`.
pub fn function_name_from_path(method: &str, path: &str) -> String {
    let mut name = method.to_lowercase();
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        if let Some(param) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            name.push_str("By");
            name.push_str(&pascal_case(param));
        } else {
            name.push_str(&pascal_case(segment));
        }
    }
    ensure_start(name, "operation")
}

/// Builds a type name prefix from a function name (capitalizes the first letter).
pub fn type_prefix(function_name: &str) -> String {
    capitalize(function_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_splitting() {
        assert_eq!(words("getPetByID"), ["get", "Pet", "By", "ID"]);
        assert_eq!(words("HTTPServer"), ["HTTP", "Server"]);
        assert_eq!(words("pet-store_v2"), ["pet", "store", "v2"]);
        assert_eq!(pascal_case("pet-store"), "PetStore");
        assert_eq!(camel_case("Get Pet By Id"), "getPetById");
    }

    /// A policy like Python's for members: snake_case, with `_` after keywords.
    struct Snake;

    impl NamingPolicy for Snake {
        fn convert(&self, _: Context, raw: &str, _: bool) -> String {
            words(raw)
                .iter()
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
                .join("_")
        }
        fn is_reserved(&self, _: Context, ident: &str) -> bool {
            matches!(ident, "class" | "from")
        }
        fn reserved_suffix(&self, _: Context) -> &str {
            "_"
        }
    }

    #[test]
    fn members_are_allocated_per_declaration_with_a_snake_case_policy() {
        let allocate = |wire: &[&str]| {
            // One scope per declaration.
            let mut names = Names::new();
            wire.iter()
                .map(|w| {
                    let ident = Snake.convert(Context::Member, w, false);
                    names
                        .claim(&Snake, Context::Member, "members", &ident, w)
                        .name
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            allocate(&["fooBar", "foo_bar", "class", "from"]),
            ["foo_bar", "foo_bar2", "class_", "from_"]
        );
        // The same declaration always gets the same names, and another declaration starts afresh.
        assert_eq!(allocate(&["fooBar", "foo_bar"]), ["foo_bar", "foo_bar2"]);
        assert_eq!(allocate(&["foo_bar"]), ["foo_bar"]);
    }
}

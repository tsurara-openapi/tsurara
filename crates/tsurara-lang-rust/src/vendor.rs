//! `runtime: vendor`: a copy of `tsurara-client` in the module `runtime` next to `types`.
//! Generated code and the copy reach it through `runtime_crate`, the module's path in the user's
//! crate (`crate::api::runtime`): the copy's `crate::` and `$crate::` paths are rewritten to it, its
//! macros are exported by path instead of `#[macro_export]`, and its cargo features are resolved.

use crate::HEADER;
use tsurara_backend::OutputFile;

/// The module of the copy, relative to the generated module.
pub const DIR: &str = "runtime";

/// The crates the copy needs in the user's `Cargo.toml`.
pub fn dependencies(client: bool) -> Vec<&'static str> {
    let mut deps = vec![
        r#"serde = { version = "1", features = ["derive"] }"#,
        r#"serde_json = { version = "1", features = ["preserve_order"] }"#,
        r#"indexmap = { version = "2", features = ["serde"] }"#,
    ];
    if client {
        deps.extend([
            r#"tokio = { version = "1", features = ["time"] }"#,
            r#"fastrand = "2""#,
            r#"httpdate = "1""#,
            r#"encoding_rs = "0.8.35""#,
            r#"reqwest = { version = "0.12", default-features = false, features = ["rustls-tls"] }"#,
        ]);
    }
    deps
}

/// The copy's files: `client` keeps the client layer (operations are generated), `mocks` the mock
/// transport (compiled like generated mocks, for tests and the crate feature `mocks`).
pub fn files(runtime_crate: &str, client: bool, mocks: bool) -> Vec<OutputFile> {
    let lib = tsurara_runtimes::RUST
        .iter()
        .find(|(p, _)| *p == "lib.rs")
        .expect("the runtime has lib.rs")
        .1;
    let module = resolve_features(lib, client, mocks);
    let mut macros = Vec::new();
    for (_, source) in tsurara_runtimes::RUST {
        for line in source.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("macro_rules! ") {
                macros.push(rest.trim_end_matches(" {").trim().to_owned());
            }
        }
    }
    let mut out = Vec::new();
    for (path, source) in tsurara_runtimes::RUST {
        let (target, mut text) = if *path == "lib.rs" {
            // The runtime supports Rust 1.85, which has no let chains for clippy's suggestion.
            let mut text = format!("#![allow(clippy::collapsible_if)]\n{module}");
            text.push('\n');
            for m in &macros {
                text.push_str(&format!(
                    "#[allow(unused_imports)]\npub(crate) use self::macros::{m};\n"
                ));
            }
            ("mod.rs".to_owned(), text)
        } else {
            let stem = path.trim_end_matches(".rs");
            if !module
                .lines()
                .any(|l| l.trim_end_matches(';').ends_with(&format!("mod {stem}")))
            {
                continue;
            }
            (path.to_string(), resolve_features(source, client, mocks))
        };
        if text.contains("#[macro_export]") {
            text = text.replace("#[macro_export]\n", "#[allow(unused_macros)]\n");
            for m in &macros {
                text.push_str(&format!("\npub(crate) use {m};\n"));
            }
        }
        out.push(OutputFile {
            path: format!("{DIR}/{target}"),
            contents: format!(
                "{HEADER}\n// {}\n\n{}",
                tsurara_runtimes::NOTICE,
                absolute_paths(&text, runtime_crate)
            ),
        });
    }
    out
}

/// `crate::`, `$crate::` and `tsurara_client::` name the runtime crate; in the copy they name its
/// module.
fn absolute_paths(source: &str, runtime_crate: &str) -> String {
    const MARK: &str = "\u{0}RT::";
    // Examples in its documentation name the crate too.
    let mut s = source
        .replace("$crate::", MARK)
        .replace("tsurara_client::", MARK);
    let mut out = String::with_capacity(s.len());
    while let Some(i) = s.find("crate::") {
        let before = s[..i].chars().last();
        out.push_str(&s[..i]);
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':') {
            out.push_str("crate::");
        } else {
            out.push_str(MARK);
        }
        s = s[i + "crate::".len()..].to_owned();
    }
    out.push_str(&s);
    out.replace(MARK, &format!("{runtime_crate}::"))
}

/// Resolves the runtime's cargo features, which the user's crate does not have: the attribute lines
/// go, and the items of disabled features go with them (each is one line, or a block).
fn resolve_features(source: &str, client: bool, mocks: bool) -> String {
    let mut out = String::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        let attr = line.trim();
        let keep = match attr {
            r#"#[cfg(feature = "client")]"# | r#"#[cfg(feature = "reqwest")]"# => Some(client),
            r#"#[cfg(not(feature = "reqwest"))]"# => Some(false),
            r#"#[cfg(all(feature = "client", feature = "testing"))]"# => {
                if client && mocks {
                    let indent = &line[..line.len() - line.trim_start().len()];
                    out.push_str(&format!("{indent}#[cfg(any(test, feature = \"mocks\"))]\n"));
                    continue;
                }
                Some(false)
            }
            _ => None,
        };
        match keep {
            None => {
                out.push_str(line);
                out.push('\n');
            }
            Some(true) => {}
            Some(false) => skip_item(&mut lines),
        }
    }
    out
}

/// Skips the item after an attribute: one line, or up to its closing brace.
fn skip_item<'a>(lines: &mut std::iter::Peekable<impl Iterator<Item = &'a str>>) {
    let mut depth = 0i32;
    for line in lines.by_ref() {
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if depth <= 0 && (line.trim_end().ends_with(';') || line.trim_end().ends_with('}')) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_crate_paths() {
        assert_eq!(
            absolute_paths(
                "use crate::a;\nimpl $crate::serde::X for T {}\npub(crate) fn f() {}\nuse other::crate::x;\n",
                "crate::api::runtime"
            ),
            "use crate::api::runtime::a;\nimpl crate::api::runtime::serde::X for T {}\npub(crate) fn f() {}\nuse other::crate::x;\n"
        );
    }

    #[test]
    fn resolves_features() {
        let src = "#[cfg(feature = \"client\")]\nmod client;\n#[cfg(not(feature = \"reqwest\"))]\n{\n    a();\n}\nfn b() {}\n";
        assert_eq!(
            resolve_features(src, true, false),
            "mod client;\nfn b() {}\n"
        );
        assert_eq!(resolve_features(src, false, false), "fn b() {}\n");
    }

    #[test]
    fn types_only_copies_the_types_layer() {
        let files = files("crate::api::runtime", false, false);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "runtime/file.rs",
                "runtime/mod.rs",
                "runtime/macros.rs",
                "runtime/nullable.rs"
            ]
        );
    }
}

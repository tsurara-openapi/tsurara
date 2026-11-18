//! Embeds the edge-case corpus (for `tsurara test-backend`) and `spec/` (for `tsurara spec`), so that
//! backends and runtimes in other repositories need only the binary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).expect("under root");
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn embed(code: &mut String, name: &str, root: &Path, keep: fn(&str) -> bool) {
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(root, root, &mut files);
    files.retain(|f| keep(f));
    files.sort();
    writeln!(code, "pub const {name}: &[(&str, &str)] = &[").unwrap();
    for f in files {
        let full = root.join(&f);
        println!("cargo:rerun-if-changed={}", full.display());
        writeln!(
            code,
            "    ({f:?}, include_str!({:?})),",
            full.to_string_lossy()
        )
        .unwrap();
    }
    code.push_str("];\n");
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let repo = manifest
        .join("../..")
        .canonicalize()
        .expect("repository root");
    let mut code = String::new();
    embed(
        &mut code,
        "EDGE_CASES",
        &repo.join("corpus/edge-cases"),
        |f| f.ends_with(".yaml") || f.ends_with(".json"),
    );
    embed(&mut code, "SPEC", &repo.join("spec"), |f| {
        f.ends_with(".json") || f.ends_with(".md")
    });
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir"));
    std::fs::write(out.join("embedded.rs"), code).expect("write embedded.rs");
}

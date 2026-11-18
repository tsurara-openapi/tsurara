//! Embeds the runtime sources of `runtimes/` (without tests) as `(path, contents)` pairs per language.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Whether a file of a runtime's source directory is vendored.
type Keep = fn(&str) -> bool;

/// The source directory of each runtime, and which files of it are vendored.
const RUNTIMES: &[(&str, &str, Keep)] = &[
    ("TYPESCRIPT", "typescript/src", |p| {
        p.ends_with(".ts") && !p.ends_with(".test.ts")
    }),
    ("PYTHON", "python/src/tsurara_client", |p| {
        (p.ends_with(".py") || p == "py.typed") && !p.contains("__pycache__")
    }),
    ("RUST", "rust/src", |p| p.ends_with(".rs")),
    ("GO", "go", |p| {
        p.ends_with(".go") && !p.ends_with("_test.go")
    }),
];

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read runtime directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).expect("under root");
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let runtimes = manifest
        .join("../../runtimes")
        .canonicalize()
        .expect("runtimes/");
    let mut code = String::new();
    for (name, dir, keep) in RUNTIMES {
        let root = runtimes.join(dir);
        println!("cargo:rerun-if-changed={}", root.display());
        let mut files = Vec::new();
        walk(&root, &root, &mut files);
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
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir"));
    std::fs::write(out.join("runtimes.rs"), code).expect("write runtimes.rs");
}

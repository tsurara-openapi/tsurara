//! An external backend (design.md 18.11): `describe` prints what the program speaks, `generate`
//! reads the model on stdin and prints the files. Configure it with
//! `language: { command: [cargo, run, --quiet, --manifest-path, path/to/Cargo.toml, --] }`.

mod backend;

use std::io::Read;

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("describe") => {
            let d = serde_json::json!({
                "protocol": 2,
                "semVersions": [tsurara_sem::SEM_VERSION],
                "markers": { "generated": backend::MARKER },
            });
            println!("{d}");
        }
        Some("generate") => {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .expect("read stdin");
            let mut request: serde_json::Value = serde_json::from_str(&input).expect("JSON");
            let model: tsurara_sem::Model =
                serde_json::from_value(request["model"].take()).expect("a model");
            let (files, warnings) = backend::generate(&model);
            let out = serde_json::json!({ "files": files, "warnings": warnings });
            println!("{out}");
        }
        _ => {
            eprintln!("usage: tsurara-backend-example describe|generate");
            std::process::exit(2);
        }
    }
}

//! `tsurara test-backend`: runs a backend outside this repository on the edge-case corpus
//! (embedded in the binary) and on definitions of the user's, and checks each output with the
//! backend's checker command, as the built-in backends are checked by the corpus tests.
//!
//! `tsurara spec` writes the embedded `spec/` (schemas and conformance vectors) for runtime authors.

use std::path::{Path, PathBuf};
use std::process::Command;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
}

/// Splits a command line on whitespace; arguments that name existing files or directories relative
/// to the current directory become absolute, since commands run in other directories.
fn command(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|a| {
            let p = Path::new(a);
            if p.is_relative() && (a.contains('/') || a.contains('.')) && p.exists() {
                std::fs::canonicalize(p)
                    .map_or_else(|_| a.to_owned(), |p| p.to_string_lossy().into_owned())
            } else {
                a.to_owned()
            }
        })
        .collect()
}

/// A definition to run: a name and its text.
struct Definition {
    name: String,
    text: String,
}

fn definitions(dirs: &[PathBuf]) -> Result<Vec<Definition>, String> {
    let mut out: Vec<Definition> = embedded::EDGE_CASES
        .iter()
        .map(|(name, text)| Definition {
            name: format!("edge-cases/{name}"),
            text: (*text).to_owned(),
        })
        .collect();
    for dir in dirs {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e == "yaml" || e == "yml" || e == "json")
            })
            .collect();
        found.sort();
        for path in found {
            out.push(Definition {
                name: path.display().to_string(),
                text: std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?,
            });
        }
    }
    Ok(out)
}

fn yaml_string(s: &str) -> String {
    serde_json::to_string(s).expect("a string serializes")
}

/// Runs the test; returns the number of failures.
pub fn run(
    backend: &str,
    check: Option<&str>,
    options: Option<&str>,
    dirs: &[PathBuf],
    keep: Option<&Path>,
) -> Result<usize, String> {
    let backend = command(backend);
    let check = check.map(command);
    if backend.is_empty() {
        return Err("--backend is empty".into());
    }
    let options: serde_json::Value = match options {
        Some(o) => serde_json::from_str(o).map_err(|e| format!("--options is not JSON: {e}"))?,
        None => serde_json::json!({}),
    };
    let work = match keep {
        Some(dir) => dir.to_owned(),
        None => std::env::temp_dir().join(format!("tsurara-test-backend-{}", std::process::id())),
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let list = definitions(dirs)?;
    let mut failures = 0;
    for (i, def) in list.iter().enumerate() {
        let dir = work.join(format!("{i:03}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let ext = if def.name.ends_with(".json") {
            "json"
        } else {
            "yaml"
        };
        let input = format!("openapi.{ext}");
        std::fs::write(dir.join(&input), &def.text).map_err(|e| e.to_string())?;
        let config = format!(
            "input: {input}\noutput: out\nlanguage:\n  command: [{}]\n  options: {}\n",
            backend
                .iter()
                .map(|a| yaml_string(a))
                .collect::<Vec<_>>()
                .join(", "),
            options
        );
        std::fs::write(dir.join("tsurara.config.yaml"), config).map_err(|e| e.to_string())?;
        let generated = Command::new(&exe)
            .args(["generate", "--config"])
            .arg(dir.join("tsurara.config.yaml"))
            .current_dir(&dir)
            .output()
            .map_err(|e| e.to_string())?;
        let mut failure = (!generated.status.success()).then(|| {
            format!(
                "generate failed:\n{}",
                String::from_utf8_lossy(&generated.stderr)
            )
        });
        if let (None, Some(check)) = (&failure, &check) {
            let out = dir.join("out");
            let _ = std::fs::create_dir_all(&out);
            let checked = Command::new(&check[0])
                .args(&check[1..])
                .current_dir(&out)
                .env("TSURARA_OUTPUT", &out)
                .env("TSURARA_DEFINITION", dir.join(&input))
                .output()
                .map_err(|e| format!("{}: {e}", check[0]))?;
            if !checked.status.success() {
                failure = Some(format!(
                    "the check failed:\n{}{}",
                    String::from_utf8_lossy(&checked.stdout),
                    String::from_utf8_lossy(&checked.stderr)
                ));
            }
        }
        match failure {
            None => {
                println!("ok   {}", def.name);
                if keep.is_none() {
                    let _ = std::fs::remove_dir_all(&dir);
                }
            }
            Some(why) => {
                failures += 1;
                println!("FAIL {} (kept in {})", def.name, dir.display());
                for line in why.trim_end().lines() {
                    println!("     {line}");
                }
            }
        }
    }
    println!("{} definition(s), {} failed", list.len(), failures);
    if failures == 0 && keep.is_none() {
        let _ = std::fs::remove_dir_all(&work);
    }
    Ok(failures)
}

/// Writes the embedded `spec/` into `dir`.
pub fn write_spec(dir: &Path) -> Result<usize, String> {
    for (path, contents) in embedded::SPEC {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().expect("a parent")).map_err(|e| e.to_string())?;
        std::fs::write(&full, contents).map_err(|e| format!("{}: {e}", full.display()))?;
    }
    Ok(embedded::SPEC.len())
}

//! A cache of checker runs that passed: the same generated files, checked by the same
//! checker with the same settings against the same runtime, toolchain and dependencies, pass again,
//! so the checker is not run. Most changes leave most outputs of the corpus as they were.
//!
//! The key covers everything a check depends on: the checker and its settings, the files, the
//! runtime sources of the language, the lock files and `mise.toml` (the toolchain versions), and this
//! crate's source (how the checkers set up their projects). Only successes are recorded, under
//! `target/checker-cache`; `TSURARA_CHECKER_CACHE=0` turns the cache off.

use crate::repo_root;
use std::path::{Path, PathBuf};

/// Files and directories every check depends on, relative to the repository root.
const COMMON: &[&str] = &["mise.toml", "package.json", "pnpm-lock.yaml"];

/// The key of a check, or `None` when the cache is off.
pub(crate) fn key(
    checker: &str,
    settings: &str,
    files: &[(String, String)],
    inputs: &[&str],
) -> Option<String> {
    if std::env::var("TSURARA_CHECKER_CACHE").is_ok_and(|v| v == "0") {
        return None;
    }
    let mut h = Hasher::new();
    h.add(checker.as_bytes());
    h.add(settings.as_bytes());
    h.add(include_bytes!("lib.rs"));
    h.add(include_bytes!("cache.rs"));
    let mut sorted: Vec<&(String, String)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, contents) in sorted {
        h.add(path.as_bytes());
        h.add(contents.as_bytes());
    }
    let root = repo_root();
    for input in COMMON.iter().chain(inputs) {
        add_path(&mut h, &root, &root.join(input));
    }
    Some(h.hex())
}

/// Whether a check with this key passed before.
pub(crate) fn hit(key: Option<&str>) -> bool {
    key.is_some_and(|k| marker(k).exists())
}

/// Records that the check with this key passed.
pub(crate) fn store(key: Option<&str>) {
    if let Some(k) = key {
        let path = marker(k);
        let _ = std::fs::create_dir_all(path.parent().expect("a parent"));
        let _ = std::fs::write(path, "");
    }
}

static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Uses `dir` instead of `target/checker-cache` in this process (for the cache's own test).
pub fn use_checker_cache_dir(dir: &Path) {
    let _ = DIR.set(dir.to_owned());
}

/// `target/checker-cache`, or the directory of `use_checker_cache_dir`.
fn marker(key: &str) -> PathBuf {
    DIR.get()
        .cloned()
        .unwrap_or_else(|| repo_root().join("target/checker-cache"))
        .join(key)
}

/// Adds a file, or every file under a directory in a stable order, with its path.
fn add_path(h: &mut Hasher, root: &Path, path: &Path) {
    let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
            .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        entries.sort();
        for entry in entries {
            let name = entry.file_name().unwrap_or_default().to_string_lossy();
            if matches!(
                name.as_ref(),
                "node_modules" | "target" | "dist" | "__pycache__" | ".venv" | ".pytest_cache"
            ) {
                continue;
            }
            add_path(h, root, &entry);
        }
    } else {
        h.add(relative.as_bytes());
        match std::fs::read(path) {
            Ok(bytes) => h.add(&bytes),
            Err(_) => h.add(b"(missing)"),
        }
    }
}

/// Two FNV-1a 64-bit hashes with different offsets, as 32 hex digits.
struct Hasher {
    a: u64,
    b: u64,
}

impl Hasher {
    fn new() -> Self {
        Self {
            a: 0xcbf2_9ce4_8422_2325,
            b: 0x6c62_272e_07bb_0142,
        }
    }

    /// Adds a length-prefixed chunk, so that chunk boundaries count.
    fn add(&mut self, bytes: &[u8]) {
        for byte in (bytes.len() as u64).to_le_bytes().iter().chain(bytes) {
            self.a = (self.a ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
            self.b = (self.b ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
            self.b = self.b.rotate_left(5);
        }
    }

    fn hex(&self) -> String {
        format!("{:016x}{:016x}", self.a, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(contents: &str) -> Vec<(String, String)> {
        vec![
            ("a.ts".into(), contents.into()),
            ("b.ts".into(), "x".into()),
        ]
    }

    #[test]
    fn keys_follow_every_input() {
        let k = |checker: &str, settings: &str, contents: &str| {
            key(
                checker,
                settings,
                &files(contents),
                &["runtimes/typescript/src"],
            )
            .unwrap()
        };
        let base = k("tsc", "strict", "1");
        assert_eq!(base, k("tsc", "strict", "1"));
        assert_ne!(base, k("tsc", "strict", "2"));
        assert_ne!(base, k("tsc", "basic", "1"));
        assert_ne!(base, k("pyright", "strict", "1"));
        // The order of the files does not matter.
        let mut reversed = files("1");
        reversed.reverse();
        assert_eq!(
            base,
            key("tsc", "strict", &reversed, &["runtimes/typescript/src"]).unwrap()
        );
    }

    #[test]
    fn chunk_boundaries_count() {
        let mut a = Hasher::new();
        a.add(b"ab");
        a.add(b"c");
        let mut b = Hasher::new();
        b.add(b"a");
        b.add(b"bc");
        assert_ne!(a.hex(), b.hex());
    }
}

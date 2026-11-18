//! Compares the runtime library version range required by generated code with the installed version
//! (design.md 8.6, chapter 12).

use std::path::{Path, PathBuf};

/// The installed version of `module`, found in `node_modules` from `dir` upward, with its manifest.
pub fn installed(dir: &Path, module: &str) -> Option<(PathBuf, String)> {
    dir.ancestors().find_map(|d| {
        let manifest = d.join("node_modules").join(module).join("package.json");
        let text = std::fs::read_to_string(&manifest).ok()?;
        let value: serde_json::Value = serde_json::from_str(&text).ok()?;
        let version = value.get("version")?.as_str()?.to_owned();
        Some((manifest, version))
    })
}

/// The version of the crate `name` in the nearest `Cargo.lock` from `dir` upward, with the lock file.
pub fn locked(dir: &Path, name: &str) -> Option<(PathBuf, String)> {
    let lock = dir
        .ancestors()
        .map(|d| d.join("Cargo.lock"))
        .find(|p| p.is_file())?;
    let text = std::fs::read_to_string(&lock).ok()?;
    let mut current: Option<&str> = None;
    for line in text.lines().map(str::trim) {
        if line == "[[package]]" {
            current = None;
        } else if let Some(n) = line.strip_prefix("name = ") {
            current = Some(n.trim_matches('"'));
        } else if let (Some(v), Some(n)) = (line.strip_prefix("version = "), current) {
            if n == name {
                return Some((lock, v.trim_matches('"').to_owned()));
            }
        }
    }
    None
}

/// What the nearest `go.mod` from `dir` upward says about a module.
pub enum GoRequirement {
    /// The required version (without `v`), with the `go.mod`.
    Version(PathBuf, String),
    /// A `replace` directive points the module elsewhere.
    Replaced(PathBuf),
}

pub fn go_required(dir: &Path, module: &str) -> Option<GoRequirement> {
    let go_mod = dir
        .ancestors()
        .map(|d| d.join("go.mod"))
        .find(|p| p.is_file())?;
    let text = std::fs::read_to_string(&go_mod).ok()?;
    let mut block = "";
    let mut version = None;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        if line == ")" {
            block = "";
            continue;
        }
        let (directive, rest) = match line.split_once(char::is_whitespace) {
            Some((d, r)) if ["require", "replace"].contains(&d) => {
                let r = r.trim();
                if r == "(" {
                    block = d;
                    continue;
                }
                (d, r)
            }
            _ => (block, line),
        };
        let mut words = rest.split_whitespace();
        if words.next() != Some(module) {
            continue;
        }
        match directive {
            "replace" => return Some(GoRequirement::Replaced(go_mod)),
            "require" => {
                version = words.next().map(|v| v.trim_start_matches('v').to_owned());
            }
            _ => {}
        }
    }
    version.map(|v| GoRequirement::Version(go_mod, v))
}

/// Whether `version` satisfies `range`. Only exact versions and caret ranges (`^1.2.3`) are understood;
/// `None` for anything else. Pre-release and build suffixes are ignored.
pub fn satisfies(range: &str, version: &str) -> Option<bool> {
    let v = parse(version)?;
    let range = range.trim();
    if let Some(base) = range.strip_prefix('^') {
        let b = parse(base)?;
        // The first non-zero component may not change (npm semver).
        let upper = match b {
            (0, 0, p) => (0, 0, p + 1),
            (0, m, _) => (0, m + 1, 0),
            (major, _, _) => (major + 1, 0, 0),
        };
        return Some(v >= b && v < upper);
    }
    Some(v == parse(range)?)
}

fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}

#[cfg(test)]
mod tests {
    use super::satisfies;

    #[test]
    fn caret_ranges() {
        assert_eq!(satisfies("^0.1.0", "0.1.7"), Some(true));
        assert_eq!(satisfies("^0.1.0", "0.2.0"), Some(false));
        assert_eq!(satisfies("^0.1.2", "0.1.1"), Some(false));
        assert_eq!(satisfies("^1.2.0", "1.9.0"), Some(true));
        assert_eq!(satisfies("^1.2.0", "2.0.0"), Some(false));
        assert_eq!(satisfies("^0.0.3", "0.0.4"), Some(false));
        assert_eq!(satisfies("^0.1.0", "0.1.0-beta.1"), Some(true));
        assert_eq!(satisfies("0.1.0", "0.1.0"), Some(true));
        assert_eq!(satisfies(">=0.1", "0.1.0"), None);
    }
}

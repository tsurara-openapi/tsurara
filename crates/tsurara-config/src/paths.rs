//! Lexical path helpers. Output directories may not exist yet, so nothing touches the file system.

use std::path::{Component, Path, PathBuf};

/// Joins `path` onto `base` unless it is absolute, and removes `.` and `..` lexically.
pub fn resolve(base: &Path, path: &str) -> PathBuf {
    normalize(&base.join(path))
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Path of `to` relative to the directory `from`, with `/` separators. Both must be absolute and normalized.
/// Returns `to` as is when they share no root (different Windows drives).
pub fn relative(from: &Path, to: &Path) -> String {
    let from: Vec<Component> = from.components().collect();
    let to_parts: Vec<Component> = to.components().collect();
    if from.first() != to_parts.first() {
        return to.to_string_lossy().replace('\\', "/");
    }
    let common = from
        .iter()
        .zip(&to_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = vec!["..".to_owned(); from.len() - common];
    parts.extend(
        to_parts[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// Like [`relative`], but as an import specifier that starts with `./` or `../`.
pub fn relative_specifier(from: &Path, to: &Path) -> String {
    let rel = relative(from, to);
    if rel.starts_with("../") || rel == ".." || Path::new(&rel).is_absolute() {
        rel
    } else if rel.is_empty() {
        ".".to_owned()
    } else {
        format!("./{rel}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(relative(&p("/a/b"), &p("/a/b/c.js")), "c.js");
        assert_eq!(
            relative(&p("/a/b/out"), &p("/a/brand.js")),
            "../../brand.js"
        );
        assert_eq!(relative(&p("/a/b"), &p("/a/b")), "");
        assert_eq!(relative_specifier(&p("/a/b"), &p("/a/b/c.js")), "./c.js");
        assert_eq!(
            relative_specifier(&p("/a/b/out"), &p("/a/brand.js")),
            "../../brand.js"
        );
        assert_eq!(relative_specifier(&p("/a/b/out"), &p("/a/b")), "..");
    }

    #[test]
    fn resolve_normalizes() {
        assert_eq!(
            resolve(Path::new("/a/b"), "../c/./d"),
            PathBuf::from("/a/c/d")
        );
        assert_eq!(resolve(Path::new("/a/b"), "/x/y"), PathBuf::from("/x/y"));
    }
}

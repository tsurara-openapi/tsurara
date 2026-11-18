//! Writes generated files to the output directory (design.md 7.2).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tsurara_backend::{Markers, OutputFile};

#[derive(Debug, thiserror::Error)]
#[error("{}: {source}", path.display())]
pub struct WriteError {
    path: PathBuf,
    #[source]
    source: std::io::Error,
}

/// Paths relative to the output root.
#[derive(Debug, Default)]
pub struct Stats {
    pub written: Vec<String>,
    pub unchanged: Vec<String>,
    pub deleted: Vec<String>,
}

/// Writes `files` under `root`. Files whose content is unchanged are not rewritten, so their
/// modification times are kept. Files that start with the backend's generated-file marker but were not
/// produced this time are deleted, with their sidecars; other files are left alone.
pub fn write_output(
    root: &Path,
    files: &[OutputFile],
    markers: &Markers,
) -> Result<Stats, WriteError> {
    let mut stats = Stats::default();
    let mut produced = HashSet::new();
    for file in files {
        let path = output_path(root, &file.path)?;
        produced.insert(path.clone());
        if std::fs::read(&path).is_ok_and(|old| old == file.contents.as_bytes()) {
            stats.unchanged.push(file.path.clone());
            continue;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| err(dir, source))?;
        }
        std::fs::write(&path, &file.contents).map_err(|source| err(&path, source))?;
        stats.written.push(file.path.clone());
    }
    let mut existing = Vec::new();
    collect_files(root, &mut existing)?;
    for path in existing {
        if produced.contains(&path) || !is_generated(&path, &markers.generated) {
            continue;
        }
        std::fs::remove_file(&path).map_err(|source| err(&path, source))?;
        for ext in &markers.sidecars {
            let mut sidecar = path.clone().into_os_string();
            sidecar.push(ext);
            match std::fs::remove_file(&sidecar) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(err(Path::new(&sidecar), e));
                }
                _ => {}
            }
        }
        let rel = path.strip_prefix(root).unwrap_or(&path);
        stats.deleted.push(rel.to_string_lossy().replace('\\', "/"));
        remove_empty_parents(root, &path);
    }
    Ok(stats)
}

/// A difference between generated files and the files on disk, by path relative to the output root.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difference {
    /// The file exists with different contents.
    Changed(String),
    /// The file was not written.
    Missing(String),
    /// A generated file that would be deleted, because it is no longer produced.
    Stale(String),
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Difference::Changed(p) => write!(f, "changed: {p}"),
            Difference::Missing(p) => write!(f, "missing: {p}"),
            Difference::Stale(p) => write!(f, "stale: {p}"),
        }
    }
}

/// What `write_output` would change, without writing anything.
pub fn diff_output(
    root: &Path,
    files: &[OutputFile],
    markers: &Markers,
) -> Result<Vec<Difference>, WriteError> {
    let mut out = Vec::new();
    let mut produced = HashSet::new();
    for file in files {
        let path = output_path(root, &file.path)?;
        produced.insert(path.clone());
        match std::fs::read(&path) {
            Ok(old) if old == file.contents.as_bytes() => {}
            Ok(_) => out.push(Difference::Changed(file.path.clone())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push(Difference::Missing(file.path.clone()))
            }
            Err(source) => return Err(err(&path, source)),
        }
    }
    let mut existing = Vec::new();
    collect_files(root, &mut existing)?;
    for path in existing {
        if produced.contains(&path) || !is_generated(&path, &markers.generated) {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path);
        out.push(Difference::Stale(rel.to_string_lossy().replace('\\', "/")));
    }
    out.sort();
    Ok(out)
}

/// The place of a generated file under `root`. Paths come from backends and plugins, which may run
/// outside tsurara, so a path that could leave the output directory is refused: it must be relative,
/// with `/` between non-empty segments that are not `.` or `..` and hold no `\\` or `:`.
fn output_path(root: &Path, rel: &str) -> Result<PathBuf, WriteError> {
    let valid = !rel.is_empty()
        && rel
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != ".." && !s.contains(['\\', ':']));
    if !valid {
        return Err(err(
            &root.join(rel),
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{rel:?} is not a relative path inside the output directory"),
            ),
        ));
    }
    Ok(root.join(rel))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), WriteError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(err(dir, source)),
    };
    for entry in entries {
        let entry = entry.map_err(|source| err(dir, source))?;
        let path = entry.path();
        let ty = entry.file_type().map_err(|source| err(&path, source))?;
        if ty.is_dir() {
            collect_files(&path, out)?;
        } else if ty.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Whether the file starts with the backend's generated-file marker.
fn is_generated(path: &Path, marker: &str) -> bool {
    use std::io::Read;
    let mut buf = vec![0; marker.len()];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok_and(|()| buf == marker.as_bytes())
}

/// Removes directories left empty by deletions, up to (not including) `root`.
fn remove_empty_parents(root: &Path, file: &Path) {
    let mut dir = file.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

fn err(path: &Path, source: std::io::Error) -> WriteError {
    WriteError {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsurara_backend::Markers;

    /// A backend whose files are marked with a `#` comment and have `.uid` sidecars, like GDScript.
    fn hash_markers() -> Markers {
        Markers {
            generated: "# Generated by tsurara. Do not edit.".to_owned(),
            version: None,
            sidecars: vec![".uid".to_owned()],
        }
    }

    fn file(path: &str, contents: &str) -> OutputFile {
        OutputFile {
            path: path.to_owned(),
            contents: contents.to_owned(),
        }
    }

    #[test]
    fn stale_files_are_found_by_the_backend_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("old.gd"),
            "# Generated by tsurara. Do not edit.\n",
        )
        .unwrap();
        std::fs::write(root.join("old.gd.uid"), "uid://abc\n").unwrap();
        std::fs::write(root.join("mine.gd"), "extends Node\n").unwrap();
        std::fs::write(
            root.join("ts.gd"),
            "// Generated by tsurara. Do not edit.\n",
        )
        .unwrap();
        let files = [file("pet.gd", "# Generated by tsurara. Do not edit.\n")];
        let markers = hash_markers();
        let diffs = diff_output(root, &files, &markers).unwrap();
        assert_eq!(
            diffs,
            [
                Difference::Missing("pet.gd".into()),
                Difference::Stale("old.gd".into())
            ]
        );
        let stats = write_output(root, &files, &markers).unwrap();
        assert_eq!(stats.deleted, ["old.gd"]);
        assert!(!root.join("old.gd").exists());
        assert!(!root.join("old.gd.uid").exists());
        assert!(root.join("mine.gd").exists());
        assert!(root.join("ts.gd").exists());
        assert!(root.join("pet.gd").exists());
    }

    #[test]
    fn paths_outside_the_output_directory_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("out");
        let markers = hash_markers();
        let outside = if cfg!(windows) {
            "C:\\x.gd"
        } else {
            "/tmp/x.gd"
        };
        for path in [
            "../x.gd",
            "a/../../x.gd",
            "a/../b.gd",
            outside,
            "",
            "a//b.gd",
            "./a.gd",
        ] {
            let files = [file(path, "x")];
            let e = write_output(&root, &files, &markers).unwrap_err();
            assert!(e.to_string().contains("not a relative path"), "{path}: {e}");
            assert!(diff_output(&root, &files, &markers).is_err(), "{path}");
        }
        assert!(!tmp.path().join("x.gd").exists());
        let stats = write_output(&root, &[file("a/b.gd", "x")], &markers).unwrap();
        assert_eq!(stats.written, ["a/b.gd"]);
    }
}

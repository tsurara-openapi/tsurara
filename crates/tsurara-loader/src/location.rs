use std::path::{Component, Path, PathBuf};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum LocationError {
    #[error("unsupported URL scheme `{0}`")]
    UnsupportedScheme(String),
    #[error("`{0}` cannot be converted to a file URL")]
    InvalidPath(PathBuf),
}

/// Turns an input string into a URL. `http(s)://` and `file://` are used as is; anything else is
/// treated as a path and resolved against `base_dir`.
pub fn resolve_input(input: &str, base_dir: &Path) -> Result<Url, LocationError> {
    if let Some(url) = parse_url(input) {
        return match url.scheme() {
            "http" | "https" | "file" => Ok(url),
            other => Err(LocationError::UnsupportedScheme(other.to_owned())),
        };
    }
    let path = Path::new(input);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };
    let absolute = if joined.is_absolute() {
        joined
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&joined))
            .unwrap_or(joined)
    };
    let normalized = normalize(&absolute);
    Url::from_file_path(&normalized).map_err(|()| LocationError::InvalidPath(normalized))
}

/// Only treats it as a URL when the scheme has two or more characters, so Windows `C:\` is not taken for a scheme.
fn parse_url(input: &str) -> Option<Url> {
    let colon = input.find(':')?;
    if colon < 2 {
        return None;
    }
    Url::parse(input).ok()
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_kept() {
        let base = Path::new("/base");
        assert_eq!(
            resolve_input("https://example.com/a.yaml", base)
                .unwrap()
                .as_str(),
            "https://example.com/a.yaml"
        );
        assert_eq!(
            resolve_input("file:///x/a.yaml", base).unwrap().as_str(),
            "file:///x/a.yaml"
        );
        assert!(resolve_input("ftp://example.com/a", base).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn paths_are_resolved_from_base() {
        let base = Path::new("/base/dir");
        assert_eq!(
            resolve_input("specs/../api.yaml", base).unwrap().as_str(),
            "file:///base/dir/api.yaml"
        );
        assert_eq!(
            resolve_input("/abs/api.yaml", base).unwrap().as_str(),
            "file:///abs/api.yaml"
        );
        assert_eq!(
            resolve_input("my spec.yaml", base).unwrap().as_str(),
            "file:///base/dir/my%20spec.yaml"
        );
    }
}

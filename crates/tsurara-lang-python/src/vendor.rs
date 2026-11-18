//! `runtime: vendor`: a copy of `tsurara_client` in the subpackage `_runtime`, which
//! generated modules import relatively instead of the package.

use crate::HEADER;
use tsurara_backend::OutputFile;

/// The subpackage of the copy.
pub const DIR: &str = "_runtime";

/// Rewrites the runtime imports of `files` and adds the copy. The runtime imports its own modules
/// relatively, so the copy is the source with the marker.
pub fn vendor(files: &mut Vec<OutputFile>) {
    for f in files.iter_mut() {
        let depth = f.path.matches('/').count();
        let package = format!("{}{DIR}", ".".repeat(depth + 1));
        f.contents = f
            .contents
            .lines()
            .map(|line| {
                let indent = &line[..line.len() - line.trim_start().len()];
                let rewritten = line
                    .trim_start()
                    .strip_prefix("from tsurara_client")
                    .filter(|rest| rest.starts_with([' ', '.']))
                    .map(|rest| format!("{indent}from {package}{rest}"));
                rewritten.unwrap_or_else(|| line.to_owned()) + "\n"
            })
            .collect();
    }
    for (path, source) in tsurara_runtimes::PYTHON {
        // `py.typed` belongs to the top-level package.
        if !path.ends_with(".py") {
            continue;
        }
        files.push(OutputFile {
            path: format!("{DIR}/{path}"),
            contents: format!("{HEADER}\n# {}\n\n{source}", tsurara_runtimes::NOTICE),
        });
    }
}

//! `runtime: vendor`: a copy of the runtime in `internal/tsurara` of the output, which
//! generated code imports through `runtime_module` (the import path of that directory, computed
//! from the user's go.mod by the configuration).

use crate::HEADER;
use tsurara_backend::OutputFile;

/// The directory of the copy, relative to the output root.
pub const DIR: &str = "internal/tsurara";

/// The import path of the runtime's package.
const UPSTREAM: &str = "github.com/tsurara-openapi/tsurara/runtimes/go";

/// Adds the copy, its imports of itself rewritten to `runtime_module`.
pub fn vendor(files: &mut Vec<OutputFile>, runtime_module: &str) {
    for (path, source) in tsurara_runtimes::GO {
        let source = source.replace(&format!("\"{UPSTREAM}"), &format!("\"{runtime_module}"));
        files.push(OutputFile {
            path: format!("{DIR}/{path}"),
            contents: format!("{HEADER}\n// {}\n\n{source}", tsurara_runtimes::NOTICE),
        });
    }
}

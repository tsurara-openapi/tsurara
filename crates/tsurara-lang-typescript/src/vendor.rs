//! `runtime: vendor`: a copy of `@tsurara/client` in `_runtime/`, which generated files
//! import with relative specifiers instead of the package.

use crate::emit::{CLIENT_RANGE_PREFIX, HEADER, relative};
use crate::options::TypeScriptOptions;
use tsurara_backend::OutputFile;
use tsurara_sem::print::string_literal;

/// The directory of the copy, relative to the output root.
pub const DIR: &str = "_runtime";

/// Rewrites the runtime imports of `files` and adds the copy.
pub fn vendor(files: &mut Vec<OutputFile>, opts: &TypeScriptOptions) {
    let ext = opts.import_extension.suffix();
    let testing = format!("{}/testing", opts.client_module);
    // Types alone (`operations: false`) import nothing of the runtime: there is nothing to copy.
    let package = &string_literal(&opts.client_module)[..opts.client_module.len() + 1];
    if !files.iter().any(|f| f.contents.contains(package)) {
        return;
    }
    for f in files.iter_mut() {
        let to = |target: &str| string_literal(&relative(&f.path, &format!("{DIR}/{target}"), ext));
        let rewritten = f
            .contents
            .replace(&string_literal(&testing), &to("testing.ts"))
            .replace(&string_literal(&opts.client_module), &to("index.ts"));
        // The copy is the version: there is no package to compare with.
        f.contents = rewritten
            .lines()
            .filter(|l| !l.starts_with(CLIENT_RANGE_PREFIX))
            .map(|l| format!("{l}\n"))
            .collect();
    }
    for (path, source) in tsurara_runtimes::TYPESCRIPT {
        files.push(OutputFile {
            path: format!("{DIR}/{path}"),
            contents: format!(
                "{HEADER}\n// {}\n\n{}",
                tsurara_runtimes::NOTICE,
                with_extension(source, ext)
            ),
        });
    }
}

/// The runtime's relative imports end in `.ts`; the copy uses the target's import extension.
fn with_extension(source: &str, ext: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(i) = rest.find("'./") {
        let (head, tail) = rest.split_at(i);
        out.push_str(head);
        let end = tail[1..]
            .find('\'')
            .map(|e| e + 1)
            .unwrap_or(tail.len() - 1);
        let spec = &tail[1..end];
        match spec.strip_suffix(".ts") {
            Some(stem) => {
                out.push('\'');
                out.push_str(stem);
                out.push_str(ext);
            }
            None => {
                out.push('\'');
                out.push_str(spec);
            }
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::with_extension;

    #[test]
    fn rewrites_relative_imports() {
        let src = "import { a } from './a.ts';\nexport * from './b.ts';\nconst s = 'x';\n";
        assert_eq!(
            with_extension(src, ".js"),
            "import { a } from './a.js';\nexport * from './b.js';\nconst s = 'x';\n"
        );
        assert_eq!(with_extension(src, ""), src.replace(".ts'", "'"));
    }
}

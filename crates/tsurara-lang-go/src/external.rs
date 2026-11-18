//! External types for Go (`x-tsurara-type` entries for `go`): a type such as `uuid.UUID` or
//! `*big.Int`, with the import path its package comes from, checked so that a definition cannot put
//! code into the output.

use tsurara_sem::ir::External;

/// Why the entry cannot be printed, if it cannot.
pub fn check(ext: &External) -> Result<(), String> {
    let code = &ext.code;
    let invalid = |why: &str| Err(format!("the type {code:?} is not allowed: {why}"));
    if code.trim().is_empty() {
        return invalid("it is empty");
    }
    let mut depth = 0i32;
    for c in code.chars() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth < 0 {
                    return invalid("brackets do not match");
                }
            }
            '.' | '*' | ',' | ' ' => {}
            c if c.is_alphanumeric() || c == '_' => {}
            c => return invalid(&format!("{c:?} is not allowed")),
        }
    }
    if depth != 0 {
        return invalid("brackets do not match");
    }
    if let Some(import) = &ext.import {
        let path = &import.from;
        let ok = !path.is_empty()
            && !path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
            && path
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._~/-".contains(c));
        if !ok {
            return Err(format!("the import path {path:?} is not allowed"));
        }
        if alias(ext).is_none() && import.name != *code {
            return Err(format!("the import name {:?} is not allowed", import.name));
        }
    }
    Ok(())
}

/// The name an import is given: `import.name` when it is an identifier other than the type (the
/// name defaults to the type, which needs no alias).
pub fn alias(ext: &External) -> Option<&str> {
    let import = ext.import.as_ref()?;
    let name = import.name.as_str();
    let identifier = name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    (identifier && name != ext.code).then_some(name)
}

//! Checks external types (`x-tsurara-type` entries for `python`) before they are printed: the type must
//! be an annotation of a restricted grammar and the import a module path, so that a definition cannot
//! put code into the output (design.md 11).

use tsurara_sem::ir::External;

/// Why the external type cannot be printed, if it cannot.
pub fn check(e: &External) -> Result<(), String> {
    check_annotation(&e.code)?;
    if let Some(i) = &e.import {
        if !is_identifier(&i.name) {
            return Err(format!("the import name {:?} is not an identifier", i.name));
        }
        // `uuid`, `pkg.mod`, `.sibling`, or `.` (the package itself).
        let module = i.from.trim_start_matches('.');
        let package_itself = module.is_empty() && i.from.starts_with('.');
        if !package_itself && !module.split('.').all(is_identifier) {
            return Err(format!("the import {:?} is not a module path", i.from));
        }
    }
    Ok(())
}

/// Names, dotted names, subscripts (`list[int]`), `|`, string literals without escapes, and numbers.
/// No calls, `=`, `:`, `;`, comments or line breaks.
fn check_annotation(code: &str) -> Result<(), String> {
    let invalid = |why: &str| Err(format!("the type {code:?} is not allowed: {why}"));
    if code.trim().is_empty() {
        return invalid("it is empty");
    }
    let mut depth = 0usize;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '.' | ',' | '|' | '-' => {}
            '[' => depth += 1,
            ']' => {
                if depth == 0 {
                    return invalid("brackets do not match");
                }
                depth -= 1;
            }
            '\'' | '"' => loop {
                match chars.next() {
                    Some(q) if q == c => break,
                    Some('\\') => return invalid("string literals cannot have escapes"),
                    Some(x) if x.is_control() => return invalid("line breaks are not allowed"),
                    Some(_) => {}
                    None => return invalid("a string literal is not closed"),
                }
            },
            c if c.is_alphanumeric() || c == '_' => {}
            c if c.is_control() => return invalid("line breaks are not allowed"),
            c => return invalid(&format!("{c:?} is not allowed")),
        }
    }
    if depth != 0 {
        return invalid("brackets do not match");
    }
    Ok(())
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

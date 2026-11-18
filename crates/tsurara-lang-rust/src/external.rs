//! External types for Rust (`x-tsurara-type` entries for `rust`): a type path with generics, checked so
//! that a definition cannot put code into the output.

/// Why the type cannot be printed, if it cannot.
pub fn check(code: &str) -> Result<(), String> {
    let invalid = |why: &str| Err(format!("the type {code:?} is not allowed: {why}"));
    if code.trim().is_empty() {
        return invalid("it is empty");
    }
    let mut depth = 0i32;
    for c in code.chars() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => {
                depth -= 1;
                if depth < 0 {
                    return invalid("brackets do not match");
                }
            }
            ':' | ',' | ' ' | '&' | '\'' => {}
            c if c.is_alphanumeric() || c == '_' => {}
            c => return invalid(&format!("{c:?} is not allowed")),
        }
    }
    if depth != 0 {
        return invalid("brackets do not match");
    }
    Ok(())
}

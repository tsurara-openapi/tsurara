//! Checks external types (`x-tsurara-type`, `typeMappings`, `overrides`) before they are printed: the
//! type must be a type expression of a restricted grammar, so that a definition cannot put code
//! into the output (design.md 11).

use tsurara_sem::ir::External;

/// Why the external type cannot be printed, if it cannot.
pub fn check(e: &External) -> Result<(), String> {
    check_type(&e.code)?;
    if let Some(i) = &e.import {
        if !is_identifier(&i.name) {
            return Err(format!("the import name {:?} is not an identifier", i.name));
        }
        if i.from.is_empty()
            || i.from
                .chars()
                .any(|c| matches!(c, '\'' | '"' | '`' | '\\') || c.is_control())
        {
            return Err(format!("the import {:?} is not a module specifier", i.from));
        }
    }
    Ok(())
}

/// Identifiers, qualified names, type arguments, arrays, tuples, parentheses, `|`, `&`, `keyof` and
/// the like (identifiers), string and template literals without escapes, and numbers. No `;`,
/// braces, `=`, comments or line breaks outside literals.
fn check_type(code: &str) -> Result<(), String> {
    let invalid = |why: &str| Err(format!("the type {code:?} is not allowed: {why}"));
    if code.trim().is_empty() {
        return invalid("it is empty");
    }
    let mut stack = Vec::new();
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '.' | ',' | '|' | '&' | '-' => {}
            '<' | '[' | '(' => stack.push(c),
            '>' | ']' | ')' => {
                let open = match c {
                    '>' => '<',
                    ']' => '[',
                    _ => '(',
                };
                if stack.pop() != Some(open) {
                    return invalid("brackets do not match");
                }
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
            // A template literal type: text, and `${...}` holding a type expression.
            '`' => loop {
                match chars.next() {
                    Some('`') => break,
                    Some('$') if chars.peek() == Some(&'{') => {
                        chars.next();
                        let mut inner = String::new();
                        loop {
                            match chars.next() {
                                Some('}') => break,
                                Some(x) => inner.push(x),
                                None => return invalid("a template literal is not closed"),
                            }
                        }
                        check_type(&inner)?;
                    }
                    Some('\\') => return invalid("template literals cannot have escapes"),
                    Some(x) if x.is_control() => return invalid("line breaks are not allowed"),
                    Some(_) => {}
                    None => return invalid("a template literal is not closed"),
                }
            },
            '/' if matches!(chars.peek(), Some('/' | '*')) => {
                return invalid("comments are not allowed");
            }
            c if c.is_alphanumeric() || c == '_' || c == '$' => {}
            c if c.is_control() => return invalid("line breaks are not allowed"),
            c => return invalid(&format!("{c:?} is not allowed")),
        }
    }
    if !stack.is_empty() {
        return invalid("brackets do not match");
    }
    Ok(())
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

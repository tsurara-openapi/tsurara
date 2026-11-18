//! Text of the definition that printers write into comments.
//!
//! The definition is untrusted: a line break inside a `//`, `#` or `///` comment starts a line of
//! code. Each language ends lines at different characters (JavaScript also at U+2028 and U+2029,
//! Python at a lone `\r`), so printers split and flatten text with these functions, which treat every
//! line break of any target language as one.

/// Whether `c` ends a line in a language tsurara generates, or is treated as such by its tools:
/// `\n`, `\r`, vertical tab, form feed, the information separators `\x1c` to `\x1e`, NEL, and the
/// Unicode line and paragraph separators.
pub fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Whether `c` is a control character (C0, DEL, C1) or a line break.
pub fn is_control(c: char) -> bool {
    c.is_control() || is_line_break(c)
}

/// Whether a key of the definition (a path, a method, a media type, a channel) holds a character that
/// generated code cannot print as it is. Such keys are rejected by the model.
pub fn has_control(s: &str) -> bool {
    s.chars().any(is_control)
}

/// The lines of `text`, split at every line break (`\r\n` counts once). Other control characters
/// become spaces, except tabs, which printers handle themselves.
pub fn lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if is_line_break(c) {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push(std::mem::take(&mut line));
        } else if c.is_control() && c != '\t' {
            line.push(' ');
        } else {
            line.push(c);
        }
    }
    out.push(line);
    out
}

/// `text` on one line: line breaks and control characters (tabs included) become spaces.
pub fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| if is_control(c) { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_at_every_line_break() {
        assert_eq!(
            lines("a\nb\r\nc\rd\u{2028}e\u{2029}f\u{85}g\u{b}h\u{c}i\u{1c}j"),
            ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]
        );
        assert_eq!(lines("a\r\n\r\nb"), ["a", "", "b"]);
        assert_eq!(lines(""), [""]);
    }

    #[test]
    fn other_control_characters_become_spaces() {
        assert_eq!(lines("a\u{0}b\u{7f}c\td"), ["a b c\td"]);
        assert_eq!(single_line("a\r\nb\tc\u{2028}d"), "a  b c d");
    }

    #[test]
    fn keys_with_control_characters_are_found() {
        for s in ["/a\n", "\r", "a\u{2028}", "\u{0}", "\u{85}", "a\tb"] {
            assert!(has_control(s), "{s:?}");
        }
        for s in ["/a*/b", "\"\"\"", "`\\${x}", "日本語"] {
            assert!(!has_control(s), "{s:?}");
        }
    }
}

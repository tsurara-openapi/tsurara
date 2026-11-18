//! A document printer shared by every backend's printer (design.md 18.8). Backends write final,
//! deterministic layout with it instead of calling external formatters.
//!
//! Published for backends outside this repository, with one version for all tsurara crates,
//! released with the CLI and moving with `semVersion`. Stable: the document type and its
//! constructors, and the printing functions.
//!
//! Widths are counted in bytes of UTF-8 (the same as characters for ASCII).
//!
//! A document is text, line breaks, nesting and groups, in the style of Wadler's "prettier printer".
//! A group is written flat (its soft lines as spaces or nothing) or broken (its soft lines as
//! newlines). Besides the classic rule (flat when it fits in the rest of the line), a group can be
//! flat when its own flat width is within a limit, wherever it starts; printers whose layout rules
//! depend on the size of a construct, not on the column, use that.

use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq)]
pub enum Doc {
    Nil,
    Text(Cow<'static, str>),
    /// A newline that is a newline in both modes.
    HardLine,
    /// `flat` when the enclosing group is flat, `broken` when it is broken.
    FlatOr(Box<Doc>, Box<Doc>),
    /// Indents the lines that start inside it by `n` more spaces.
    Nest(usize, Box<Doc>),
    Concat(Vec<Doc>),
    Group(Fit, Box<Doc>),
}

/// When a group is written flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Its flat width fits in the rest of the line (`width` of `render` minus the column).
    Remaining,
    /// Its flat width is at most this many characters.
    Within(usize),
    /// Never: always broken.
    Never,
}

impl Doc {
    pub fn text(s: impl Into<Cow<'static, str>>) -> Doc {
        Doc::Text(s.into())
    }

    /// A soft line: `flat` when the group is flat, a newline when it is broken.
    pub fn line(flat: &'static str) -> Doc {
        Doc::FlatOr(Box::new(Doc::text(flat)), Box::new(Doc::HardLine))
    }

    pub fn flat_or(flat: Doc, broken: Doc) -> Doc {
        Doc::FlatOr(Box::new(flat), Box::new(broken))
    }

    pub fn nest(n: usize, doc: Doc) -> Doc {
        Doc::Nest(n, Box::new(doc))
    }

    pub fn group(fit: Fit, doc: Doc) -> Doc {
        Doc::Group(fit, Box::new(doc))
    }

    pub fn concat(docs: impl IntoIterator<Item = Doc>) -> Doc {
        Doc::Concat(docs.into_iter().collect())
    }

    /// The width of the document written flat, in bytes of UTF-8, or `None` if it cannot be flat: it
    /// contains a hard line, or a nested group that would break on its own (a `Within` group wider
    /// than its limit, or a `Never` group).
    pub fn flat_width(&self) -> Option<usize> {
        match self {
            Doc::Nil => Some(0),
            Doc::Text(t) => {
                if t.contains('\n') {
                    None
                } else {
                    Some(t.len())
                }
            }
            Doc::HardLine => None,
            Doc::FlatOr(flat, _) => flat.flat_width(),
            Doc::Nest(_, d) | Doc::Group(Fit::Remaining, d) => d.flat_width(),
            Doc::Group(Fit::Within(n), d) => d.flat_width().filter(|w| w <= n),
            Doc::Group(Fit::Never, _) => None,
            Doc::Concat(ds) => ds.iter().try_fold(0, |acc, d| Some(acc + d.flat_width()?)),
        }
    }
}

impl From<&'static str> for Doc {
    fn from(s: &'static str) -> Doc {
        Doc::text(s)
    }
}

impl From<String> for Doc {
    fn from(s: String) -> Doc {
        Doc::text(s)
    }
}

/// Writes `doc`; `width` is the line width for `Fit::Remaining` groups.
pub fn render(doc: &Doc, width: usize) -> String {
    let mut r = Renderer {
        out: String::new(),
        column: 0,
        pending_indent: None,
        width,
    };
    r.write(doc, 0, false);
    r.out
}

struct Renderer {
    out: String,
    column: usize,
    /// Indentation of a line that has started but has no text yet, written with its first text, so
    /// that empty lines have no trailing spaces.
    pending_indent: Option<usize>,
    width: usize,
}

impl Renderer {
    fn write(&mut self, doc: &Doc, indent: usize, flat: bool) {
        match doc {
            Doc::Nil => {}
            Doc::Text(t) => {
                if t.is_empty() {
                    return;
                }
                if let Some(n) = self.pending_indent.take() {
                    self.out.extend(std::iter::repeat_n(' ', n));
                    self.column = n;
                }
                self.out.push_str(t);
                match t.rfind('\n') {
                    Some(i) => self.column = t.len() - i - 1,
                    None => self.column += t.len(),
                }
            }
            Doc::HardLine => {
                self.out.push('\n');
                self.column = 0;
                self.pending_indent = Some(indent);
            }
            Doc::FlatOr(f, b) => {
                if flat {
                    self.write(f, indent, true);
                } else {
                    self.write(b, indent, false);
                }
            }
            Doc::Nest(n, d) => self.write(d, indent + n, flat),
            Doc::Concat(ds) => {
                for d in ds {
                    self.write(d, indent, flat);
                }
            }
            Doc::Group(fit, d) => {
                let flat = flat || {
                    let start = match self.pending_indent {
                        Some(n) => n,
                        None => self.column,
                    };
                    match fit {
                        Fit::Remaining => d.flat_width().is_some_and(|w| start + w <= self.width),
                        _ => doc.flat_width().is_some(),
                    }
                };
                self.write(d, indent, flat);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(fields: &[&'static str], fit: Fit) -> Doc {
        let mut inner = vec![Doc::line(" ")];
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                inner.push(Doc::flat_or(
                    Doc::text("; "),
                    Doc::concat([Doc::text(";"), Doc::HardLine]),
                ));
            }
            inner.push(Doc::text(*f));
        }
        inner.push(Doc::flat_or(Doc::text(" "), Doc::concat([Doc::text(";")])));
        Doc::group(
            fit,
            Doc::concat([
                Doc::text("{"),
                Doc::nest(2, Doc::concat(inner)),
                Doc::line(""),
                Doc::text("}"),
            ]),
        )
    }

    #[test]
    fn groups_are_flat_within_their_limit_and_broken_otherwise() {
        assert_eq!(
            render(&object(&["a: string", "b: number"], Fit::Within(30)), 80),
            "{ a: string; b: number }"
        );
        assert_eq!(
            render(&object(&["a: string", "b: number"], Fit::Within(10)), 80),
            "{\n  a: string;\n  b: number;\n}"
        );
        assert_eq!(
            render(&object(&["a: string"], Fit::Never), 80),
            "{\n  a: string;\n}"
        );
    }

    #[test]
    fn remaining_groups_depend_on_the_column() {
        let doc = Doc::concat([
            Doc::text("x".repeat(70)),
            object(&["a: string"], Fit::Remaining),
        ]);
        assert!(render(&doc, 80).contains('\n'));
        let doc = Doc::concat([Doc::text("x"), object(&["a: string"], Fit::Remaining)]);
        assert!(!render(&doc, 80).contains('\n'));
    }

    #[test]
    fn hard_lines_break_their_groups_and_empty_lines_have_no_indent() {
        let doc = Doc::group(
            Fit::Within(100),
            Doc::nest(
                2,
                Doc::concat([Doc::text("a"), Doc::HardLine, Doc::HardLine, Doc::text("b")]),
            ),
        );
        assert_eq!(render(&doc, 80), "a\n\n  b");
    }

    #[test]
    fn nested_groups_are_decided_on_their_own() {
        let inner = object(&["x: number"], Fit::Within(20));
        let outer = Doc::group(
            Fit::Never,
            Doc::concat([
                Doc::text("{"),
                Doc::nest(
                    2,
                    Doc::concat([Doc::HardLine, Doc::text("p: "), inner, Doc::text(";")]),
                ),
                Doc::HardLine,
                Doc::text("}"),
            ]),
        );
        assert_eq!(render(&outer, 80), "{\n  p: { x: number };\n}");
    }
}

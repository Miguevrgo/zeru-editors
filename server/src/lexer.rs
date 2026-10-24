//! The lexer: Zeru source into tokens with byte spans, comments included,
//! never failing. What the compiler would reject becomes an `Illegal` token
//! carrying the message.

pub const KEYWORDS: [&str; 26] = [
    "fn", "var", "const", "if", "else", "import", "while", "return", "for", "in", "break",
    "continue", "as", "pub", "struct", "enum", "trait", "str", "match", "default", "self", "true",
    "false", "asm", "volatile", "None",
];

pub const PRIMITIVES: [&str; 14] = [
    "i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize", "f32", "f64", "bool",
    "void",
];

/// The operators, longest first so the first match is the longest one.
const OPERATORS: [&str; 51] = [
    ">>=", "<<=", "+%=", "-%=", "*%=", "..=", "==", "!=", "<=", ">=", "&&", "||", "<<", ">>", "+=",
    "-=", "*=", "/=", "%=", "^=", "&=", "|=", "+%", "-%", "*%", "=>", "::", "..", "=", "+", "-",
    "*", "/", "%", "<", ">", "!", "&", "|", "^", "(", ")", "{", "}", "[", "]", ":", ";", ",", ".",
    "?",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn touches(self, offset: usize) -> bool {
        self.start <= offset && offset <= self.end
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Ident,
    Keyword,
    Int,
    Float,
    Str,
    Comment,
    Op,
    Illegal(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: Kind,
    pub span: Span,
}

pub fn lex(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        let rest = &text[at..];
        let c = bytes[at];
        let after_dot = tokens
            .iter()
            .rev()
            .find(|t: &&Token| t.kind != Kind::Comment)
            .is_some_and(|t| &text[t.span.start..t.span.end] == ".");
        let kind = if c.is_ascii_whitespace() {
            at += 1;
            continue;
        } else if rest.starts_with("//") {
            at += rest.find('\n').unwrap_or(rest.len());
            Kind::Comment
        } else if let Some(body) = rest.strip_prefix("/*") {
            match body.find("*/") {
                Some(end) => {
                    at += end + 4;
                    Kind::Comment
                }
                None => {
                    at += 2;
                    Kind::Illegal("Unterminated block comment".into())
                }
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            at += rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            match KEYWORDS.contains(&&text[start..at]) {
                true => Kind::Keyword,
                false => Kind::Ident,
            }
        } else if c.is_ascii_digit() {
            let (len, kind) = number(rest, after_dot);
            at += len;
            kind
        } else if c == b'"' || c == b'`' {
            let close = c as char;
            let mut chars = rest.char_indices().skip(1);
            let mut kind = match close {
                '"' => Kind::Illegal("Unterminated String".into()),
                _ => Kind::Illegal("Unterminated raw string".into()),
            };
            at = text.len();
            while let Some((i, ch)) = chars.next() {
                if ch == '\\' && close == '"' {
                    chars.next();
                } else if ch == close {
                    at = start + i + 1;
                    kind = Kind::Str;
                    break;
                }
            }
            kind
        } else if c == b'\'' {
            let skip = match rest[1..].starts_with('\\') {
                true => 3,
                false => 1 + rest[1..].chars().next().map_or(0, char::len_utf8),
            };
            match rest.get(skip..).and_then(|r| r.find(['\'', '\n'])) {
                Some(i) if bytes[start + skip + i] == b'\'' => {
                    at += skip + i + 1;
                    Kind::Str
                }
                _ => {
                    at += 1;
                    Kind::Illegal("Unterminated character literal".into())
                }
            }
        } else {
            match OPERATORS.iter().find(|op| rest.starts_with(**op)) {
                Some(op) => {
                    at += op.len();
                    Kind::Op
                }
                None => {
                    let ch = rest.chars().next().unwrap_or_default();
                    at += ch.len_utf8();
                    Kind::Illegal(format!("Unexpected character '{ch}'"))
                }
            }
        };
        tokens.push(Token {
            kind,
            span: Span::new(start, at),
        });
    }
    tokens
}

/// A number literal at the start of `rest`: its length and kind. After a
/// `.` only digits are read, so `t.0.1` indexes twice.
fn number(rest: &str, after_dot: bool) -> (usize, Kind) {
    let digits = |s: &str| {
        s.find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(s.len())
    };
    if after_dot {
        let len = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        return (len, Kind::Int);
    }
    let mut len = digits(rest);
    let mut kind = Kind::Int;
    let fraction = rest[len..].strip_prefix('.');
    if fraction.is_some_and(|f| f.starts_with(|c: char| c.is_ascii_digit())) {
        len += 1 + digits(&rest[len + 1..]);
        kind = Kind::Float;
    }
    let exponent = rest[..len].ends_with(['e', 'E']) && rest[len..].starts_with(['-', '+']);
    if exponent && !rest.starts_with("0x") {
        len += 1 + digits(&rest[len + 1..]);
        kind = Kind::Float;
    }
    (len, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(Kind, &str)> {
        lex(text)
            .into_iter()
            .map(|t| (t.kind, &text[t.span.start..t.span.end]))
            .collect()
    }

    #[test]
    fn operators_take_the_longest_match() {
        let found = kinds("a >>= b +% c :: d .. e");
        let ops: Vec<&str> = found
            .iter()
            .filter(|(k, _)| *k == Kind::Op)
            .map(|(_, s)| *s)
            .collect();
        assert_eq!(ops, [">>=", "+%", "::", ".."]);
    }

    #[test]
    fn literals_and_comments() {
        let found = kinds(
            "var s = \"a\\\"b\"; // note\n/* block */ 'x' '\\n' `raw\nline` 2.5e-3 t.0.1 0..n",
        );
        let expected = [
            (Kind::Keyword, "var"),
            (Kind::Ident, "s"),
            (Kind::Op, "="),
            (Kind::Str, "\"a\\\"b\""),
            (Kind::Op, ";"),
            (Kind::Comment, "// note"),
            (Kind::Comment, "/* block */"),
            (Kind::Str, "'x'"),
            (Kind::Str, "'\\n'"),
            (Kind::Str, "`raw\nline`"),
            (Kind::Float, "2.5e-3"),
            (Kind::Ident, "t"),
            (Kind::Op, "."),
            (Kind::Int, "0"),
            (Kind::Op, "."),
            (Kind::Int, "1"),
            (Kind::Int, "0"),
            (Kind::Op, ".."),
            (Kind::Ident, "n"),
        ];
        assert_eq!(found, expected);
    }

    #[test]
    fn broken_text_lexes_to_the_end() {
        for (text, message) in [
            ("\"open", "Unterminated String"),
            ("/* open", "Unterminated block comment"),
            ("a $ b", "Unexpected character '$'"),
        ] {
            let illegal = lex(text).into_iter().find_map(|t| match t.kind {
                Kind::Illegal(m) => Some(m),
                _ => None,
            });
            assert!(illegal.is_some_and(|m| m.starts_with(message)), "{text}");
        }
    }
}

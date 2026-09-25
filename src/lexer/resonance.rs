//! Klang v2 resonance tokens (Phase 2).
//!
//! Standalone v2 lexer: it does not touch the v1 tokenizer in
//! [`crate::parser`]. Spans are exact byte offsets into the source.
//! Grammar choice (documented): `|>` is ordinary left-to-right
//! application and never bypasses type checking; `dep=` lexes as two
//! tokens (`Dep`, `Eq`) so `dep = x` and `dep=x` share one grammar.

use super::{LexError, Result};

/// One v2 lexical token kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// `?` — Dissonant qualifier.
    Question,
    /// `!` — Harmonic qualifier.
    Bang,
    /// `!=` — not-equal (so `!` lexing never swallows it).
    NotEq,
    /// `|>` — pipeline (left-to-right application).
    PipeGt,
    /// A lone `|` (kept explicit so the parser reports it).
    Pipe,
    /// `flow` keyword.
    Flow,
    /// `echo` keyword.
    Echo,
    /// `listen` keyword.
    Listen,
    /// `tune` keyword.
    Tune,
    /// `verify` keyword.
    Verify,
    /// `dep` keyword (pairs with `Eq` to form `dep=`).
    Dep,
    /// `=` (also the second half of `dep=`).
    Eq,
    /// `->` — return-type arrow (flows, echo fns).
    Arrow,
    /// `>` (bodies; kept so Flow bodies lex without a second pass).
    Gt,
    /// `>=`.
    GtEq,
    /// `<`.
    Lt,
    /// `<=`.
    LtEq,
    /// `+`.
    Plus,
    /// `-` (also first half of `->`; arrow takes precedence).
    Minus,
    /// `*`.
    Star,
    /// `/` (line comments `//...` still skip before this).
    Slash,
    /// `;`.
    Semi,
    /// `.`.
    Dot,
    /// `(`.
    LParen,
    /// `)`.
    RParen,
    /// `{`.
    LBrace,
    /// `}`.
    RBrace,
    /// `,`.
    Comma,
    /// `:`.
    Colon,
    /// Bare identifier (keywords above take precedence).
    Ident(String),
    /// Decimal integer literal.
    IntLit(i64),
    /// String literal (escape-decoded value).
    StrLit(String),
    /// End of input (zero-width at `source.len()`).
    Eof,
}

/// One v2 token with its exact byte span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Token kind.
    pub kind: TokenKind,
    /// Inclusive byte offset where the token starts.
    pub start: usize,
    /// Exclusive byte offset where the token ends.
    pub end: usize,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn err(offset: usize, message: &str) -> LexError {
    LexError {
        offset,
        message: message.to_string(),
    }
}

/// Lex `source` into v2 tokens.
///
/// Skips whitespace and `//` line comments. Returns the first structured
/// error on malformed input (unexpected character, unterminated string,
/// integer out of range). The trailing [`TokenKind::Eof`] is always present
/// on success.
pub fn lex(source: &str) -> Result<Vec<Token>> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut toks: Vec<Token> = Vec::new();
    let mut i: usize = 0;
    while i < n {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < n && bytes[i + 1] == b'/' {
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        match c {
            '?' => {
                toks.push(Token {
                    kind: TokenKind::Question,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '!' => {
                if i + 1 < n && bytes[i + 1] == b'=' {
                    toks.push(Token {
                        kind: TokenKind::NotEq,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    toks.push(Token {
                        kind: TokenKind::Bang,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '|' => {
                if i + 1 < n && bytes[i + 1] == b'>' {
                    toks.push(Token {
                        kind: TokenKind::PipeGt,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    toks.push(Token {
                        kind: TokenKind::Pipe,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '=' => {
                toks.push(Token {
                    kind: TokenKind::Eq,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '-' => {
                if i + 1 < n && bytes[i + 1] == b'>' {
                    toks.push(Token {
                        kind: TokenKind::Arrow,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    toks.push(Token {
                        kind: TokenKind::Minus,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '>' => {
                if i + 1 < n && bytes[i + 1] == b'=' {
                    toks.push(Token {
                        kind: TokenKind::GtEq,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    toks.push(Token {
                        kind: TokenKind::Gt,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '<' => {
                if i + 1 < n && bytes[i + 1] == b'=' {
                    toks.push(Token {
                        kind: TokenKind::LtEq,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    toks.push(Token {
                        kind: TokenKind::Lt,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '+' => {
                toks.push(Token {
                    kind: TokenKind::Plus,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '*' => {
                toks.push(Token {
                    kind: TokenKind::Star,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '/' => {
                toks.push(Token {
                    kind: TokenKind::Slash,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            ';' => {
                toks.push(Token {
                    kind: TokenKind::Semi,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '.' => {
                toks.push(Token {
                    kind: TokenKind::Dot,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '(' => {
                toks.push(Token {
                    kind: TokenKind::LParen,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            ')' => {
                toks.push(Token {
                    kind: TokenKind::RParen,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '{' => {
                toks.push(Token {
                    kind: TokenKind::LBrace,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '}' => {
                toks.push(Token {
                    kind: TokenKind::RBrace,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            ',' => {
                toks.push(Token {
                    kind: TokenKind::Comma,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            ':' => {
                toks.push(Token {
                    kind: TokenKind::Colon,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '"' => {
                let mut j = i + 1;
                let mut val = String::new();
                let mut closed = false;
                while j < n {
                    if bytes[j] == b'"' {
                        closed = true;
                        break;
                    }
                    if bytes[j] == b'\\' && j + 1 < n {
                        match bytes[j + 1] {
                            b'n' => val.push('\n'),
                            b't' => val.push('\t'),
                            b'"' => val.push('"'),
                            b'\\' => val.push('\\'),
                            other => {
                                val.push('\\');
                                val.push(other as char);
                            }
                        }
                        j += 2;
                    } else {
                        let ch = source[j..].chars().next().unwrap_or('\u{FFFD}');
                        val.push(ch);
                        j += ch.len_utf8();
                    }
                }
                if !closed {
                    return Err(err(start, "unterminated string literal"));
                }
                toks.push(Token {
                    kind: TokenKind::StrLit(val),
                    start,
                    end: (j + 1).min(source.len()),
                });
                i = j + 1;
            }
            _ if c.is_ascii_digit() => {
                let mut j = i;
                while j < n && (bytes[j] as char).is_ascii_digit() {
                    j += 1;
                }
                match source[i..j].parse::<i64>() {
                    Ok(num) => toks.push(Token {
                        kind: TokenKind::IntLit(num),
                        start,
                        end: j,
                    }),
                    Err(_) => return Err(err(start, "integer literal out of range")),
                }
                i = j;
            }
            _ if is_ident_start(c) => {
                let mut j = i;
                while j < n && is_ident_char(bytes[j] as char) {
                    j += 1;
                }
                let word = &source[i..j];
                let kind = match word {
                    "flow" => TokenKind::Flow,
                    "echo" => TokenKind::Echo,
                    "listen" => TokenKind::Listen,
                    "tune" => TokenKind::Tune,
                    "verify" => TokenKind::Verify,
                    "dep" => TokenKind::Dep,
                    _ => TokenKind::Ident(word.to_string()),
                };
                toks.push(Token {
                    kind,
                    start,
                    end: j,
                });
                i = j;
            }
            _ => {
                return Err(err(start, &format!("unexpected character `{c}`")));
            }
        }
    }
    toks.push(Token {
        kind: TokenKind::Eof,
        start: n,
        end: n,
    });
    Ok(toks)
}

//! Hand-written recursive descent parser with structural identity.
//!
//! Identity mechanism: a stack of scopes, one per entered parent. The next
//! child index lives inside the innermost scope, so the full path (not a bare
//! counter) is the identity. A child is always allocated while its parent
//! scope entry is still on the stack.

//! v2 Flow (`flow`) and Echo (`echo`/`listen`) parsers live in [`flow`]
//! and [`echo`]; they run on [`crate::lexer`] tokens and leave the v1
//! grammar below untouched.

/// v2 Echo parsing (`echo fn`, `listen`).
pub mod echo;
/// v2 Flow parsing (explicit `dep=` environment).
pub mod flow;
/// v2 full program parsing (schemas, echo fns, functions with resonance types).
pub mod v2;

use crate::ast::{
    AssignStmt, AssignTarget, Block, BreakStmt, ContinueStmt, Effect, EnumDecl, EnumVariant, Expr,
    ForInStmt, ForRangeStmt, FunctionDecl, IfStmt, LetStmt, MatchArm, ModDecl, NodeId, Param,
    PrintStmt, Program, ReturnStmt, Stmt, StructDecl, TaskGroup, TryCatchStmt, WhileStmt,
};
use crate::diagnostics::Diagnostic;

/// One `pub`-prefixed declaration (keyword routed after `pub`).
enum DeclItem {
    Struct(StructDecl),
    Enum(EnumDecl),
    Fn(FunctionDecl),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Fn,
    Let,
    Return,
    Try,
    Catch,
    TaskGroup,
    Spawn,
    Await,
    Throws,
    Async,
    Cancel,
    If,
    Else,
    Print,
    True,
    False,
    While,
    For,
    In,
    Struct,
    Break,
    Continue,
    Import,
    Enum,
    Match,
    Mod,
    Pub,
    Arrow,
    FatArrow,
    ColonColon,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Eq,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Bang,
    AndAnd,
    OrOr,
    Amp,
    Pipe,
    Invalid,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Semi,
    Comma,
    Colon,
    Dot,
    DotDot,
    Ident(String),
    IntLit(i64),
    FloatLit(u64),
    StrLit(String),
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub start: usize,
    pub end: usize,
}

fn is_ident_char(c: char) -> bool {
    // ASCII only: byte-indexed slicing below would split multibyte chars.
    c.is_ascii_alphanumeric() || c == '_'
}

/// One comment kept as trivia by the lexer (S3): `//...` to end of line
/// or `/* ... */`. The parser still ignores comments (behavior unchanged);
/// the formatter re-attaches them to the canonical output so `fmt` never
/// deletes them. `start`/`end` are byte offsets into the lexed source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub start: usize,
    pub end: usize,
}

impl Comment {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }
}

fn tokenize(
    source: &str,
) -> (
    Vec<Token>,
    Vec<(usize, usize, String)>,
    Vec<Comment>,
) {
    let bytes = source.as_bytes();
    let mut toks: Vec<Token> = Vec::new();
    let mut lex_errs: Vec<(usize, usize, String)> = Vec::new();
    let mut comments: Vec<Comment> = Vec::new();
    let mut i: usize = 0;
    let n = bytes.len();
    while i < n {
        let c = bytes[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < n && bytes[i + 1] == b'/' {
            let cstart = i;
            i += 2;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            comments.push(Comment { start: cstart, end: i });
            continue;
        }
        if c == '/' && i + 1 < n && bytes[i + 1] == b'*' {
            let cstart = i;
            i += 2;
            let mut closed = false;
            while i + 1 < n {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    closed = true;
                    i += 2;
                    break;
                }
                i += 1;
            }
            if !closed {
                // Unterminated block comment: do not silently discard the
                // rest of the file. Emit a diagnostic via an Invalid token
                // and stop lexing.
                lex_errs.push((cstart, n, "unterminated block comment".to_string()));
                toks.push(Token {
                    kind: TokenKind::Invalid,
                    start: cstart,
                    end: n,
                });
                i = n;
            } else {
                comments.push(Comment { start: cstart, end: i });
            }
            continue;
        }
        let start = i;
        match c {
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
            '=' => {
                if i + 1 < n && bytes[i + 1] == b'=' {
                    toks.push(Token {
                        kind: TokenKind::EqEq,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else if i + 1 < n && bytes[i + 1] == b'>' {
                    toks.push(Token {
                        kind: TokenKind::FatArrow,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    toks.push(Token {
                        kind: TokenKind::Eq,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
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
            '+' => {
                toks.push(Token {
                    kind: TokenKind::Plus,
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
            ',' => {
                toks.push(Token {
                    kind: TokenKind::Comma,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            ':' => {
                if i + 1 < n && bytes[i + 1] == b':' {
                    toks.push(Token {
                        kind: TokenKind::ColonColon,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    toks.push(Token {
                        kind: TokenKind::Colon,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '.' => {
                if i + 1 < n && bytes[i + 1] == b'.' {
                    toks.push(Token {
                        kind: TokenKind::DotDot,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    toks.push(Token {
                        kind: TokenKind::Dot,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '&' => {
                if i + 1 < n && bytes[i + 1] == b'&' {
                    toks.push(Token {
                        kind: TokenKind::AndAnd,
                        start,
                        end: i + 2,
                    });
                    i += 2;
                } else {
                    // Lone `&` is not an operator: surface it so the
                    // parser errors instead of silently dropping it.
                    toks.push(Token {
                        kind: TokenKind::Amp,
                        start,
                        end: i + 1,
                    });
                    i += 1;
                }
            }
            '|' => {
                if i + 1 < n && bytes[i + 1] == b'|' {
                    toks.push(Token {
                        kind: TokenKind::OrOr,
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
            '%' => {
                toks.push(Token {
                    kind: TokenKind::Percent,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '[' => {
                toks.push(Token {
                    kind: TokenKind::LBracket,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            ']' => {
                toks.push(Token {
                    kind: TokenKind::RBracket,
                    start,
                    end: i + 1,
                });
                i += 1;
            }
            '"' => {
                // String literal with simple escapes. Decodes UTF-8 by chars
                // (not bytes) so non-ASCII literals are preserved.
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
                            b'n' => {
                                val.push('\n');
                                j += 2;
                            }
                            b'r' => {
                                val.push('\r');
                                j += 2;
                            }
                            b't' => {
                                val.push('\t');
                                j += 2;
                            }
                            b'"' => {
                                val.push('"');
                                j += 2;
                            }
                            b'\\' => {
                                val.push('\\');
                                j += 2;
                            }
                            b'u' => {
                                // Standard `\uXXXX` Unicode escape: 4 hex
                                // digits decode to the codepoint. Malformed
                                // sequences fall through to the literal
                                // pass-through below (same as other unknown
                                // escapes), never a panic or silent drop.
                                if j + 5 < n
                                    && bytes[j + 2].is_ascii_hexdigit()
                                    && bytes[j + 3].is_ascii_hexdigit()
                                    && bytes[j + 4].is_ascii_hexdigit()
                                    && bytes[j + 5].is_ascii_hexdigit()
                                {
                                    let hex = &source[j + 2..j + 6];
                                    if let Ok(cp) = u32::from_str_radix(hex, 16) {
                                        if let Some(ch) = char::from_u32(cp) {
                                            val.push(ch);
                                            j += 6;
                                        } else {
                                            val.push('\\');
                                            val.push('u');
                                            j += 2;
                                        }
                                    } else {
                                        val.push('\\');
                                        val.push('u');
                                        j += 2;
                                    }
                                } else {
                                    val.push('\\');
                                    val.push('u');
                                    j += 2;
                                }
                            }
                            other => {
                                val.push('\\');
                                val.push(other as char);
                                j += 2;
                            }
                        }
                    } else {
                        // Decode one full UTF-8 char to stay on boundaries.
                        let ch = source[j..].chars().next().unwrap_or('\u{FFFD}');
                        val.push(ch);
                        j += ch.len_utf8();
                    }
                }
                if !closed {
                    lex_errs.push((start, n, "unterminated string literal".to_string()));
                    toks.push(Token {
                        kind: TokenKind::Invalid,
                        start,
                        end: n,
                    });
                    i = n;
                } else {
                    toks.push(Token {
                        kind: TokenKind::StrLit(val),
                        start,
                        end: (j + 1).min(source.len()),
                    });
                    i = j + 1;
                }
            }
            _ if c.is_ascii_digit() => {
                let mut j = i;
                while j < n && (bytes[j] as char).is_ascii_digit() {
                    j += 1;
                }
                // Float when `digits.digits` (but not `0..10`: dot must be
                // followed by a digit).
                if j + 1 < n && bytes[j] == b'.' && (bytes[j + 1] as char).is_ascii_digit() {
                    let mut k = j + 1;
                    while k < n && (bytes[k] as char).is_ascii_digit() {
                        k += 1;
                    }
                    match source[i..k].parse::<f64>() {
                        Ok(num) => toks.push(Token {
                            kind: TokenKind::FloatLit(num.to_bits()),
                            start,
                            end: k,
                        }),
                        Err(_) => {
                            lex_errs.push((start, k, "invalid float literal".to_string()));
                            toks.push(Token {
                                kind: TokenKind::Invalid,
                                start,
                                end: k,
                            });
                        }
                    }
                    i = k;
                } else {
                    match source[i..j].parse::<i64>() {
                        Ok(num) => toks.push(Token {
                            kind: TokenKind::IntLit(num),
                            start,
                            end: j,
                        }),
                        Err(_) => {
                            // Overflow beyond i64 (and thus i32): do not
                            // silently become 0. Surface as invalid so the
                            // parser reports instead of evaluating to 0.
                            lex_errs.push((start, j, "integer literal out of range".to_string()));
                            toks.push(Token {
                                kind: TokenKind::Invalid,
                                start,
                                end: j,
                            });
                        }
                    }
                    i = j;
                }
            }
            _ if c.is_ascii_alphabetic() || c == '_' => {
                let mut j = i;
                while j < n && is_ident_char(bytes[j] as char) {
                    j += 1;
                }
                let word = &source[i..j];
                let kind = match word {
                    "fn" => TokenKind::Fn,
                    "let" => TokenKind::Let,
                    "return" => TokenKind::Return,
                    "try" => TokenKind::Try,
                    "catch" => TokenKind::Catch,
                    "task_group" => TokenKind::TaskGroup,
                    "spawn" => TokenKind::Spawn,
                    "await" => TokenKind::Await,
                    "throws" => TokenKind::Throws,
                    "async" => TokenKind::Async,
                    "cancel" => TokenKind::Cancel,
                    "if" => TokenKind::If,
                    "else" => TokenKind::Else,
                    "print" => TokenKind::Print,
                    "true" => TokenKind::True,
                    "false" => TokenKind::False,
                    "while" => TokenKind::While,
                    "for" => TokenKind::For,
                    "in" => TokenKind::In,
                    "struct" => TokenKind::Struct,
                    "enum" => TokenKind::Enum,
                    "match" => TokenKind::Match,
                    "mod" => TokenKind::Mod,
                    "pub" => TokenKind::Pub,
                    "break" => TokenKind::Break,
                    "continue" => TokenKind::Continue,
                    "import" => TokenKind::Import,
                    other => TokenKind::Ident(other.to_string()),
                };
                toks.push(Token {
                    kind,
                    start,
                    end: j,
                });
                i = j;
            }
            _ => {
                // Anything else (stray punctuation, non-ASCII): emit an
                // explicit token so the parser reports it instead of the
                // byte loop silently skipping it. Advance one full char
                // to stay on char boundaries.
                let len = source[i..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                toks.push(Token {
                    kind: TokenKind::Invalid,
                    start,
                    end: i + len,
                });
                i += len;
            }
        }
    }
    toks.push(Token {
        kind: TokenKind::Eof,
        start: n,
        end: n,
    });
    (toks, lex_errs, comments)
}

/// Lex `source` keeping comments as trivia (S3): tokens (with a trailing
/// `Eof`), comments in source order, and the first lex error as a
/// diagnostic (like [`Parser::new_with_file`]). The parser itself still
/// ignores comments; the formatter uses the trivia to re-attach them.
pub fn lex_with_comments(
    source: &str,
    file: &str,
) -> (Vec<Token>, Vec<Comment>, Option<Diagnostic>) {
    let (tokens, lex_errs, comments) = tokenize(source);
    let lex_error = lex_errs
        .into_iter()
        .next()
        .map(|(s, e, msg)| Diagnostic::parse_error(file, s, e, &msg));
    (tokens, comments, lex_error)
}

#[derive(Debug, Clone)]
struct Scope {
    path: Vec<u32>,
    next: u32,
}

/// Restore per-scope child counters after speculative parsing.
fn restore_scope_next(scopes: &mut [Scope], saved: &[u32]) {
    for (s, n) in scopes.iter_mut().zip(saved.iter()) {
        s.next = *n;
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    scopes: Vec<Scope>,
    file: String,
    lex_error: Option<Diagnostic>,
}

impl Parser {
    pub fn new(source: &str) -> Self {
        Self::new_with_file(source, "input.warden")
    }

    pub fn new_with_file(source: &str, file: &str) -> Self {
        let (tokens, _comments, lex_error) = lex_with_comments(source, file);
        Self {
            tokens,
            pos: 0,
            scopes: vec![Scope {
                path: Vec::new(),
                next: 0,
            }],
            file: file.to_string(),
            lex_error,
        }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn err(&self, start: usize, end: usize, message: &str) -> Diagnostic {
        Diagnostic::parse_error(&self.file, start, end, message)
    }

    fn next_id(&mut self) -> NodeId {
        let scope = self.scopes.last_mut().expect("root scope always present");
        let mut path = scope.path.clone();
        path.push(scope.next);
        scope.next += 1;
        NodeId::new(path)
    }

    fn enter_scope(&mut self, id: &NodeId) {
        self.scopes.push(Scope {
            path: id.path.clone(),
            next: 0,
        });
    }

    fn exit_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn consume_semi_opt(&mut self) {
        if self.peek().kind == TokenKind::Semi {
            self.bump();
        }
    }

    /// Optional leading `pub` on a declaration. Inside a `mod` it marks the
    /// item visible across modules; at top level it is accepted and ignored.
    fn eat_pub(&mut self) -> bool {
        if self.peek().kind == TokenKind::Pub {
            self.bump();
            true
        } else {
            false
        }
    }

    /// One `pub`-prefixed declaration, routed to its parser by the keyword
    /// after `pub` (which the decl parser itself consumes).
    fn parse_pub_decl(&mut self) -> Result<DeclItem, Diagnostic> {
        let next = self.tokens.get(self.pos + 1).map(|t| t.kind.clone());
        match next {
            Some(TokenKind::Struct) => Ok(DeclItem::Struct(self.parse_struct()?)),
            Some(TokenKind::Enum) => Ok(DeclItem::Enum(self.parse_enum()?)),
            _ => Ok(DeclItem::Fn(self.parse_function()?)),
        }
    }

    /// Push one parsed `pub` item into the right list.
    fn push_decl(
        structs: &mut Vec<StructDecl>,
        enums: &mut Vec<EnumDecl>,
        functions: &mut Vec<FunctionDecl>,
        item: DeclItem,
    ) {
        match item {
            DeclItem::Struct(s) => structs.push(s),
            DeclItem::Enum(e) => enums.push(e),
            DeclItem::Fn(f) => functions.push(f),
        }
    }

    /// A type annotation: one identifier with optional `::` segments
    /// (`i32`, `T`, `lexer::Token`) plus optional explicit generic
    /// arguments (`Opt<i32>`, `m::Box<T>`, nested `Opt<Opt<i32>>`).
    /// Joined as written (e.g. `"Opt<i32>"`); resolution in HIR/modules
    /// strips the `<...>` suffix to find the base nominal type. `<` here
    /// is unambiguous (no comparison operator exists in type position),
    /// so this parses greedily with no backtracking — unlike call-site
    /// generics, which need speculative disambiguation.
    fn parse_ty_name(&mut self, what: &str) -> Result<String, Diagnostic> {
        // Closure type annotation: `fn(i32, str) -> i32`. `fn` lexes as
        // its own token, so this is unambiguous in type position.
        if self.peek().kind == TokenKind::Fn {
            self.bump();
            self.expect(&TokenKind::LParen, "`(`")?;
            let mut params = Vec::new();
            if self.peek().kind != TokenKind::RParen {
                loop {
                    params.push(self.parse_ty_name("parameter type")?);
                    if self.peek().kind == TokenKind::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
            self.expect(&TokenKind::Arrow, "`->`")?;
            let ret = self.parse_ty_name("return type")?;
            return Ok(format!("fn({})->{}", params.join(", "), ret));
        }
        let (mut name, _, _) = self.expect_ident(what)?;
        while self.peek().kind == TokenKind::ColonColon {
            self.bump();
            let (seg, _, _) = self.expect_ident(what)?;
            name.push_str("::");
            name.push_str(&seg);
        }
        if self.peek().kind != TokenKind::Lt {
            return Ok(name);
        }
        // Explicit generic arguments: `<` Type (, Type)* `>`, recursive
        // for nesting. At least one argument is required (`Opt<>` is a
        // parse error, not an empty instantiation).
        self.bump();
        if self.peek().kind == TokenKind::Gt {
            let p = self.peek().clone();
            return Err(self.err(p.start, p.end, "expected a type argument"));
        }
        let mut args = Vec::new();
        loop {
            args.push(self.parse_ty_name("type argument")?);
            if self.peek().kind == TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(&TokenKind::Gt, "`>`")?;
        Ok(format!("{name}<{}>", args.join(", ")))
    }

    /// Optional `<T, U, ...>` type-parameter list after a declaration name.
    /// Absent (the common case) yields an empty vec; no new syntax nodes are
    /// created, so structural identity is unaffected.
    fn parse_type_params_opt(&mut self) -> Result<Vec<String>, Diagnostic> {
        if self.peek().kind != TokenKind::Lt {
            return Ok(Vec::new());
        }
        self.bump();
        let mut params = Vec::new();
        if self.peek().kind != TokenKind::Gt {
            loop {
                let (name, _, _) = self.expect_ident("type parameter name")?;
                params.push(name);
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(&TokenKind::Gt, "`>`")?;
        Ok(params)
    }

    fn expect(&mut self, want: &TokenKind, what: &str) -> Result<Token, Diagnostic> {
        let t = self.peek().clone();
        let matches = matches!(
            (want, &t.kind),
            (TokenKind::Fn, TokenKind::Fn)
                | (TokenKind::Let, TokenKind::Let)
                | (TokenKind::Return, TokenKind::Return)
                | (TokenKind::TaskGroup, TokenKind::TaskGroup)
                | (TokenKind::Spawn, TokenKind::Spawn)
                | (TokenKind::Await, TokenKind::Await)
                | (TokenKind::If, TokenKind::If)
                | (TokenKind::Else, TokenKind::Else)
                | (TokenKind::Try, TokenKind::Try)
                | (TokenKind::Catch, TokenKind::Catch)
                | (TokenKind::Print, TokenKind::Print)
                | (TokenKind::While, TokenKind::While)
                | (TokenKind::For, TokenKind::For)
                | (TokenKind::In, TokenKind::In)
                | (TokenKind::Struct, TokenKind::Struct)
                | (TokenKind::Enum, TokenKind::Enum)
                | (TokenKind::Match, TokenKind::Match)
                | (TokenKind::Arrow, TokenKind::Arrow)
                | (TokenKind::FatArrow, TokenKind::FatArrow)
                | (TokenKind::ColonColon, TokenKind::ColonColon)
                | (TokenKind::LParen, TokenKind::LParen)
                | (TokenKind::RParen, TokenKind::RParen)
                | (TokenKind::LBrace, TokenKind::LBrace)
                | (TokenKind::RBrace, TokenKind::RBrace)
                | (TokenKind::LBracket, TokenKind::LBracket)
                | (TokenKind::RBracket, TokenKind::RBracket)
                | (TokenKind::Eq, TokenKind::Eq)
                | (TokenKind::Comma, TokenKind::Comma)
                | (TokenKind::Colon, TokenKind::Colon)
                | (TokenKind::Gt, TokenKind::Gt)
                | (TokenKind::Mod, TokenKind::Mod)
        );
        if matches {
            Ok(self.bump())
        } else {
            Err(self.err(t.start, t.end, &format!("expected {what}")))
        }
    }

    /// Optional `-> type` after a function signature (D4). An omitted arrow
    /// desugars to `-> void` at parse time, so every downstream stage (HIR
    /// sigs, MIR, fmt) sees exactly today's `-> void` programs — except the
    /// `omitted` flag, which the CLI exit-code mapping uses: an entry with
    /// no declared return type always exits 0 (Phase 3b decision). A
    /// `return <expr>` inside such a body then fails as a local `E-TYPE`
    /// against `void` with a fix hint, instead of an `E-PARSE` at the `{`.
    /// `-> ()` stays rejected: `parse_ty_name` only accepts identifiers
    /// (plus `::` segments and generics), so only `void` spells unit.
    fn parse_return_ty_opt(&mut self) -> Result<(String, bool), Diagnostic> {
        if self.peek().kind == TokenKind::Arrow {
            self.bump();
            return Ok((self.parse_ty_name("return type")?, false));
        }
        Ok(("void".to_string(), true))
    }

    fn expect_ident(&mut self, what: &str) -> Result<(String, usize, usize), Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Ident(name) => {
                let name = name.clone();
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok((name, s, e))
            }
            _ => Err(self.err(t.start, t.end, &format!("expected {what}"))),
        }
    }

    pub fn parse_program(&mut self) -> Result<Program, Diagnostic> {
        if let Some(e) = self.lex_error.clone() {
            return Err(e);
        }
        let mut mods = Vec::new();
        let mut enums = Vec::new();
        let mut structs = Vec::new();
        let mut imports = Vec::new();
        let mut selective_imports = Vec::new();
        let mut aliased_imports = Vec::new();
        let mut functions = Vec::new();
        loop {
            match &self.peek().kind {
                TokenKind::Eof => break,
                TokenKind::Invalid => {
                    let p = self.peek().clone();
                    return Err(self.err(p.start, p.end, "invalid token"));
                }
                TokenKind::Dot => {
                    let p = self.peek().clone();
                    return Err(self.err(p.start, p.end, "stray `.`"));
                }
                TokenKind::Import => {
                    let t = self.bump();
                    match &self.peek().kind {
                        TokenKind::StrLit(path) => {
                            let path = path.clone();
                            self.bump();
                            // Contextual `as`: `import "p" as m`. `as` is
                            // not a keyword (it lexes as `Ident`), so a
                            // function literally named `as` still parses —
                            // the alias position only exists here.
                            if let TokenKind::Ident(w) = self.peek().kind.clone() {
                                if w == "as" {
                                    self.bump();
                                    let (alias, _, _) =
                                        self.expect_ident("alias name")?;
                                    self.consume_semi_opt();
                                    aliased_imports.push(crate::ast::AliasedImport {
                                        path,
                                        alias,
                                    });
                                    let _ = t.start;
                                    continue;
                                }
                            }
                            self.consume_semi_opt();
                            imports.push(path);
                        }
                        TokenKind::LBrace => {
                            selective_imports.push(self.parse_selective_import()?);
                            // Selective imports cannot take an alias: an
                            // alias exposes a whole file's API (`import "p"
                            // as m`), and renaming a subset would leave the
                            // subset's helpers unresolvable.
                            if let TokenKind::Ident(w) = self.peek().kind.clone() {
                                if w == "as" {
                                    let p = self.peek().clone();
                                    return Err(self.err(
                                        p.start,
                                        p.end,
                                        "selective imports cannot take an alias: use `import \"path\" as name` for the whole file",
                                    ));
                                }
                            }
                        }
                        _ => {
                            let p = self.peek().clone();
                            return Err(self.err(
                                p.start,
                                p.end,
                                "expected `\"path\"` or `{ names } from \"path\"` after `import`",
                            ));
                        }
                    }
                    let _ = t.start;
                }
                TokenKind::Mod => mods.push(self.parse_mod()?),
                TokenKind::Pub => {
                    let item = self.parse_pub_decl()?;
                    Self::push_decl(&mut structs, &mut enums, &mut functions, item);
                }
                TokenKind::Struct => structs.push(self.parse_struct()?),
                TokenKind::Enum => enums.push(self.parse_enum()?),
                _ => functions.push(self.parse_function()?),
            }
        }
        Ok(Program {
            mods,
            enums,
            structs,
            imports,
            selective_imports,
            aliased_imports,
            alias_scopes: Vec::new(),
            functions,
        })
    }

    /// `import { a, b } from "path"`: selective import of named top-level
    /// items. Called just after `import` when the next token is `{`.
    /// `from` is a plain identifier (never a keyword), so this is
    /// unambiguous. Empty braces and duplicates-in-one-list are `E-PARSE`
    /// and dedupe respectively — both loud-or-harmless, never silent.
    fn parse_selective_import(&mut self) -> Result<crate::ast::SelectiveImport, Diagnostic> {
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut names = Vec::new();
        loop {
            match &self.peek().kind {
                TokenKind::RBrace => {
                    self.bump();
                    break;
                }
                _ => {}
            }
            let (name, _, _) = self.expect_ident("imported name")?;
            if !names.contains(&name) {
                names.push(name);
            }
            match &self.peek().kind {
                TokenKind::Comma => {
                    self.bump();
                }
                TokenKind::RBrace => {}
                _ => {
                    let p = self.peek().clone();
                    return Err(self.err(p.start, p.end, "expected `,` or `}`"));
                }
            }
        }
        if names.is_empty() {
            let p = self.peek().clone();
            return Err(self.err(p.start, p.end, "expected at least one name in `{ }`"));
        }
        let (from, _, _) = self.expect_ident("`from`")?;
        if from != "from" {
            let p = self.peek().clone();
            return Err(self.err(p.start, p.end, "expected `from \"path\"` after `{ names }`"));
        }
        let path = match &self.peek().kind {
            TokenKind::StrLit(path) => {
                let path = path.clone();
                self.bump();
                path
            }
            _ => {
                let p = self.peek().clone();
                return Err(self.err(p.start, p.end, "expected `\"path\"` after `from`"));
            }
        };
        self.consume_semi_opt();
        Ok(crate::ast::SelectiveImport { names, path })
    }

    fn parse_mod(&mut self) -> Result<ModDecl, Diagnostic> {
        self.expect(&TokenKind::Mod, "`mod`")?;
        let (name, _, _) = self.expect_ident("module name")?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let id = self.next_id();
        // Members generate their ids while the module scope is entered, so
        // every member path extends the module path (structural identity).
        self.enter_scope(&id);
        let mut structs = Vec::new();
        let mut enums = Vec::new();
        let mut functions = Vec::new();
        loop {
            match &self.peek().kind {
                TokenKind::RBrace => {
                    self.bump();
                    break;
                }
                TokenKind::Eof => {
                    let p = self.peek().clone();
                    self.exit_scope();
                    return Err(self.err(p.start, p.end, "expected `}`"));
                }
                TokenKind::Pub => {
                    let item = self.parse_pub_decl()?;
                    Self::push_decl(&mut structs, &mut enums, &mut functions, item);
                }
                TokenKind::Struct => structs.push(self.parse_struct()?),
                TokenKind::Enum => enums.push(self.parse_enum()?),
                _ => functions.push(self.parse_function()?),
            }
        }
        self.exit_scope();
        Ok(ModDecl {
            id,
            name,
            structs,
            enums,
            functions,
        })
    }

    fn parse_struct(&mut self) -> Result<StructDecl, Diagnostic> {
        let is_pub = self.eat_pub();
        self.expect(&TokenKind::Struct, "`struct`")?;
        let (name, _, _) = self.expect_ident("struct name")?;
        let type_params = self.parse_type_params_opt()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let id = self.next_id();
        let mut fields = Vec::new();
        loop {
            if self.peek().kind == TokenKind::RBrace {
                self.bump();
                break;
            }
            if self.peek().kind == TokenKind::Eof {
                let p = self.peek().clone();
                return Err(self.err(p.start, p.end, "expected `}`"));
            }
            let (fname, _, _) = self.expect_ident("field name")?;
            self.expect(&TokenKind::Colon, "`:`")?;
            let fty = self.parse_ty_name("field type")?;
            fields.push(Param {
                name: fname,
                ty: fty,
            });
            self.consume_comma_opt();
        }
        Ok(StructDecl {
            id,
            name,
            is_pub,
            type_params,
            fields,
        })
    }

    fn parse_enum(&mut self) -> Result<EnumDecl, Diagnostic> {
        let is_pub = self.eat_pub();
        self.expect(&TokenKind::Enum, "`enum`")?;
        let (name, _, _) = self.expect_ident("enum name")?;
        let type_params = self.parse_type_params_opt()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let id = self.next_id();
        let mut variants = Vec::new();
        loop {
            if self.peek().kind == TokenKind::RBrace {
                self.bump();
                break;
            }
            if self.peek().kind == TokenKind::Eof {
                let p = self.peek().clone();
                return Err(self.err(p.start, p.end, "expected `}`"));
            }
            let (vname, _, _) = self.expect_ident("variant name")?;
            let mut fields = Vec::new();
            if self.peek().kind == TokenKind::LParen {
                self.bump();
                if self.peek().kind != TokenKind::RParen {
                    // Tuple payloads (`V(i32, str)`) vs named payloads
                    // (`V(x: i32, y: str)`): decided by the FIRST field
                    // only, then enforced for the rest (mixing is
                    // `E-PARSE` — a silent mixed reading would bind the
                    // wrong shape). Tuple fields synthesize `f0..fN`
                    // names matching the positional storage; payload
                    // names are inert (binding is positional), so this
                    // is behavior-preserving.
                    let mut tuple_shape: Option<bool> = None;
                    loop {
                        let is_named = match &self.peek().kind {
                            TokenKind::Ident(_) => matches!(
                                self.tokens.get(self.pos + 1).map(|t| &t.kind),
                                Some(TokenKind::Colon)
                            ),
                            _ => false,
                        };
                        if tuple_shape.is_none() {
                            tuple_shape = Some(!is_named);
                        } else if tuple_shape != Some(!is_named) {
                            let p = self.peek().clone();
                            return Err(self.err(
                                p.start,
                                p.end,
                                "cannot mix named and tuple payload fields in one variant",
                            ));
                        }
                        if is_named {
                            let (fname, _, _) = self.expect_ident("field name")?;
                            self.expect(&TokenKind::Colon, "`:`")?;
                            let fty = self.parse_ty_name("field type")?;
                            fields.push(Param {
                                name: fname,
                                ty: fty,
                            });
                        } else {
                            let fty = self.parse_ty_name("field type")?;
                            let fname = format!("f{}", fields.len());
                            fields.push(Param { name: fname, ty: fty });
                        }
                        if self.peek().kind == TokenKind::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RParen, "`)`")?;
            }
            variants.push(EnumVariant {
                name: vname,
                fields,
            });
            self.consume_comma_opt();
        }
        Ok(EnumDecl {
            id,
            name,
            is_pub,
            type_params,
            variants,
        })
    }

    fn consume_comma_opt(&mut self) {
        if self.peek().kind == TokenKind::Comma {
            self.bump();
        }
    }

    fn parse_function(&mut self) -> Result<FunctionDecl, Diagnostic> {
        let is_pub = self.eat_pub();
        self.expect(&TokenKind::Fn, "`fn`")?;
        let (name, ns, ne) = self.expect_ident("function name")?;
        let type_params = self.parse_type_params_opt()?;
        self.expect(&TokenKind::LParen, "`(`")?;
        let params = self.parse_params()?;
        self.expect(&TokenKind::RParen, "`)`")?;
        let (return_ty, return_ty_omitted) = self.parse_return_ty_opt()?;
        let mut effects = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::Throws => {
                    self.bump();
                    effects.push(Effect::Throws);
                }
                TokenKind::Async => {
                    self.bump();
                    effects.push(Effect::Async);
                }
                TokenKind::Cancel => {
                    self.bump();
                    effects.push(Effect::Cancel);
                }
                _ => break,
            }
        }
        self.expect(&TokenKind::LBrace, "`{`")?;
        let fn_id = self.next_id();
        self.enter_scope(&fn_id);
        let blk_id = self.next_id();
        self.enter_scope(&blk_id);
        let mut stmts = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                _ => stmts.push(self.parse_stmt()?),
            }
        }
        let end_tok = self.peek().clone();
        match end_tok.kind {
            TokenKind::RBrace => {
                self.bump();
            }
            _ => {
                let e = self.err(end_tok.start, end_tok.end, "expected `}`");
                self.exit_scope();
                self.exit_scope();
                return Err(e);
            }
        }
        self.exit_scope();
        self.exit_scope();
        Ok(FunctionDecl {
            id: fn_id,
            name,
            name_span: (ns, ne),
            is_pub,
            type_params,
            params,
            return_ty,
            return_ty_omitted,
            effects,
            body: Block { id: blk_id, stmts },
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, Diagnostic> {
        let mut params = Vec::new();
        if self.peek().kind == TokenKind::RParen {
            return Ok(params);
        }
        loop {
            let (name, _, _) = self.expect_ident("parameter name")?;
            self.expect(&TokenKind::Colon, "`:`")?;
            let ty = self.parse_ty_name("parameter type")?;
            params.push(Param { name, ty });
            if self.peek().kind == TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        Ok(params)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        match self.peek().kind {
            TokenKind::Let => Ok(Stmt::Let(self.parse_let()?)),
            TokenKind::Return => Ok(Stmt::Return(self.parse_return()?)),
            TokenKind::TaskGroup => Ok(Stmt::TaskGroup(self.parse_task_group()?)),
            TokenKind::If => Ok(Stmt::If(self.parse_if()?)),
            TokenKind::Print => Ok(Stmt::Print(self.parse_print()?)),
            TokenKind::While => Ok(Stmt::While(self.parse_while()?)),
            TokenKind::Try => Ok(Stmt::TryCatch(self.parse_try()?)),
            TokenKind::For => self.parse_for(),
            TokenKind::Break => {
                let _ = self.bump();
                self.consume_semi_opt();
                Ok(Stmt::Break(BreakStmt { id: self.next_id() }))
            }
            TokenKind::Continue => {
                let _ = self.bump();
                self.consume_semi_opt();
                Ok(Stmt::Continue(ContinueStmt { id: self.next_id() }))
            }
            _ => {
                // `x = ...` / `a[i] = ...` / `p.f = ...` or an expression.
                if let Some(assign) = self.try_parse_assign()? {
                    return Ok(Stmt::Assign(assign));
                }
                let e = self.parse_expr()?;
                self.consume_semi_opt();
                Ok(Stmt::Expr(e))
            }
        }
    }

    /// Speculative assignment parse: returns `Ok(None)` when this is not an
    /// assignment. Restores the token position on speculation failure (any
    /// `NodeId`s allocated meanwhile are skipped, which keeps identity
    /// unique and parent-prefixed).
    fn try_parse_assign(&mut self) -> Result<Option<AssignStmt>, Diagnostic> {
        let saved = self.pos;
        let scopes_next: Vec<u32> = self.scopes.iter().map(|s| s.next).collect();
        let target = match self.parse_assign_target() {
            Ok(t) => t,
            Err(_) => {
                self.pos = saved;
                restore_scope_next(&mut self.scopes, &scopes_next);
                return Ok(None);
            }
        };
        if self.peek().kind != TokenKind::Eq {
            self.pos = saved;
            restore_scope_next(&mut self.scopes, &scopes_next);
            return Ok(None);
        }
        // `=` but not `==` (tokenizer already splits those).
        self.bump();
        let value = self.parse_expr()?;
        self.consume_semi_opt();
        let id = self.next_id();
        Ok(Some(AssignStmt { id, target, value }))
    }

    /// Speculative explicit-generic call parse (`<T, ...> (` after a bare
    /// function name). Returns `Ok(Some(args))` with the `<...>` consumed
    /// and `(` still pending when the full generic shape is present;
    /// `Ok(None)` (position restored) otherwise, letting the caller fall
    /// back to a plain variable so `<` parses as comparison. Type-argument
    /// failures inside the brackets also fall back rather than erroring,
    /// matching the Rust/C++ backtracking discipline: only a complete
    /// `< Type (, Type)* > (` commits to the generic reading.
    fn try_parse_generic_call_args(&mut self) -> Result<Option<Vec<String>>, Diagnostic> {
        let saved = self.pos;
        let scopes_next: Vec<u32> = self.scopes.iter().map(|s| s.next).collect();
        let restore = |me: &mut Self| {
            me.pos = saved;
            restore_scope_next(&mut me.scopes, &scopes_next);
        };
        // Caller checked `peek == Lt`.
        self.bump();
        // Empty `<>` is not a generic instantiation.
        if self.peek().kind == TokenKind::Gt {
            restore(self);
            return Ok(None);
        }
        let mut args = Vec::new();
        loop {
            match self.parse_ty_name("type argument") {
                Ok(t) => args.push(t),
                Err(_) => {
                    restore(self);
                    return Ok(None);
                }
            }
            if self.peek().kind == TokenKind::Comma {
                self.bump();
            } else {
                break;
            }
        }
        if self.peek().kind != TokenKind::Gt {
            restore(self);
            return Ok(None);
        }
        self.bump();
        if self.peek().kind != TokenKind::LParen {
            restore(self);
            return Ok(None);
        }
        Ok(Some(args))
    }

    fn parse_assign_target(&mut self) -> Result<AssignTarget, Diagnostic> {
        // Base must be a bare variable; index/field chains build on it.
        let (name, _, _) = self.expect_ident("assignment target")?;
        let mut base = Expr::Var {
            id: self.next_id(),
            name,
        };
        loop {
            match self.peek().kind {
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket, "`]`")?;
                    let id = self.next_id();
                    base = Expr::Index {
                        id,
                        base: Box::new(base),
                        index: Box::new(index),
                    };
                }
                TokenKind::Dot => {
                    self.bump();
                    let (field, _, _) = self.expect_ident("field name")?;
                    let id = self.next_id();
                    base = Expr::Field {
                        id,
                        base: Box::new(base),
                        field,
                    };
                }
                _ => break,
            }
        }
        match base {
            Expr::Var { name, .. } => Ok(AssignTarget::Var { name }),
            Expr::Index { base, index, .. } => Ok(AssignTarget::Index { base, index }),
            Expr::Field { base, field, .. } => Ok(AssignTarget::Field { base, field }),
            _ => {
                let p = self.peek().clone();
                Err(self.err(p.start, p.end, "invalid assignment target"))
            }
        }
    }

    fn parse_while(&mut self) -> Result<WhileStmt, Diagnostic> {
        self.expect(&TokenKind::While, "`while`")?;
        let cond = self.parse_expr()?;
        let id = self.next_id();
        self.enter_scope(&id);
        let body = self.parse_block_with_id()?;
        self.exit_scope();
        Ok(WhileStmt { id, cond, body })
    }

    fn parse_try(&mut self) -> Result<TryCatchStmt, Diagnostic> {
        self.expect(&TokenKind::Try, "`try`")?;
        let id = self.next_id();
        self.enter_scope(&id);
        let body = self.parse_block_with_id()?;
        self.expect(&TokenKind::Catch, "`catch`")?;
        let (var, _, _) = self.expect_ident("catch binding name")?;
        let handler = self.parse_block_with_id()?;
        self.exit_scope();
        Ok(TryCatchStmt {
            id,
            var,
            body,
            handler,
        })
    }

    fn parse_for(&mut self) -> Result<Stmt, Diagnostic> {
        self.expect(&TokenKind::For, "`for`")?;
        let (var, _, _) = self.expect_ident("loop variable")?;
        self.expect(&TokenKind::In, "`in`")?;
        let first = self.parse_expr()?;
        if self.peek().kind == TokenKind::DotDot {
            self.bump();
            let end = self.parse_expr()?;
            let id = self.next_id();
            self.enter_scope(&id);
            let body = self.parse_block_with_id()?;
            self.exit_scope();
            Ok(Stmt::ForRange(ForRangeStmt {
                id,
                var,
                start: first,
                end,
                body,
            }))
        } else {
            let id = self.next_id();
            self.enter_scope(&id);
            let body = self.parse_block_with_id()?;
            self.exit_scope();
            Ok(Stmt::ForIn(ForInStmt {
                id,
                var,
                iter: first,
                body,
            }))
        }
    }

    fn parse_let(&mut self) -> Result<LetStmt, Diagnostic> {
        let kw = self.expect(&TokenKind::Let, "`let`")?;
        let (name, ns, ne) = self.expect_ident("binding name")?;
        self.expect(&TokenKind::Eq, "`=`")?;
        let value = self.parse_expr()?;
        self.consume_semi_opt();
        let id = self.next_id();
        Ok(LetStmt {
            id,
            name,
            name_span: (ns, ne),
            value,
            span: (kw.start, ne),
        })
    }

    fn parse_return(&mut self) -> Result<ReturnStmt, Diagnostic> {
        let kw = self.expect(&TokenKind::Return, "`return`")?;
        let value = self.parse_expr()?;
        self.consume_semi_opt();
        let id = self.next_id();
        Ok(ReturnStmt {
            id,
            value,
            span: (kw.start, kw.end),
        })
    }

    fn parse_task_group(&mut self) -> Result<TaskGroup, Diagnostic> {
        self.expect(&TokenKind::TaskGroup, "`task_group`")?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let tg_id = self.next_id();
        self.enter_scope(&tg_id);
        let blk_id = self.next_id();
        self.enter_scope(&blk_id);
        let stmts = self.parse_stmt_list()?;
        let end_tok = self.peek().clone();
        match end_tok.kind {
            TokenKind::RBrace => {
                self.bump();
            }
            _ => {
                let e = self.err(end_tok.start, end_tok.end, "expected `}`");
                self.exit_scope();
                self.exit_scope();
                return Err(e);
            }
        }
        self.exit_scope();
        self.exit_scope();
        Ok(TaskGroup {
            id: tg_id,
            body: Block { id: blk_id, stmts },
        })
    }

    fn parse_stmt_list(&mut self) -> Result<Vec<Stmt>, Diagnostic> {
        let mut stmts = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                _ => stmts.push(self.parse_stmt()?),
            }
        }
        Ok(stmts)
    }

    fn parse_block_with_id(&mut self) -> Result<Block, Diagnostic> {
        self.expect(&TokenKind::LBrace, "`{`")?;
        let blk_id = self.next_id();
        self.enter_scope(&blk_id);
        let stmts = self.parse_stmt_list()?;
        self.expect(&TokenKind::RBrace, "`}`")?;
        self.exit_scope();
        Ok(Block { id: blk_id, stmts })
    }

    fn parse_if(&mut self) -> Result<IfStmt, Diagnostic> {
        self.expect(&TokenKind::If, "`if`")?;
        let cond = self.parse_expr()?;
        let if_id = self.next_id();
        self.enter_scope(&if_id);
        let then_block = self.parse_block_with_id()?;
        let else_block = if self.peek().kind == TokenKind::Else {
            self.bump();
            if self.peek().kind == TokenKind::If {
                // `else if ...` desugars to `else { if ... }` so identity
                // stays uniform (every `If` owns its scope + blocks). The
                // wrapper block is synthetic: no braces are consumed.
                let blk_id = self.next_id();
                self.enter_scope(&blk_id);
                let inner = self.parse_if()?;
                let stmts = vec![Stmt::If(inner)];
                self.exit_scope();
                Some(Block { id: blk_id, stmts })
            } else {
                Some(self.parse_block_with_id()?)
            }
        } else {
            None
        };
        self.exit_scope();
        Ok(IfStmt {
            id: if_id,
            cond,
            then_block,
            else_block,
        })
    }

    fn parse_match(&mut self) -> Result<Expr, Diagnostic> {
        self.expect(&TokenKind::Match, "`match`")?;
        let scrutinee = self.parse_expr()?;
        let open = self.expect(&TokenKind::LBrace, "`{`")?;
        let id = self.next_id();
        let mut arms = Vec::new();
        loop {
            if self.peek().kind == TokenKind::RBrace {
                self.bump();
                break;
            }
            if self.peek().kind == TokenKind::Eof {
                let p = self.peek().clone();
                return Err(self.err(p.start, p.end, "expected `}`"));
            }
            arms.push(self.parse_match_arm()?);
            self.consume_comma_opt();
        }
        if arms.is_empty() {
            return Err(self.err(open.start, open.end, "match needs at least one arm"));
        }
        Ok(Expr::Match {
            id,
            scrutinee: Box::new(scrutinee),
            arms,
        })
    }

    fn parse_match_arm(&mut self) -> Result<MatchArm, Diagnostic> {
        let id = self.next_id();
        let head = self.peek().clone();
        if let TokenKind::Ident(n) = &head.kind {
            if n == "_" {
                self.bump();
                let guard = self.parse_match_guard_opt()?;
                self.expect(&TokenKind::FatArrow, "`=>`")?;
                let (stmts, body) = self.parse_match_arm_body(&id)?;
                return Ok(MatchArm {
                    id,
                    enum_name: None,
                    variant: None,
                    bindings: Vec::new(),
                    stmts,
                    guard,
                    body,
                });
            }
        }
        let (mut enum_name, _, _) = self.expect_ident("enum name")?;
        self.expect(&TokenKind::ColonColon, "`::`")?;
        let (mut variant, _, _) = self.expect_ident("variant name")?;
        if self.peek().kind == TokenKind::ColonColon {
            // `mod::Enum::Variant`: the first two segments name the enum.
            self.bump();
            let (real_variant, _, _) = self.expect_ident("variant name")?;
            enum_name = format!("{enum_name}::{variant}");
            variant = real_variant;
        }
        let mut bindings = Vec::new();
        if self.peek().kind == TokenKind::LParen {
            self.bump();
            if self.peek().kind != TokenKind::RParen {
                loop {
                    bindings.push(self.parse_match_binding()?);
                    if self.peek().kind == TokenKind::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
        }
        let guard = self.parse_match_guard_opt()?;
        self.expect(&TokenKind::FatArrow, "`=>`")?;
        let (stmts, body) = self.parse_match_arm_body(&id)?;
        Ok(MatchArm {
            id,
            enum_name: Some(enum_name),
            variant: Some(variant),
            bindings,
            stmts,
            guard,
            body,
        })
    }

    /// One pattern binding element: `_` (ignore), `name` (bind), or
    /// `Enum::Variant(sub...)` / `m::Enum::Variant(sub...)` (nested
    /// destructure). A bare identifier always binds — even `Nil` — so
    /// nested enum patterns must be qualified (unambiguous, mirroring
    /// the top-level rule that bare names are bindings). The nested
    /// sub-list is optional: `E::V` matches the tag with no bindings.
    fn parse_match_binding(&mut self) -> Result<crate::ast::MatchBinding, Diagnostic> {
        if let TokenKind::Ident(n) = &self.peek().kind {
            if n == "_" {
                self.bump();
                return Ok(crate::ast::MatchBinding::Ignore);
            }
        }
        let (first, _, _) = self.expect_ident("binding name")?;
        let mut segs = vec![first];
        while self.peek().kind == TokenKind::ColonColon {
            self.bump();
            let (s, _, _) = self.expect_ident("variant name")?;
            segs.push(s);
            if segs.len() > 3 {
                let p = self.peek().clone();
                return Err(self.err(p.start, p.end, "pattern path too long"));
            }
        }
        if segs.len() == 1 {
            return Ok(crate::ast::MatchBinding::Bind(segs.pop().expect("one segment")));
        }
        let variant = segs.pop().expect("variant last");
        let enum_name = segs.join("::");
        let mut sub = Vec::new();
        if self.peek().kind == TokenKind::LParen {
            self.bump();
            if self.peek().kind != TokenKind::RParen {
                loop {
                    sub.push(self.parse_match_binding()?);
                    if self.peek().kind == TokenKind::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
        }
        Ok(crate::ast::MatchBinding::Nested {
            enum_name,
            variant,
            bindings: sub,
        })
    }

    /// Optional `if condition` guard between a match pattern and `=>`.
    /// `if` lexes as its own token (never an identifier), so this is
    /// unambiguous: a pattern cannot contain a bare `if`.
    fn parse_match_guard_opt(&mut self) -> Result<Option<Expr>, Diagnostic> {
        if self.peek().kind != TokenKind::If {
            return Ok(None);
        }
        self.bump();
        Ok(Some(self.parse_expr()?))
    }

    /// Arm body after `=>`: either a single expression (with empty setup
    /// statements) or a `{ ... }` block of statements ending in a trailing
    /// expression that yields the arm's value (same statement semantics as
    /// an `if` block body). A `{ ... }` here is never a map literal: map
    /// shapes (`{"a": 1}`, `{a: 1}`) fail inside as `E-PARSE`, exactly as
    /// they did when the braces parsed as a map literal before.
    fn parse_match_arm_body(&mut self, arm_id: &NodeId) -> Result<(Vec<Stmt>, Expr), Diagnostic> {
        if self.peek().kind != TokenKind::LBrace {
            return Ok((Vec::new(), self.parse_expr()?));
        }
        self.bump();
        self.enter_scope(arm_id);
        let mut stmts = Vec::new();
        loop {
            let t = self.peek().clone();
            match &t.kind {
                TokenKind::RBrace => {
                    self.exit_scope();
                    return Err(self.err(
                        t.start,
                        t.end,
                        "match arm block must end with an expression",
                    ));
                }
                TokenKind::Eof => {
                    let e = self.err(t.start, t.end, "expected `}`");
                    self.exit_scope();
                    return Err(e);
                }
                _ => {}
            }
            let stmt = self.parse_stmt()?;
            // A trailing `expr }` is the arm's value; anything else is a
            // setup statement and parsing continues.
            if let Stmt::Expr(e) = &stmt {
                if self.peek().kind == TokenKind::RBrace {
                    self.bump();
                    let body = e.clone();
                    self.exit_scope();
                    return Ok((stmts, body));
                }
            }
            stmts.push(stmt);
        }
    }

    fn parse_print(&mut self) -> Result<PrintStmt, Diagnostic> {
        let kw = self.expect(&TokenKind::Print, "`print`")?;
        let value = if self.peek().kind == TokenKind::LParen {
            self.bump();
            let v = self.parse_expr()?;
            self.expect(&TokenKind::RParen, "`)`")?;
            v
        } else {
            self.parse_expr()?
        };
        self.consume_semi_opt();
        let id = self.next_id();
        let _ = kw.start;
        Ok(PrintStmt { id, value })
    }

    fn parse_expr(&mut self) -> Result<Expr, Diagnostic> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_and()?;
        while self.peek().kind == TokenKind::OrOr {
            self.bump();
            let right = self.parse_and()?;
            let id = self.next_id();
            left = Expr::Or {
                id,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_equality()?;
        while self.peek().kind == TokenKind::AndAnd {
            self.bump();
            let right = self.parse_equality()?;
            let id = self.next_id();
            left = Expr::And {
                id,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_comparison()?;
        loop {
            let is_eq = matches!(self.peek().kind, TokenKind::EqEq);
            let is_neq = matches!(self.peek().kind, TokenKind::NotEq);
            if !is_eq && !is_neq {
                break;
            }
            self.bump();
            let right = self.parse_comparison()?;
            let id = self.next_id();
            left = if is_eq {
                Expr::Eq {
                    id,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            } else {
                Expr::NotEq {
                    id,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_add()?;
        loop {
            let kind = match self.peek().kind {
                TokenKind::Lt => 0,
                TokenKind::LtEq => 1,
                TokenKind::Gt => 2,
                TokenKind::GtEq => 3,
                _ => break,
            };
            self.bump();
            let right = self.parse_add()?;
            let id = self.next_id();
            left = match kind {
                0 => Expr::Lt {
                    id,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                1 => Expr::LtEq {
                    id,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                2 => Expr::Gt {
                    id,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                _ => Expr::GtEq {
                    id,
                    left: Box::new(left),
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    fn parse_add(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_mul()?;
        loop {
            match self.peek().kind {
                TokenKind::Plus => {
                    self.bump();
                    let right = self.parse_mul()?;
                    let id = self.next_id();
                    left = Expr::Add {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::Minus => {
                    self.bump();
                    let right = self.parse_mul()?;
                    let id = self.next_id();
                    left = Expr::Sub {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_mul(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek().kind {
                TokenKind::Star => {
                    self.bump();
                    let right = self.parse_unary()?;
                    let id = self.next_id();
                    left = Expr::Mul {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::Slash => {
                    self.bump();
                    let right = self.parse_unary()?;
                    let id = self.next_id();
                    left = Expr::Div {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::Percent => {
                    self.bump();
                    let right = self.parse_unary()?;
                    let id = self.next_id();
                    left = Expr::Mod {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, Diagnostic> {
        if self.peek().kind == TokenKind::Minus {
            self.bump();
            let inner = self.parse_unary()?;
            let id = self.next_id();
            return Ok(Expr::Neg {
                id,
                inner: Box::new(inner),
            });
        }
        if self.peek().kind == TokenKind::Bang {
            self.bump();
            let inner = self.parse_unary()?;
            let id = self.next_id();
            return Ok(Expr::Not {
                id,
                inner: Box::new(inner),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut base = self.parse_primary()?;
        loop {
            match self.peek().kind {
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket, "`]`")?;
                    let id = self.next_id();
                    base = Expr::Index {
                        id,
                        base: Box::new(base),
                        index: Box::new(index),
                    };
                }
                TokenKind::Dot => {
                    self.bump();
                    let (name, _, _) = self.expect_ident("field or method name")?;
                    if self.peek().kind == TokenKind::LParen {
                        // `base.method(args...)`.
                        self.bump();
                        let mut args = Vec::new();
                        if self.peek().kind != TokenKind::RParen {
                            loop {
                                args.push(self.parse_expr()?);
                                if self.peek().kind == TokenKind::Comma {
                                    self.bump();
                                } else {
                                    break;
                                }
                            }
                        }
                        self.expect(&TokenKind::RParen, "`)`")?;
                        let id = self.next_id();
                        base = Expr::MethodCall {
                            id,
                            base: Box::new(base),
                            method: name,
                            args,
                        };
                    } else {
                        let id = self.next_id();
                        base = Expr::Field {
                            id,
                            base: Box::new(base),
                            field: name,
                        };
                    }
                }
                _ => break,
            }
        }
        Ok(base)
    }

    fn parse_primary(&mut self) -> Result<Expr, Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::IntLit(v) => {
                let v = *v;
                self.bump();
                let id = self.next_id();
                Ok(Expr::Int { id, value: v })
            }
            TokenKind::FloatLit(bits) => {
                let bits = *bits;
                self.bump();
                let id = self.next_id();
                Ok(Expr::Float { id, bits })
            }
            TokenKind::StrLit(s) => {
                let s = s.clone();
                self.bump();
                let id = self.next_id();
                Ok(Expr::Str { id, value: s })
            }
            TokenKind::True => {
                self.bump();
                let id = self.next_id();
                Ok(Expr::Bool { id, value: true })
            }
            TokenKind::False => {
                self.bump();
                let id = self.next_id();
                Ok(Expr::Bool { id, value: false })
            }
            TokenKind::LBracket => {
                self.bump();
                let mut elems = Vec::new();
                if self.peek().kind != TokenKind::RBracket {
                    loop {
                        elems.push(self.parse_expr()?);
                        if self.peek().kind == TokenKind::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RBracket, "`]`")?;
                let id = self.next_id();
                Ok(Expr::ArrayLit { id, elems })
            }
            TokenKind::LBrace => {
                // `{"key": value, ...}` map literal.
                self.bump();
                let mut entries = Vec::new();
                if self.peek().kind != TokenKind::RBrace {
                    loop {
                        let key = match &self.peek().kind {
                            TokenKind::StrLit(s) => {
                                let s = s.clone();
                                self.bump();
                                s
                            }
                            TokenKind::Ident(s) => {
                                let s = s.clone();
                                self.bump();
                                s
                            }
                            _ => {
                                let p = self.peek().clone();
                                return Err(self.err(
                                    p.start,
                                    p.end,
                                    "expected a `\"key\"` or `key` in map literal",
                                ));
                            }
                        };
                        self.expect(&TokenKind::Colon, "`:`")?;
                        let val = self.parse_expr()?;
                        entries.push((key, val));
                        if self.peek().kind == TokenKind::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}`")?;
                let id = self.next_id();
                Ok(Expr::MapLit { id, entries })
            }
            TokenKind::LParen => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RParen, "`)`")?;
                Ok(e)
            }
            TokenKind::Spawn => {
                // S6 `spawn(cmd, args)` process builtin shares the `spawn`
                // spelling with task-group `spawn <call>`. A `spawn(`
                // parenthesized list is the builtin Call, except the one
                // shape task code legitimately uses: a single Call inside
                // (`spawn (f())`). Anything else (0 args, 1 non-Call arg,
                // 2 args, 3+ args) is the builtin, so arity/type checking
                // reports `E-ARITY`/`E-TYPE` instead of a task error. No
                // valid existing program has Spawn+LParen+comma or
                // Spawn+LParen+non-Call, so no existing meaning changes.
                if self.tokens.get(self.pos + 1).map(|t| t.kind.clone())
                    == Some(TokenKind::LParen)
                {
                    let saved = self.pos;
                    let scopes_next: Vec<u32> =
                        self.scopes.iter().map(|s| s.next).collect();
                    self.bump();
                    self.bump();
                    let mut args = Vec::new();
                    let mut ok = true;
                    if self.peek().kind != TokenKind::RParen {
                        loop {
                            match self.parse_expr() {
                                Ok(a) => args.push(a),
                                Err(_) => {
                                    ok = false;
                                    break;
                                }
                            }
                            if self.peek().kind == TokenKind::Comma {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    if ok && self.peek().kind == TokenKind::RParen {
                        let single_call = args.len() == 1
                            && matches!(args[0], Expr::Call { .. });
                        if !single_call {
                            self.bump();
                            let id = self.next_id();
                            return Ok(Expr::Call {
                                id,
                                func: "spawn".to_string(),
                                type_args: Vec::new(),
                                args,
                            });
                        }
                    }
                    self.pos = saved;
                    restore_scope_next(&mut self.scopes, &scopes_next);
                }
                let kw = self.bump();
                let inner = self.parse_primary()?;
                let id = self.next_id();
                let _ = kw.start;
                Ok(Expr::Spawn {
                    id,
                    call: Box::new(inner),
                })
            }
            TokenKind::Await => {
                self.bump();
                let (name, _, _) = self.expect_ident("task handle after `await`")?;
                let id = self.next_id();
                Ok(Expr::Await { id, name })
            }
            TokenKind::Match => self.parse_match(),
            TokenKind::Fn => self.parse_closure(),
            TokenKind::Ident(name) => {
                let name = name.clone();
                let (s, e) = (t.start, t.end);
                self.bump();
                if self.peek().kind == TokenKind::ColonColon {
                    // `Enum::Variant`, `Enum::Variant(args)`,
                    // `mod::Enum::Variant`, `mod::Enum::Variant(args)`,
                    // `mod::Fn(args)` (resolved in modules), or
                    // `mod::Struct { ... }` (qualified struct literal).
                    self.bump();
                    let (second, _, _) = self.expect_ident("name after `::`")?;
                    if self.peek().kind == TokenKind::ColonColon {
                        self.bump();
                        let (variant, _, _) = self.expect_ident("variant name after `::`")?;
                        let args = self.parse_call_args_opt()?;
                        let id = self.next_id();
                        let _ = (s, e);
                        return Ok(Expr::EnumCtor {
                            id,
                            enum_name: format!("{name}::{second}"),
                            variant,
                            args,
                            type_args: Vec::new(),
                        });
                    }
                    if self.peek().kind == TokenKind::LBrace {
                        // `mod::Struct { f: v, ... }`; a `{` can never start
                        // enum-constructor args, so this is unambiguous.
                        self.bump();
                        let fields = self.parse_struct_lit_fields()?;
                        let id = self.next_id();
                        let _ = (s, e);
                        return Ok(Expr::StructLit {
                            id,
                            name: format!("{name}::{second}"),
                            fields,
                        });
                    }
                    // Two-segment `m::f(args)` — or `m::f<T>(args)` with
                    // explicit type arguments (BUGHUNT-2: the qualified
                    // twin of the bare-name `<...>` backtracking above).
                    // Commit only on the full `< Type (, Type)* > (`
                    // shape; anything else stays a comparison, exactly
                    // like the bare-name case (`m::f < 123` untouched).
                    // (The `(` after `>` is consumed by `bump`, like the
                    // bare-name arm — `parse_call_args_opt` would expect
                    // to consume it itself, so the loop is inline here.)
                    if self.peek().kind == TokenKind::Lt {
                        match self.try_parse_generic_call_args()? {
                            Some(type_args) => {
                                self.bump();
                                let mut args = Vec::new();
                                if self.peek().kind != TokenKind::RParen {
                                    loop {
                                        args.push(self.parse_expr()?);
                                        if self.peek().kind == TokenKind::Comma {
                                            self.bump();
                                        } else {
                                            break;
                                        }
                                    }
                                }
                                self.expect(&TokenKind::RParen, "`)`")?;
                                let id = self.next_id();
                                let _ = (s, e);
                                return Ok(Expr::EnumCtor {
                                    id,
                                    enum_name: name,
                                    variant: second,
                                    args,
                                    type_args,
                                });
                            }
                            None => {}
                        }
                    }
                    let args = self.parse_call_args_opt()?;
                    let id = self.next_id();
                    let _ = (s, e);
                    return Ok(Expr::EnumCtor {
                        id,
                        enum_name: name,
                        variant: second,
                        args,
                        type_args: Vec::new(),
                    });
                }
                if self.peek().kind == TokenKind::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    if self.peek().kind != TokenKind::RParen {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.peek().kind == TokenKind::Comma {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RParen, "`)`")?;
                    let id = self.next_id();
                    let _ = (s, e);
                    Ok(Expr::Call {
                        id,
                        func: name,
                        type_args: Vec::new(),
                        args,
                    })
                } else if self.peek().kind == TokenKind::Lt {
                    // Explicit generic instantiation (`count<T>(...)`,
                    // `count<i32>(...)`): ambiguous with `<` comparison, so
                    // speculative with backtracking. Commit only when the
                    // full `< Type (, Type)* > (` shape is present;
                    // otherwise fall back to a plain variable and let the
                    // comparison layer handle `<` (preserving `a < b`).
                    match self.try_parse_generic_call_args()? {
                        Some(type_args) => {
                            self.bump();
                            let mut args = Vec::new();
                            if self.peek().kind != TokenKind::RParen {
                                loop {
                                    args.push(self.parse_expr()?);
                                    if self.peek().kind == TokenKind::Comma {
                                        self.bump();
                                    } else {
                                        break;
                                    }
                                }
                            }
                            self.expect(&TokenKind::RParen, "`)`")?;
                            let id = self.next_id();
                            let _ = (s, e);
                            Ok(Expr::Call {
                                id,
                                func: name,
                                type_args,
                                args,
                            })
                        }
                        None => {
                            let id = self.next_id();
                            Ok(Expr::Var { id, name })
                        }
                    }
                } else if self.peek().kind == TokenKind::LBrace
                    && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                {
                    // `Point { x: 1, y: 2 }` struct literal (capitalized).
                    self.bump();
                    let fields = self.parse_struct_lit_fields()?;
                    let id = self.next_id();
                    Ok(Expr::StructLit { id, name, fields })
                } else {
                    let id = self.next_id();
                    Ok(Expr::Var { id, name })
                }
            }
            _ => Err(self.err(t.start, t.end, "expected an expression")),
        }
    }

    /// `fn(params) { body }` anonymous function value. Same optional-`->`
    /// shape as a named `fn` (omitted means `-> void`); no name and no
    /// effects: closures are synchronous values, so `spawn`/`await` inside
    /// the body is rejected at check time.
    fn parse_closure(&mut self) -> Result<Expr, Diagnostic> {
        self.expect(&TokenKind::Fn, "`fn`")?;
        let id = self.next_id();
        self.enter_scope(&id);
        self.expect(&TokenKind::LParen, "`(`")?;
        let params = self.parse_params()?;
        self.expect(&TokenKind::RParen, "`)`")?;
        let (return_ty, _omitted) = self.parse_return_ty_opt()?;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let blk_id = self.next_id();
        self.enter_scope(&blk_id);
        let mut stmts = Vec::new();
        loop {
            match self.peek().kind {
                TokenKind::RBrace | TokenKind::Eof => break,
                _ => stmts.push(self.parse_stmt()?),
            }
        }
        let end_tok = self.peek().clone();
        match end_tok.kind {
            TokenKind::RBrace => {
                self.bump();
            }
            _ => {
                let e = self.err(end_tok.start, end_tok.end, "expected `}`");
                self.exit_scope();
                self.exit_scope();
                return Err(e);
            }
        }
        self.exit_scope();
        self.exit_scope();
        Ok(Expr::Closure {
            id,
            params,
            return_ty,
            body: Block { id: blk_id, stmts },
        })
    }

    /// Optional `(a, b, ...)` call/constructor arguments (empty when absent).
    fn parse_call_args_opt(&mut self) -> Result<Vec<Expr>, Diagnostic> {
        let mut args = Vec::new();
        if self.peek().kind == TokenKind::LParen {
            self.bump();
            if self.peek().kind != TokenKind::RParen {
                loop {
                    args.push(self.parse_expr()?);
                    if self.peek().kind == TokenKind::Comma {
                        self.bump();
                    } else {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
        }
        Ok(args)
    }

    /// `{ f: v, ... }` struct-literal fields (brace already consumed).
    fn parse_struct_lit_fields(&mut self) -> Result<Vec<(String, Expr)>, Diagnostic> {
        let mut fields = Vec::new();
        if self.peek().kind != TokenKind::RBrace {
            loop {
                let (fname, _, _) = self.expect_ident("field name")?;
                self.expect(&TokenKind::Colon, "`:`")?;
                let val = self.parse_expr()?;
                fields.push((fname, val));
                if self.peek().kind == TokenKind::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RBrace, "`}`")?;
        Ok(fields)
    }
}

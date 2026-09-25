//! Klang v2 Echo parsing (Phase 4).
//!
//! Grammar:
//!
//! ```text
//! echo_decl   := "echo" "fn" ident "(" param ("," param)* ")" "->" ty "{" body "}"
//! param       := ident ":" ty
//! listen_expr := "listen" "(" ident ")"
//! ty          := ["?" | "!"] ident
//! ```
//!
//! Incomplete Echo syntax yields recoverable `E-PARSE-ECHO` diagnostics
//! pointing at the truncation site. `Echo<T>` handle types are checked by
//! name in later passes; the parser only records the inner type string.

use crate::ast::echo::{EchoDecl, EchoOwnership, ListenExpr};
use crate::ast::NodeId;
use crate::diagnostics::Diagnostic;
use crate::lexer::resonance::TokenKind;
use crate::lexer::{lex, Token};

const FILE: &str = "input.v2";

struct Scope {
    path: Vec<u32>,
    next: u32,
}

struct P {
    toks: Vec<Token>,
    pos: usize,
    scopes: Vec<Scope>,
}

impl P {
    fn new(toks: Vec<Token>) -> Self {
        Self {
            toks,
            pos: 0,
            scopes: vec![Scope {
                path: Vec::new(),
                next: 0,
            }],
        }
    }

    fn peek(&self) -> &Token {
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn next_id(&mut self) -> NodeId {
        let s = self.scopes.last_mut().expect("root scope");
        let mut path = s.path.clone();
        path.push(s.next);
        s.next += 1;
        NodeId::new(path)
    }

    fn err(&self, start: usize, end: usize, message: &str) -> Diagnostic {
        crate::ai_safety::diagnostics::parse_echo(FILE, start, end, message)
    }

    fn expect_tok(
        &mut self,
        pred: impl Fn(&TokenKind) -> bool,
        what: &str,
    ) -> Result<Token, Diagnostic> {
        let t = self.peek().clone();
        if pred(&t.kind) {
            Ok(self.bump())
        } else {
            Err(self.err(t.start, t.end, &format!("expected {what}")))
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<String, Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Ident(n) => {
                let n = n.clone();
                self.bump();
                Ok(n)
            }
            _ => Err(self.err(t.start, t.end, &format!("expected {what}"))),
        }
    }

    fn parse_ty(&mut self, what: &str) -> Result<String, Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Question => {
                self.bump();
                Ok(format!("?{}", self.expect_ident(what)?))
            }
            TokenKind::Bang => {
                self.bump();
                Ok(format!("!{}", self.expect_ident(what)?))
            }
            TokenKind::Ident(_) => self.expect_ident(what),
            _ => Err(self.err(t.start, t.end, &format!("expected {what}"))),
        }
    }
}

/// Initial ownership of a freshly parsed Echo handle.
pub fn initial_ownership() -> EchoOwnership {
    EchoOwnership::Created
}

/// Parse `echo fn name(params) -> Ty { body }`.
pub fn parse_echo_decl(source: &str) -> Result<EchoDecl, Diagnostic> {
    let toks = lex(source).map_err(|e| {
        Diagnostic::error(
            "E-PARSE-ECHO",
            &format!("lex error: {}", e.message),
            FILE,
            e.offset,
            e.offset,
            "input does not match the v2 echo grammar",
            &["check echo/listen spelling"],
            "syntax/echo",
        )
    })?;
    let mut p = P::new(toks);
    p.expect_tok(|k| matches!(k, TokenKind::Echo), "`echo`")?;
    // `fn` lexes as a plain identifier in the v2 lexer (v1 owns it).
    let t = p.peek().clone();
    match &t.kind {
        TokenKind::Ident(n) if n == "fn" => {
            p.bump();
        }
        _ => return Err(p.err(t.start, t.end, "expected `fn`")),
    }
    let name = p.expect_ident("echo function name")?;
    p.expect_tok(|k| matches!(k, TokenKind::LParen), "`(`")?;
    let mut params: Vec<(String, String)> = Vec::new();
    if !matches!(p.peek().kind, TokenKind::RParen) {
        loop {
            let pn = p.expect_ident("parameter name")?;
            p.expect_tok(|k| matches!(k, TokenKind::Colon), "`:`")?;
            let ty = p.parse_ty("parameter type")?;
            params.push((pn, ty));
            if matches!(p.peek().kind, TokenKind::Comma) {
                p.bump();
            } else {
                break;
            }
        }
    }
    p.expect_tok(|k| matches!(k, TokenKind::RParen), "`)`")?;
    p.expect_tok(|k| matches!(k, TokenKind::Arrow), "`->`")?;
    let return_ty = p.parse_ty("return type")?;
    p.expect_tok(|k| matches!(k, TokenKind::LBrace), "`{`")?;
    // Body is opaque in Phase 4: skip to the matching `}` for recovery.
    let mut depth: usize = 1;
    while depth > 0 {
        let t = p.peek().clone();
        match &t.kind {
            TokenKind::LBrace => {
                depth += 1;
                p.bump();
            }
            TokenKind::RBrace => {
                depth -= 1;
                p.bump();
            }
            TokenKind::Eof => {
                return Err(p.err(t.start, t.end, "expected `}`"));
            }
            _ => {
                p.bump();
            }
        }
    }
    Ok(EchoDecl {
        id: p.next_id(),
        name,
        params,
        return_ty,
    })
}

/// Parse `listen(handle)`.
pub fn parse_listen(source: &str) -> Result<ListenExpr, Diagnostic> {
    let toks = lex(source).map_err(|e| {
        Diagnostic::error(
            "E-PARSE-ECHO",
            &format!("lex error: {}", e.message),
            FILE,
            e.offset,
            e.offset,
            "input does not match the v2 echo grammar",
            &["write listen(handle)"],
            "syntax/echo",
        )
    })?;
    let mut p = P::new(toks);
    p.expect_tok(|k| matches!(k, TokenKind::Listen), "`listen`")?;
    p.expect_tok(|k| matches!(k, TokenKind::LParen), "`(`")?;
    let handle = p.expect_ident("echo handle")?;
    p.expect_tok(|k| matches!(k, TokenKind::RParen), "`)`")?;
    Ok(ListenExpr {
        id: p.next_id(),
        handle,
    })
}

//! Klang v2 Flow parsing (Phase 4).
//!
//! Grammar:
//!
//! ```text
//! flow_decl := "flow" "(" flow_arg ("," flow_arg)* ")" ("->" ty)? "{" body "}"
//! flow_arg  := param | dep
//! param     := ident ":" ty
//! dep       := "dep" "=" ident ":" ty
//! ty        := ["?" | "!"] ident
//! ```
//!
//! Identity: the declaration id is allocated first, its scope entered,
//! then the expression id is allocated inside it, so
//! `expr.id.starts_with(&decl.id)` always holds. Duplicate dependency
//! names are `E-PARSE-FLOW`, never silently merged.

use crate::ast::flow::{FlowDecl, FlowDependency, FlowExpr, FlowParam};
use crate::ast::NodeId;
use crate::diagnostics::Diagnostic;
use crate::lexer::resonance::TokenKind;
use crate::lexer::{lex, Token};

const FILE: &str = "input.v2";

#[derive(Debug)]
struct Scope {
    path: Vec<u32>,
    next: u32,
}

/// Minimal recursive-descent parser over v2 tokens with a scope stack.
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

    fn enter(&mut self, id: &NodeId) {
        self.scopes.push(Scope {
            path: id.path.clone(),
            next: 0,
        });
    }

    fn exit(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    fn err(&self, start: usize, end: usize, message: &str) -> Diagnostic {
        crate::ai_safety::diagnostics::parse_flow(FILE, start, end, message)
    }

    fn expect_kind(&mut self, want: &TokenKind, what: &str) -> Result<Token, Diagnostic> {
        let t = self.peek().clone();
        let ok = match (want, &t.kind) {
            (TokenKind::LParen, TokenKind::LParen)
            | (TokenKind::RParen, TokenKind::RParen)
            | (TokenKind::LBrace, TokenKind::LBrace)
            | (TokenKind::RBrace, TokenKind::RBrace)
            | (TokenKind::Comma, TokenKind::Comma)
            | (TokenKind::Colon, TokenKind::Colon)
            | (TokenKind::Eq, TokenKind::Eq)
            | (TokenKind::Arrow, TokenKind::Arrow)
            | (TokenKind::Flow, TokenKind::Flow) => true,
            _ => false,
        };
        if ok {
            Ok(self.bump())
        } else {
            Err(self.err(t.start, t.end, &format!("expected {what}")))
        }
    }

    fn expect_ident(&mut self, what: &str) -> Result<(String, usize, usize), Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Ident(n) => {
                let n = n.clone();
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok((n, s, e))
            }
            _ => Err(self.err(t.start, t.end, &format!("expected {what}"))),
        }
    }

    /// `["?" | "!"] ident` — resonance prefix folds into the stored name.
    fn parse_ty(&mut self, what: &str) -> Result<String, Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Question => {
                self.bump();
                let (n, _, _) = self.expect_ident(what)?;
                Ok(format!("?{n}"))
            }
            TokenKind::Bang => {
                self.bump();
                let (n, _, _) = self.expect_ident(what)?;
                Ok(format!("!{n}"))
            }
            TokenKind::Ident(_) => Ok(self.expect_ident(what)?.0),
            _ => Err(self.err(t.start, t.end, &format!("expected {what}"))),
        }
    }
}

/// Parse one `flow(...) ... { ... }` declaration from `source`.
///
/// `name` is the let-bound name when known (`Some`), else `None`.
pub fn parse_flow_decl(source: &str, name: Option<&str>) -> Result<FlowDecl, Diagnostic> {
    let toks = lex(source).map_err(|e| {
        Diagnostic::error(
            "E-PARSE-FLOW",
            &format!("lex error: {}", e.message),
            FILE,
            e.offset,
            e.offset,
            "input does not match the v2 flow grammar",
            &["check ? ! |> flow dep= spelling"],
            "syntax/flow",
        )
    })?;
    let mut p = P::new(toks);
    p.expect_kind(&TokenKind::Flow, "`flow`")?;
    p.expect_kind(&TokenKind::LParen, "`(`")?;
    let mut params: Vec<FlowParam> = Vec::new();
    let mut deps: Vec<FlowDependency> = Vec::new();
    if !matches!(p.peek().kind, TokenKind::RParen) {
        loop {
            if matches!(p.peek().kind, TokenKind::Dep) {
                p.bump();
                p.expect_kind(&TokenKind::Eq, "`=` after `dep`")?;
                let (dname, ds, de) = p.expect_ident("dependency name")?;
                if deps.iter().any(|d: &FlowDependency| d.name == dname) {
                    return Err(p.err(ds, de, &format!("duplicate dependency `{dname}`")));
                }
                p.expect_kind(&TokenKind::Colon, "`:`")?;
                let ty = p.parse_ty("dependency type")?;
                deps.push(FlowDependency { name: dname, ty });
            } else {
                let (pname, _, _) = p.expect_ident("parameter name")?;
                p.expect_kind(&TokenKind::Colon, "`:`")?;
                let ty = p.parse_ty("parameter type")?;
                params.push(FlowParam { name: pname, ty });
            }
            if matches!(p.peek().kind, TokenKind::Comma) {
                p.bump();
            } else {
                break;
            }
        }
    }
    p.expect_kind(&TokenKind::RParen, "`)`")?;
    // Optional `-> Ret`.
    let return_ty = if matches!(p.peek().kind, TokenKind::Arrow) {
        p.bump();
        Some(p.parse_ty("return type")?)
    } else {
        None
    };
    p.expect_kind(&TokenKind::LBrace, "`{`")?;
    let body = capture_body(source, &mut p)?;
    let decl_id = p.next_id();
    p.enter(&decl_id);
    let expr_id = p.next_id();
    p.exit();
    Ok(FlowDecl {
        id: decl_id,
        name: name.map(str::to_string),
        expr: FlowExpr {
            id: expr_id,
            params,
            deps,
            return_ty,
            body,
        },
    })
}

/// Capture `{ ... }` body raw source with nesting, consuming the final `}`.
fn capture_body(source: &str, p: &mut P) -> Result<String, Diagnostic> {
    // Current token is the first inside the braces.
    let mut depth: usize = 1;
    let start = p.peek().start;
    loop {
        let t = p.peek().clone();
        match &t.kind {
            TokenKind::LBrace => {
                depth += 1;
                p.bump();
            }
            TokenKind::RBrace => {
                depth -= 1;
                p.bump();
                if depth == 0 {
                    let end = t.start;
                    return Ok(source[start.min(source.len())..end.min(source.len())].to_string());
                }
            }
            TokenKind::Eof => {
                return Err(p.err(t.start, t.end, "expected `}`"));
            }
            _ => {
                p.bump();
            }
        }
    }
}

//! Klang v2 full program parser (Phase 10+).
//!
//! Parses complete v2 programs with schemas, echo fns, flows, and functions
//! using resonance types. Uses the v2 lexer from `crate::lexer::resonance`.

use crate::ast::v2::{
    SchemaDecl, SchemaField, V2AssignStmt, V2AssignTarget, V2Block, V2Expr,
    V2FunctionDecl, V2IfStmt, V2LetStmt, V2Param, V2PrintStmt,
    V2ReturnStmt, V2Stmt, V2TuneExpr, V2VerifyExpr,
};
use std::collections::HashMap;
use crate::ast::{echo::EchoDecl, flow::{FlowDecl, FlowExpr, FlowParam, FlowDependency}, resonance::{QualifiedType, ResonanceQualifier}, NodeId};
use crate::diagnostics::Diagnostic;
use crate::lexer::resonance::{lex, Token, TokenKind};

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
    src: String,
    schemas: Vec<SchemaDecl>,
    echo_fns: Vec<EchoDecl>,
    echo_bodies: HashMap<String, V2Block>,
    flows: Vec<FlowDecl>,
    functions: Vec<V2FunctionDecl>,
}

impl P {
    fn new(toks: Vec<Token>, src: String) -> Self {
        Self {
            toks,
            pos: 0,
            scopes: vec![Scope {
                path: Vec::new(),
                next: 0,
            }],
            src,
            schemas: Vec::new(),
            echo_fns: Vec::new(),
            echo_bodies: HashMap::new(),
            flows: Vec::new(),
            functions: Vec::new(),
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
        crate::ai_safety::diagnostics::parse_v2(FILE, start, end, message)
    }

    fn expect_kind(&mut self, want: &TokenKind, what: &str) -> Result<Token, Diagnostic> {
        let t = self.peek().clone();
        // Unit variants compare by equality; data variants (Ident/StrLit/…)
        // are never passed here, so exact equality is the right check and
        // automatically covers Lt/Gt/Semi/Dot/etc. without an allowlist.
        if want == &t.kind {
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
            TokenKind::Schema => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("schema".to_string(), s, e))
            }
            TokenKind::Echo => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("echo".to_string(), s, e))
            }
            TokenKind::Flow => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("flow".to_string(), s, e))
            }
            TokenKind::Let => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("let".to_string(), s, e))
            }
            TokenKind::Return => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("return".to_string(), s, e))
            }
            TokenKind::If => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("if".to_string(), s, e))
            }
            TokenKind::Else => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("else".to_string(), s, e))
            }
            TokenKind::Print => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("print".to_string(), s, e))
            }
            TokenKind::Tune => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("tune".to_string(), s, e))
            }
            TokenKind::Verify => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("verify".to_string(), s, e))
            }
            TokenKind::Listen => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("listen".to_string(), s, e))
            }
            TokenKind::Dep => {
                let (s, e) = (t.start, t.end);
                self.bump();
                Ok(("dep".to_string(), s, e))
            }
            _ => Err(self.err(t.start, t.end, &format!("expected {what}"))),
        }
    }

    fn parse_qualified_type(&mut self, what: &str) -> Result<QualifiedType, Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Question => {
                self.bump();
                let (base, _, _) = self.expect_ident(what)?;
                Ok(QualifiedType::new(ResonanceQualifier::Dissonant, &base))
            }
            TokenKind::Bang => {
                self.bump();
                let (base, _, _) = self.expect_ident(what)?;
                Ok(QualifiedType::new(ResonanceQualifier::Harmonic, &base))
            }
            TokenKind::Ident(_) => {
                let (base, _, _) = self.expect_ident(what)?;
                Ok(QualifiedType::new(ResonanceQualifier::Consonant, &base))
            }
            TokenKind::Schema => {
                let (base, _, _) = self.expect_ident(what)?;
                Ok(QualifiedType::new(ResonanceQualifier::Consonant, &base))
            }
            _ => Err(self.err(t.start, t.end, &format!("expected {what}"))),
        }
    }

    fn parse_schema_decl(&mut self) -> Result<SchemaDecl, Diagnostic> {
        self.expect_kind(&TokenKind::Schema, "`schema`")?;
        let (name, ns, ne) = self.expect_ident("schema name")?;
        let version = match &self.peek().kind {
            TokenKind::StrLit(v) => {
                let v = v.clone();
                self.bump();
                v
            }
            _ => {
                let t = self.peek().clone();
                return Err(self.err(t.start, t.end, "expected schema version string"));
            }
        };
        self.expect_kind(&TokenKind::LBrace, "`{`")?;
        let mut fields = Vec::new();
        while !matches!(self.peek().kind, TokenKind::RBrace | TokenKind::Eof) {
            let (fname, _, _) = self.expect_ident("field name")?;
            self.expect_kind(&TokenKind::Colon, "`:`")?;
            let fty = self.parse_type_name()?;
            fields.push(SchemaField { name: fname, ty: fty });
            if matches!(self.peek().kind, TokenKind::Comma) {
                self.bump();
            }
        }
        self.expect_kind(&TokenKind::RBrace, "`}`")?;
        Ok(SchemaDecl {
            id: self.next_id(),
            name,
            name_span: (ns, ne),
            version,
            fields,
        })
    }

    fn parse_type_name(&mut self) -> Result<String, Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Ident(n) => {
                let n = n.clone();
                self.bump();
                Ok(n)
            }
            TokenKind::Schema => {
                self.bump();
                Ok("schema".to_string())
            }
            TokenKind::Echo => {
                self.bump();
                Ok("echo".to_string())
            }
            TokenKind::Flow => {
                self.bump();
                Ok("flow".to_string())
            }
            _ => Err(self.err(t.start, t.end, "expected type name")),
        }
    }

    fn parse_echo_decl(&mut self) -> Result<EchoDecl, Diagnostic> {
        self.expect_kind(&TokenKind::Echo, "`echo`")?;
        // `fn` lexes as a plain identifier in the v2 lexer (v1 owns it).
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Ident(n) if n == "fn" => {
                self.bump();
            }
            _ => return Err(self.err(t.start, t.end, "expected `fn`")),
        }
        let (name, _, _) = self.expect_ident("echo function name")?;
        self.expect_kind(&TokenKind::LParen, "`(`")?;
        let mut params: Vec<(String, String)> = Vec::new();
        if !matches!(self.peek().kind, TokenKind::RParen) {
            loop {
                let (pname, _, _) = self.expect_ident("parameter name")?;
                self.expect_kind(&TokenKind::Colon, "`:`")?;
                let pty = self.parse_qualified_type("parameter type")?;
                params.push((pname, pty.display()));
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect_kind(&TokenKind::RParen, "`)`")?;
        self.expect_kind(&TokenKind::Arrow, "`->`")?;
        let return_ty = self.parse_qualified_type("return type")?;
        self.expect_kind(&TokenKind::LBrace, "`{`")?;
        // Real body: parse as a block of v2 statements (not skipped).
        // This is what the v2 interpreter executes for background work.
        let decl_id = self.next_id();
        self.enter(&decl_id);
        let blk_id = self.next_id();
        self.enter(&blk_id);
        let mut stmts = Vec::new();
        while !matches!(self.peek().kind, TokenKind::RBrace | TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        let end_tok = self.peek().clone();
        match end_tok.kind {
            TokenKind::RBrace => {
                self.bump();
            }
            _ => {
                let e = self.err(end_tok.start, end_tok.end, "expected `}`");
                self.exit();
                self.exit();
                return Err(e);
            }
        }
        self.exit();
        self.exit();
        let body = V2Block { id: blk_id, stmts };
        self.echo_bodies.insert(name.clone(), body);
        Ok(EchoDecl {
            id: decl_id,
            name,
            params,
            return_ty: return_ty.display(),
        })
    }

    fn parse_flow_expr(&mut self) -> Result<FlowExpr, Diagnostic> {
        self.expect_kind(&TokenKind::Flow, "`flow`")?;
        self.expect_kind(&TokenKind::LParen, "`(`")?;
        let mut params: Vec<FlowParam> = Vec::new();
        let mut deps: Vec<FlowDependency> = Vec::new();
        if !matches!(self.peek().kind, TokenKind::RParen) {
            loop {
                if matches!(self.peek().kind, TokenKind::Dep) {
                    self.bump();
                    self.expect_kind(&TokenKind::Eq, "`=` after `dep`")?;
                    let (dname, ds, de) = self.expect_ident("dependency name")?;
                    if deps.iter().any(|d: &FlowDependency| d.name == dname) {
                        return Err(self.err(ds, de, &format!("duplicate dependency `{dname}`")));
                    }
                    self.expect_kind(&TokenKind::Colon, "`:`")?;
                    let ty = self.parse_qualified_type("dependency type")?;
                    deps.push(FlowDependency { name: dname, ty: ty.display() });
                } else {
                    let (pname, _, _) = self.expect_ident("parameter name")?;
                    self.expect_kind(&TokenKind::Colon, "`:`")?;
                    let ty = self.parse_qualified_type("parameter type")?;
                    params.push(FlowParam { name: pname, ty: ty.display() });
                }
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect_kind(&TokenKind::RParen, "`)`")?;
        let return_ty = if matches!(self.peek().kind, TokenKind::Arrow) {
            self.bump();
            Some(self.parse_qualified_type("return type")?.display())
        } else {
            None
        };
        self.expect_kind(&TokenKind::LBrace, "`{`")?;
        let mut depth: usize = 1;
        let body_start = self.peek().start;
        while depth > 0 {
            let t = self.peek().clone();
            match &t.kind {
                TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RBrace => {
                    depth -= 1;
                    let body_end = t.start;
                    self.bump();
                    if depth == 0 {
                        // Capture the real body source so the interpreter
                        // executes what was written, not a placeholder.
                        let end = body_end.min(self.src.len());
                        let start = body_start.min(end);
                        let body = self.src[start..end].to_string();
                        return Ok(FlowExpr {
                            id: self.next_id(),
                            params,
                            deps,
                            return_ty,
                            body,
                        });
                    }
                }
                TokenKind::Eof => {
                    return Err(self.err(t.start, t.end, "expected `}`"));
                }
                _ => {
                    self.bump();
                }
            }
        }
        Ok(FlowExpr {
            id: self.next_id(),
            params,
            deps,
            return_ty,
            body: String::new(),
        })
    }

    fn parse_flow_decl(&mut self) -> Result<FlowDecl, Diagnostic> {
        self.expect_kind(&TokenKind::Flow, "`flow`")?;
        self.expect_kind(&TokenKind::LParen, "`(`")?;
        let mut params: Vec<FlowParam> = Vec::new();
        let mut deps: Vec<FlowDependency> = Vec::new();
        if !matches!(self.peek().kind, TokenKind::RParen) {
            loop {
                if matches!(self.peek().kind, TokenKind::Dep) {
                    self.bump();
                    self.expect_kind(&TokenKind::Eq, "`=` after `dep`")?;
                    let (dname, ds, de) = self.expect_ident("dependency name")?;
                    if deps.iter().any(|d: &FlowDependency| d.name == dname) {
                        return Err(self.err(ds, de, &format!("duplicate dependency `{dname}`")));
                    }
                    self.expect_kind(&TokenKind::Colon, "`:`")?;
                    let ty = self.parse_qualified_type("dependency type")?;
                    deps.push(FlowDependency { name: dname, ty: ty.display() });
                } else {
                    let (pname, _, _) = self.expect_ident("parameter name")?;
                    self.expect_kind(&TokenKind::Colon, "`:`")?;
                    let ty = self.parse_qualified_type("parameter type")?;
                    params.push(FlowParam { name: pname, ty: ty.display() });
                }
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect_kind(&TokenKind::RParen, "`)`")?;
        let return_ty = if matches!(self.peek().kind, TokenKind::Arrow) {
            self.bump();
            Some(self.parse_qualified_type("return type")?.display())
        } else {
            None
        };
        self.expect_kind(&TokenKind::LBrace, "`{`")?;
        let body = self.capture_body()?;
        let decl_id = self.next_id();
        self.enter(&decl_id);
        let expr_id = self.next_id();
        self.exit();
        Ok(FlowDecl {
            id: decl_id,
            name: None, // Top-level flow declarations don't have a let-bound name in this syntax
            expr: FlowExpr {
                id: expr_id,
                params,
                deps,
                return_ty,
                body,
            },
        })
    }

    fn capture_body(&mut self) -> Result<String, Diagnostic> {
        let mut depth: usize = 1;
        let body_start = self.peek().start;
        loop {
            let t = self.peek().clone();
            match &t.kind {
                TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RBrace => {
                    depth -= 1;
                    let body_end = t.start;
                    self.bump();
                    if depth == 0 {
                        let end = body_end.min(self.src.len());
                        let start = body_start.min(end);
                        return Ok(self.src[start..end].to_string());
                    }
                }
                TokenKind::Eof => {
                    return Err(self.err(t.start, t.end, "expected `}`"));
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn parse_function(&mut self) -> Result<V2FunctionDecl, Diagnostic> {
        let is_pub = false; // v2 doesn't have pub yet
        // `fn` lexes as a plain identifier in the v2 lexer (v1 owns it).
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::Ident(n) if n == "fn" => {
                self.bump();
            }
            _ => return Err(self.err(t.start, t.end, "expected `fn`")),
        }
        let (name, ns, ne) = self.expect_ident("function name")?;
        self.expect_kind(&TokenKind::LParen, "`(`")?;
        let mut params: Vec<V2Param> = Vec::new();
        if !matches!(self.peek().kind, TokenKind::RParen) {
            loop {
                let (pname, _, _) = self.expect_ident("parameter name")?;
                self.expect_kind(&TokenKind::Colon, "`:`")?;
                let pty = self.parse_qualified_type("parameter type")?;
                params.push(V2Param { name: pname, ty: pty });
                if matches!(self.peek().kind, TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect_kind(&TokenKind::RParen, "`)`")?;
        self.expect_kind(&TokenKind::Arrow, "`->`")?;
        let return_ty = self.parse_qualified_type("return type")?;
        self.expect_kind(&TokenKind::LBrace, "`{`")?;
        let fn_id = self.next_id();
        self.enter(&fn_id);
        let blk_id = self.next_id();
        self.enter(&blk_id);
        let mut stmts = Vec::new();
        while !matches!(self.peek().kind, TokenKind::RBrace | TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        let end_tok = self.peek().clone();
        match end_tok.kind {
            TokenKind::RBrace => {
                self.bump();
            }
            _ => {
                let e = self.err(end_tok.start, end_tok.end, "expected `}`");
                self.exit();
                self.exit();
                return Err(e);
            }
        }
        self.exit();
        self.exit();
        Ok(V2FunctionDecl {
            id: fn_id,
            name,
            name_span: (ns, ne),
            is_pub,
            params,
            return_ty,
            body: V2Block { id: blk_id, stmts },
        })
    }

    fn parse_stmt(&mut self) -> Result<V2Stmt, Diagnostic> {
        match self.peek().kind {
            TokenKind::Let => Ok(V2Stmt::Let(self.parse_let()?)),
            TokenKind::Return => Ok(V2Stmt::Return(self.parse_return()?)),
            TokenKind::If => Ok(V2Stmt::If(self.parse_if()?)),
            TokenKind::Print => Ok(V2Stmt::Print(self.parse_print()?)),
            TokenKind::Tune => {
                let expr = self.parse_tune_expr()?;
                self.consume_semi_opt();
                Ok(V2Stmt::Expr(V2Expr::Tune(expr)))
            }
            TokenKind::Verify => {
                let expr = self.parse_verify_expr()?;
                self.consume_semi_opt();
                Ok(V2Stmt::Expr(V2Expr::Verify(expr)))
            }
            _ => {
                // Try assignment
                if let Some(assign) = self.try_parse_assign()? {
                    return Ok(V2Stmt::Assign(assign));
                }
                let e = self.parse_expr()?;
                self.consume_semi_opt();
                Ok(V2Stmt::Expr(e))
            }
        }
    }

    fn consume_semi_opt(&mut self) {
        if self.peek().kind == TokenKind::Semi {
            self.bump();
        }
    }

    fn try_parse_assign(&mut self) -> Result<Option<V2AssignStmt>, Diagnostic> {
        let saved = self.pos;
        let scopes_next: Vec<u32> = self.scopes.iter().map(|s| s.next).collect();
        let target = match self.parse_assign_target() {
            Ok(t) => t,
            Err(_) => {
                self.pos = saved;
                self.restore_scopes(&scopes_next);
                return Ok(None);
            }
        };
        if self.peek().kind != TokenKind::Eq {
            self.pos = saved;
            self.restore_scopes(&scopes_next);
            return Ok(None);
        }
        self.bump();
        let value = self.parse_expr()?;
        self.consume_semi_opt();
        let id = self.next_id();
        Ok(Some(V2AssignStmt { id, target, value }))
    }

    fn restore_scopes(&mut self, saved: &[u32]) {
        for (s, n) in self.scopes.iter_mut().zip(saved.iter()) {
            s.next = *n;
        }
    }

    fn parse_assign_target(&mut self) -> Result<V2AssignTarget, Diagnostic> {
        let (name, _, _) = self.expect_ident("assignment target")?;
        let mut base = V2Expr::Var {
            id: self.next_id(),
            name: name.clone(),
        };
        loop {
            match self.peek().kind {
                TokenKind::LBracket => {
                    self.bump();
                    let index = self.parse_expr()?;
                    self.expect_kind(&TokenKind::RBracket, "`]`")?;
                    let id = self.next_id();
                    base = V2Expr::Index {
                        id,
                        base: Box::new(base),
                        index: Box::new(index),
                    };
                }
                TokenKind::Dot => {
                    self.bump();
                    let (field, _, _) = self.expect_ident("field name")?;
                    let id = self.next_id();
                    base = V2Expr::Field {
                        id,
                        base: Box::new(base),
                        field,
                    };
                }
                _ => break,
            }
        }
        match base {
            V2Expr::Var { name, .. } => Ok(V2AssignTarget::Var { name }),
            V2Expr::Index { base, index, .. } => Ok(V2AssignTarget::Index { base, index }),
            V2Expr::Field { base, field, .. } => Ok(V2AssignTarget::Field { base, field }),
            _ => {
                let t = self.peek().clone();
                Err(self.err(t.start, t.end, "invalid assignment target"))
            }
        }
    }

    fn parse_let(&mut self) -> Result<V2LetStmt, Diagnostic> {
        let kw = self.expect_kind(&TokenKind::Let, "`let`")?;
        let (name, ns, ne) = self.expect_ident("binding name")?;
        self.expect_kind(&TokenKind::Eq, "`=`")?;
        let value = self.parse_expr()?;
        self.consume_semi_opt();
        let id = self.next_id();
        Ok(V2LetStmt {
            id,
            name,
            name_span: (ns, ne),
            value,
            span: (kw.start, ne),
        })
    }

    fn parse_return(&mut self) -> Result<V2ReturnStmt, Diagnostic> {
        let kw = self.expect_kind(&TokenKind::Return, "`return`")?;
        let value = self.parse_expr()?;
        self.consume_semi_opt();
        let id = self.next_id();
        Ok(V2ReturnStmt {
            id,
            value,
            span: (kw.start, kw.end),
        })
    }

    fn parse_if(&mut self) -> Result<V2IfStmt, Diagnostic> {
        self.expect_kind(&TokenKind::If, "`if`")?;
        let cond = self.parse_expr()?;
        let if_id = self.next_id();
        self.enter(&if_id);
        let then_block = self.parse_block_with_id()?;
        let else_block = if matches!(self.peek().kind, TokenKind::Else) {
            self.bump();
            if matches!(self.peek().kind, TokenKind::If) {
                let blk_id = self.next_id();
                self.enter(&blk_id);
                let inner = self.parse_if()?;
                let stmts = vec![V2Stmt::If(inner)];
                self.exit();
                Some(V2Block { id: blk_id, stmts })
            } else {
                Some(self.parse_block_with_id()?)
            }
        } else {
            None
        };
        self.exit();
        Ok(V2IfStmt {
            id: if_id,
            cond,
            then_block,
            else_block,
        })
    }

    fn parse_print(&mut self) -> Result<V2PrintStmt, Diagnostic> {
        let _kw = self.expect_kind(&TokenKind::Print, "`print`")?;
        let value = if matches!(self.peek().kind, TokenKind::LParen) {
            self.bump();
            let v = self.parse_expr()?;
            self.expect_kind(&TokenKind::RParen, "`)`")?;
            v
        } else {
            self.parse_expr()?
        };
        self.consume_semi_opt();
        let id = self.next_id();
        Ok(V2PrintStmt { id, value })
    }

    fn parse_block_with_id(&mut self) -> Result<V2Block, Diagnostic> {
        self.expect_kind(&TokenKind::LBrace, "`{`")?;
        let blk_id = self.next_id();
        self.enter(&blk_id);
        let mut stmts = Vec::new();
        while !matches!(self.peek().kind, TokenKind::RBrace | TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        self.expect_kind(&TokenKind::RBrace, "`}`")?;
        self.exit();
        Ok(V2Block { id: blk_id, stmts })
    }

    fn parse_tune_expr(&mut self) -> Result<V2TuneExpr, Diagnostic> {
        self.expect_kind(&TokenKind::Tune, "`tune`")?;
        self.expect_kind(&TokenKind::Lt, "`<`")?;
        let target_ty = self.parse_qualified_type("tune target type")?;
        self.expect_kind(&TokenKind::Gt, "`>`")?;
        self.expect_kind(&TokenKind::LParen, "`(`")?;
        let value = self.parse_expr()?;
        self.expect_kind(&TokenKind::RParen, "`)`")?;
        Ok(V2TuneExpr {
            id: self.next_id(),
            target_ty,
            value: Box::new(value),
        })
    }

    fn parse_verify_expr(&mut self) -> Result<V2VerifyExpr, Diagnostic> {
        self.expect_kind(&TokenKind::Verify, "`verify`")?;
        self.expect_kind(&TokenKind::Lt, "`<`")?;
        let target_ty = self.parse_qualified_type("verify target type")?;
        self.expect_kind(&TokenKind::Gt, "`>`")?;
        self.expect_kind(&TokenKind::LParen, "`(`")?;
        let value = self.parse_expr()?;
        self.expect_kind(&TokenKind::RParen, "`)`")?;
        Ok(V2VerifyExpr {
            id: self.next_id(),
            target_ty,
            value: Box::new(value),
        })
    }

    fn parse_expr(&mut self) -> Result<V2Expr, Diagnostic> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<V2Expr, Diagnostic> {
        let mut left = self.parse_and()?;
        while matches!(self.peek().kind, TokenKind::OrOr) {
            self.bump();
            let right = self.parse_and()?;
            let id = self.next_id();
            left = V2Expr::Or {
                id,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<V2Expr, Diagnostic> {
        let mut left = self.parse_equality()?;
        while matches!(self.peek().kind, TokenKind::AndAnd) {
            self.bump();
            let right = self.parse_equality()?;
            let id = self.next_id();
            left = V2Expr::And {
                id,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<V2Expr, Diagnostic> {
        let mut left = self.parse_comparison()?;
        loop {
            match self.peek().kind {
                TokenKind::EqEq => {
                    self.bump();
                    let right = self.parse_comparison()?;
                    let id = self.next_id();
                    left = V2Expr::Eq {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::NotEq => {
                    self.bump();
                    let right = self.parse_comparison()?;
                    let id = self.next_id();
                    left = V2Expr::NotEq {
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

    fn parse_comparison(&mut self) -> Result<V2Expr, Diagnostic> {
        let mut left = self.parse_additive()?;
        loop {
            match self.peek().kind {
                TokenKind::Lt => {
                    self.bump();
                    let right = self.parse_additive()?;
                    let id = self.next_id();
                    left = V2Expr::Lt {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::LtEq => {
                    self.bump();
                    let right = self.parse_additive()?;
                    let id = self.next_id();
                    left = V2Expr::LtEq {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::Gt => {
                    self.bump();
                    let right = self.parse_additive()?;
                    let id = self.next_id();
                    left = V2Expr::Gt {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::GtEq => {
                    self.bump();
                    let right = self.parse_additive()?;
                    let id = self.next_id();
                    left = V2Expr::GtEq {
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

    fn parse_additive(&mut self) -> Result<V2Expr, Diagnostic> {
        let mut left = self.parse_multiplicative()?;
        loop {
            match self.peek().kind {
                TokenKind::Plus => {
                    self.bump();
                    let right = self.parse_multiplicative()?;
                    let id = self.next_id();
                    left = V2Expr::Add {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::Minus => {
                    self.bump();
                    let right = self.parse_multiplicative()?;
                    let id = self.next_id();
                    left = V2Expr::Sub {
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

    fn parse_multiplicative(&mut self) -> Result<V2Expr, Diagnostic> {
        let mut left = self.parse_primary()?;
        loop {
            match self.peek().kind {
                TokenKind::Star => {
                    self.bump();
                    let right = self.parse_primary()?;
                    let id = self.next_id();
                    left = V2Expr::Mul {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::Slash => {
                    self.bump();
                    let right = self.parse_primary()?;
                    let id = self.next_id();
                    left = V2Expr::Div {
                        id,
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                TokenKind::Percent => {
                    self.bump();
                    let right = self.parse_primary()?;
                    let id = self.next_id();
                    left = V2Expr::Mod {
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

    fn parse_primary(&mut self) -> Result<V2Expr, Diagnostic> {
        let t = self.peek().clone();
        match &t.kind {
            TokenKind::IntLit(v) => {
                let v = *v;
                self.bump();
                Ok(V2Expr::Int { id: self.next_id(), value: v })
            }
            TokenKind::StrLit(v) => {
                let v = v.clone();
                self.bump();
                Ok(V2Expr::Str { id: self.next_id(), value: v })
            }
            TokenKind::True => {
                self.bump();
                Ok(V2Expr::Bool { id: self.next_id(), value: true })
            }
            TokenKind::False => {
                self.bump();
                Ok(V2Expr::Bool { id: self.next_id(), value: false })
            }
            TokenKind::Ident(n) if n == "true" => {
                self.bump();
                Ok(V2Expr::Bool { id: self.next_id(), value: true })
            }
            TokenKind::Ident(n) if n == "false" => {
                self.bump();
                Ok(V2Expr::Bool { id: self.next_id(), value: false })
            }
            TokenKind::Ident(_) => {
                let (name, _, _) = self.expect_ident("variable")?;
                if matches!(self.peek().kind, TokenKind::LParen) {
                    self.bump();
                    let mut args = Vec::new();
                    if !matches!(self.peek().kind, TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if matches!(self.peek().kind, TokenKind::Comma) {
                                self.bump();
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect_kind(&TokenKind::RParen, "`)`")?;
                    Ok(V2Expr::Call {
                        id: self.next_id(),
                        func: name,
                        args,
                    })
                } else {
                    Ok(V2Expr::Var { id: self.next_id(), name })
                }
            }
            TokenKind::Listen => {
                self.bump();
                self.expect_kind(&TokenKind::LParen, "`(`")?;
                let (handle, _, _) = self.expect_ident("echo handle")?;
                self.expect_kind(&TokenKind::RParen, "`)`")?;
                Ok(V2Expr::Listen {
                    id: self.next_id(),
                    handle,
                })
            }
            TokenKind::Flow => {
                let flow = self.parse_flow_expr()?;
                Ok(V2Expr::Flow(flow))
            }
            TokenKind::Tune => {
                let t = self.parse_tune_expr()?;
                Ok(V2Expr::Tune(t))
            }
            TokenKind::Verify => {
                let v = self.parse_verify_expr()?;
                Ok(V2Expr::Verify(v))
            }
            TokenKind::LParen => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect_kind(&TokenKind::RParen, "`)`")?;
                Ok(e)
            }
            TokenKind::LBrace => {
                // Struct/Map literal - simplified for now
                self.bump();
                let mut entries = Vec::new();
                if !matches!(self.peek().kind, TokenKind::RBrace) {
                    loop {
                        let (key, _, _) = self.expect_ident("field/key name")?;
                        self.expect_kind(&TokenKind::Colon, "`:`")?;
                        let value = self.parse_expr()?;
                        entries.push((key, value));
                        if matches!(self.peek().kind, TokenKind::Comma) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.expect_kind(&TokenKind::RBrace, "`}`")?;
                Ok(V2Expr::StructLit {
                    id: self.next_id(),
                    name: "Struct".to_string(), // Simplified
                    fields: entries,
                })
            }
            _ => Err(self.err(t.start, t.end, "expected expression")),
        }
    }
}

/// Parse a single v2 expression (used by the v2 interpreter for flow bodies).
pub fn parse_v2_expr(source: &str) -> Result<V2Expr, Diagnostic> {
    let toks = lex(source).map_err(|e| {
        Diagnostic::error(
            "E-PARSE-V2",
            &format!("lex error: {}", e.message),
            FILE,
            e.offset,
            e.offset,
            "input does not match the v2 grammar",
            &["check flow body spelling"],
            "syntax/v2",
        )
    })?;
    let mut p = P::new(toks, source.to_string());
    let e = p.parse_expr()?;
    Ok(e)
}

pub fn parse_v2_program(source: &str) -> Result<crate::ast::v2::V2Program, Diagnostic> {
    let toks = lex(source).map_err(|e| {
        Diagnostic::error(
            "E-PARSE-V2",
            &format!("lex error: {}", e.message),
            FILE,
            e.offset,
            e.offset,
            "input does not match the v2 grammar",
            &["check schema echo flow tune listen spelling"],
            "syntax/v2",
        )
    })?;
    let mut p = P::new(toks, source.to_string());
    while !matches!(p.peek().kind, TokenKind::Eof) {
        match p.peek().kind {
            TokenKind::Schema => {
                let decl = p.parse_schema_decl()?;
                p.schemas.push(decl);
            }
            TokenKind::Echo => {
                let decl = p.parse_echo_decl()?;
                p.echo_fns.push(decl);
            }
            TokenKind::Ident(_) => {
                // Check if it's "fn" for function declarations
                let t = p.peek().clone();
                if let TokenKind::Ident(n) = &t.kind {
                    if n == "fn" {
                        let decl = p.parse_function()?;
                        p.functions.push(decl);
                        continue;
                    }
                }
                let t = p.peek().clone();
                return Err(p.err(t.start, t.end, "expected schema, echo fn, or function"));
            }
            TokenKind::Flow => {
                let decl = p.parse_flow_decl()?;
                p.flows.push(decl);
            }
            _ => {
                let t = p.peek().clone();
                return Err(p.err(t.start, t.end, "expected schema, echo fn, or function"));
            }
        }
    }
    Ok(crate::ast::v2::V2Program {
        schemas: p.schemas,
        echo_fns: p.echo_fns,
        echo_bodies: p.echo_bodies,
        flows: p.flows,
        functions: p.functions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_parse_simple_program() {
        let src = r#"
schema Profile "1" {
    name: str
}

echo fn fetch_profile(name: str) -> !Profile {
    tune<Profile>({name: name})
}

fn main() -> str {
    let profile = fetch_profile("ada")
    let h = listen(profile)
    return "ok"
}
"#;
        let prog = parse_v2_program(src).expect("parses");
        assert_eq!(prog.schemas.len(), 1);
        assert_eq!(prog.schemas[0].name, "Profile");
        assert_eq!(prog.echo_fns.len(), 1);
        assert_eq!(prog.echo_fns[0].name, "fetch_profile");
        assert_eq!(prog.functions.len(), 1);
        assert_eq!(prog.functions[0].name, "main");
    }

    #[test]
    fn v2_parse_flow_in_function() {
        let src = r#"
fn classify(score: i32) -> str {
    let f = flow(score: i32, dep=threshold: i32) -> str { score >= threshold }
    return f(85)
}
"#;
        let prog = parse_v2_program(src).expect("parses");
        assert_eq!(prog.functions.len(), 1);
    }
}
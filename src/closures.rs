//! Closures (KLANG-FOUNDATION-2 Step 3): anonymous function values.
//!
//! A closure literal `fn(x: i32) -> i32 { return x * 2 }` is an
//! expression evaluating to a first-class value that captures its
//! enclosing scope **by value**: the captured bindings are snapshotted
//! when the literal runs, so later mutations of the outer variables —
//! including the outer function having returned — never affect the
//! closure. This matches Klang's "Managed mode only, no real borrow
//! checker" model: copy semantics, never aliasing.
//!
//! Compilation strategy (lambda lifting over existing ops, no new
//! runtime semantics beyond one value variant):
//! - Each literal lowers to a synthetic [`crate::mir`] function whose
//!   name ([`closure_fn_name`]) contains a NUL byte, which no user
//!   identifier can spell, so collisions are impossible by construction.
//!   Its leading parameters are the captures (same order as
//!   [`closure_captures`]), followed by the declared parameters.
//! - At the literal site a `ClosureNew` op snapshots the captures into
//!   a `Value::Closure`; calling through a closure-valued variable
//!   dispatches dynamically in the existing `Call` op.
//! - Anything the int-only JIT cannot represent (`ClosureNew`, dynamic
//!   dispatch) is a loud rejection, never silent wrong code.

use std::collections::BTreeSet;

use crate::ast::{Block, Expr, NodeId, Param, Stmt};

/// Synthetic MIR/HIR name for the lifted function of one closure literal.
/// The NUL byte is unrepresentable in Klang identifiers (the v1 lexer
/// rejects it), so no user function can ever collide with this.
pub fn closure_fn_name(id: &NodeId) -> String {
    let mut out = String::from("__closure\0");
    for (i, seg) in id.path.iter().enumerate() {
        if i > 0 {
            out.push('_');
        }
        out.push_str(&seg.to_string());
    }
    out
}

/// One closure literal found in a body (cloned out for the HIR pre-pass
/// and MIR synthesis, which both need owned data).
#[derive(Debug, Clone)]
pub struct ClosureInfo {
    pub id: NodeId,
    pub params: Vec<Param>,
    pub return_ty: String,
    pub body: Block,
}

/// Collect every closure literal in `block`, including ones nested inside
/// other closures, match arms, and blocks.
pub fn collect_closures(block: &Block, out: &mut Vec<ClosureInfo>) {
    for s in &block.stmts {
        collect_closures_stmt(s, out);
    }
}

fn collect_closures_stmt(s: &Stmt, out: &mut Vec<ClosureInfo>) {
    match s {
        Stmt::Let(l) => collect_closures_expr(&l.value, out),
        Stmt::Assign(a) => {
            match &a.target {
                crate::ast::AssignTarget::Var { .. } => {}
                crate::ast::AssignTarget::Index { base, index } => {
                    collect_closures_expr(base, out);
                    collect_closures_expr(index, out);
                }
                crate::ast::AssignTarget::Field { base, .. } => {
                    collect_closures_expr(base, out)
                }
            }
            collect_closures_expr(&a.value, out);
        }
        Stmt::Return(r) => collect_closures_expr(&r.value, out),
        Stmt::TaskGroup(g) => collect_closures(&g.body, out),
        Stmt::If(i) => {
            collect_closures_expr(&i.cond, out);
            collect_closures(&i.then_block, out);
            if let Some(e) = &i.else_block {
                collect_closures(e, out);
            }
        }
        Stmt::Print(p) => collect_closures_expr(&p.value, out),
        Stmt::While(w) => {
            collect_closures_expr(&w.cond, out);
            collect_closures(&w.body, out);
        }
        Stmt::ForRange(fr) => {
            collect_closures_expr(&fr.start, out);
            collect_closures_expr(&fr.end, out);
            collect_closures(&fr.body, out);
        }
        Stmt::ForIn(fi) => {
            collect_closures_expr(&fi.iter, out);
            collect_closures(&fi.body, out);
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::TryCatch(t) => {
            collect_closures(&t.body, out);
            collect_closures(&t.handler, out);
        }
        Stmt::Expr(e) => collect_closures_expr(e, out),
    }
}

fn collect_closures_expr(e: &Expr, out: &mut Vec<ClosureInfo>) {
    match e {
        Expr::Closure {
            id,
            params,
            return_ty,
            body,
        } => {
            out.push(ClosureInfo {
                id: id.clone(),
                params: params.clone(),
                return_ty: return_ty.clone(),
                body: body.clone(),
            });
            collect_closures(body, out);
        }
        Expr::ArrayLit { elems, .. } => {
            for el in elems {
                collect_closures_expr(el, out);
            }
        }
        Expr::MapLit { entries, .. } => {
            for (_, v) in entries {
                collect_closures_expr(v, out);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields {
                collect_closures_expr(v, out);
            }
        }
        Expr::EnumCtor { args, .. } => {
            for a in args {
                collect_closures_expr(a, out);
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            collect_closures_expr(scrutinee, out);
            for arm in arms {
                for s in &arm.stmts {
                    collect_closures_stmt(s, out);
                }
                if let Some(g) = &arm.guard {
                    collect_closures_expr(g, out);
                }
                collect_closures_expr(&arm.body, out);
            }
        }
        Expr::Index { base, index, .. } => {
            collect_closures_expr(base, out);
            collect_closures_expr(index, out);
        }
        Expr::Field { base, .. } => collect_closures_expr(base, out),
        Expr::MethodCall { base, args, .. } => {
            collect_closures_expr(base, out);
            for a in args {
                collect_closures_expr(a, out);
            }
        }
        Expr::Spawn { call, .. } => collect_closures_expr(call, out),
        Expr::Call { args, .. } => {
            for a in args {
                collect_closures_expr(a, out);
            }
        }
        Expr::Add { left, right, .. }
        | Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. }
        | Expr::Eq { left, right, .. }
        | Expr::NotEq { left, right, .. }
        | Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. }
        | Expr::And { left, right, .. }
        | Expr::Or { left, right, .. } => {
            collect_closures_expr(left, out);
            collect_closures_expr(right, out);
        }
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => {
            collect_closures_expr(inner, out)
        }
        Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Str { .. }
        | Expr::Bool { .. }
        | Expr::Var { .. }
        | Expr::Await { .. } => {}
    }
}

/// Names the closure must snapshot from its enclosing scope, sorted for
/// determinism: every free variable of the body (used but neither a
/// declared parameter nor bound by a `let`/`for`/`catch`/match binding
/// inside the body). Call-target names count as uses (a closure-valued
/// variable is called dynamically), while static function names captured
/// along the way lower to harmless snapshots that the lifted body never
/// reads. Nested closures propagate their own free variables outward:
/// names they need must be present in the outer frame at creation.
pub fn closure_captures(params: &[Param], body: &Block) -> Vec<String> {
    let mut bound: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
    let mut uses: BTreeSet<String> = BTreeSet::new();
    free_vars_block(body, &mut bound, &mut uses);
    for p in params {
        uses.remove(&p.name);
    }
    uses.into_iter().collect()
}

fn free_vars_block(block: &Block, bound: &mut Vec<String>, uses: &mut BTreeSet<String>) {
    for s in &block.stmts {
        free_vars_stmt(s, bound, uses);
    }
}

fn free_vars_stmt(s: &Stmt, bound: &mut Vec<String>, uses: &mut BTreeSet<String>) {
    match s {
        Stmt::Let(l) => {
            free_vars_expr(&l.value, bound, uses);
            if !bound.contains(&l.name) {
                bound.push(l.name.clone());
            }
        }
        Stmt::Assign(a) => {
            match &a.target {
                crate::ast::AssignTarget::Var { name } => {
                    if !bound.contains(name) {
                        uses.insert(name.clone());
                    }
                }
                crate::ast::AssignTarget::Index { base, index } => {
                    free_vars_expr(base, bound, uses);
                    free_vars_expr(index, bound, uses);
                }
                crate::ast::AssignTarget::Field { base, .. } => {
                    free_vars_expr(base, bound, uses)
                }
            }
            free_vars_expr(&a.value, bound, uses);
        }
        Stmt::Return(r) => free_vars_expr(&r.value, bound, uses),
        Stmt::TaskGroup(g) => free_vars_block(&g.body, bound, uses),
        Stmt::If(i) => {
            free_vars_expr(&i.cond, bound, uses);
            let mut then_bound = bound.clone();
            free_vars_block(&i.then_block, &mut then_bound, uses);
            if let Some(e) = &i.else_block {
                let mut else_bound = bound.clone();
                free_vars_block(e, &mut else_bound, uses);
            }
        }
        Stmt::Print(p) => free_vars_expr(&p.value, bound, uses),
        Stmt::While(w) => {
            free_vars_expr(&w.cond, bound, uses);
            let mut inner = bound.clone();
            free_vars_block(&w.body, &mut inner, uses);
        }
        Stmt::ForRange(fr) => {
            free_vars_expr(&fr.start, bound, uses);
            free_vars_expr(&fr.end, bound, uses);
            let mut inner = bound.clone();
            inner.push(fr.var.clone());
            free_vars_block(&fr.body, &mut inner, uses);
        }
        Stmt::ForIn(fi) => {
            free_vars_expr(&fi.iter, bound, uses);
            let mut inner = bound.clone();
            inner.push(fi.var.clone());
            free_vars_block(&fi.body, &mut inner, uses);
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::TryCatch(t) => {
            let mut inner = bound.clone();
            free_vars_block(&t.body, &mut inner, uses);
            let mut handler = bound.clone();
            handler.push(t.var.clone());
            free_vars_block(&t.handler, &mut handler, uses);
        }
        Stmt::Expr(e) => free_vars_expr(e, bound, uses),
    }
}

fn free_vars_expr(e: &Expr, bound: &[String], uses: &mut BTreeSet<String>) {
    match e {
        Expr::Var { name, .. } => {
            if !bound.contains(name) {
                uses.insert(name.clone());
            }
        }
        Expr::Closure {
            params, body, ..
        } => {
            // The nested literal's own free variables are uses at this
            // creation site (the outer frame must provide them); names
            // bound inside the nested body stay local via a cloned scope.
            let mut inner: Vec<String> = bound.to_vec();
            for p in params {
                if !inner.contains(&p.name) {
                    inner.push(p.name.clone());
                }
            }
            free_vars_block(body, &mut inner, uses);
        }
        Expr::ArrayLit { elems, .. } => {
            for el in elems {
                free_vars_expr(el, bound, uses);
            }
        }
        Expr::MapLit { entries, .. } => {
            for (_, v) in entries {
                free_vars_expr(v, bound, uses);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields {
                free_vars_expr(v, bound, uses);
            }
        }
        Expr::EnumCtor { args, .. } => {
            for a in args {
                free_vars_expr(a, bound, uses);
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            free_vars_expr(scrutinee, bound, uses);
            for arm in arms {
                let mut arm_bound: Vec<String> = bound.to_vec();
                arm_bound.extend(arm.bindings.iter().cloned());
                for s in &arm.stmts {
                    free_vars_stmt(s, &mut arm_bound, uses);
                }
                if let Some(g) = &arm.guard {
                    free_vars_expr(g, &arm_bound, uses);
                }
                free_vars_expr(&arm.body, &arm_bound, uses);
            }
        }
        Expr::Index { base, index, .. } => {
            free_vars_expr(base, bound, uses);
            free_vars_expr(index, bound, uses);
        }
        Expr::Field { base, .. } => free_vars_expr(base, bound, uses),
        Expr::MethodCall { base, args, .. } => {
            free_vars_expr(base, bound, uses);
            for a in args {
                free_vars_expr(a, bound, uses);
            }
        }
        Expr::Spawn { call, .. } => free_vars_expr(call, bound, uses),
        Expr::Await { name, .. } => {
            if !bound.contains(name) {
                uses.insert(name.clone());
            }
        }
        Expr::Call { func, args, .. } => {
            // The callee may be a closure-valued variable (dynamic
            // dispatch reads it from scope), so it counts as a use; a
            // static function name captured alongside is a harmless extra
            // snapshot the lifted body never reads.
            if !bound.contains(func) {
                uses.insert(func.clone());
            }
            for a in args {
                free_vars_expr(a, bound, uses);
            }
        }
        Expr::Add { left, right, .. }
        | Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. }
        | Expr::Eq { left, right, .. }
        | Expr::NotEq { left, right, .. }
        | Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. }
        | Expr::And { left, right, .. }
        | Expr::Or { left, right, .. } => {
            free_vars_expr(left, bound, uses);
            free_vars_expr(right, bound, uses);
        }
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => {
            free_vars_expr(inner, bound, uses)
        }
        Expr::Int { .. } | Expr::Float { .. } | Expr::Str { .. } | Expr::Bool { .. } => {}
    }
}

/// Split a canonical closure type annotation (`fn(i32, str)->i32`,
/// exactly as [`crate::parser`] emits it) into parameter annotations and
/// the return annotation. Returns `None` for anything else (including
/// malformed shapes, which resolve leniently to `Unknown` like every
/// other unknown annotation). Commas inside nested `fn(...)` parameter
/// lists and `<...>` generic arguments never split.
pub fn split_closure_ty(s: &str) -> Option<(Vec<String>, String)> {
    let mut rest = s.strip_prefix("fn(")?;
    // Find the paren matching `fn(`.
    let mut depth: usize = 1;
    let mut idx = 0;
    let bytes = rest.as_bytes();
    while idx < bytes.len() {
        match bytes[idx] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        idx += 1;
    }
    if depth != 0 {
        return None;
    }
    let params_str = &rest[..idx];
    rest = &rest[idx + 1..];
    let ret = rest.strip_prefix("->")?;
    if ret.is_empty() {
        return None;
    }
    let mut params = Vec::new();
    if !params_str.is_empty() {
        let mut start = 0;
        let mut pdepth: usize = 0;
        let mut adepth: usize = 0;
        let pb = params_str.as_bytes();
        let mut i = 0;
        while i <= pb.len() {
            let end = i == pb.len()
                || (pb[i] == b',' && pdepth == 0 && adepth == 0);
            if end {
                let part = params_str[start..i].trim();
                if part.is_empty() {
                    return None;
                }
                params.push(part.to_string());
                start = i + 1;
            } else {
                match pb[i] {
                    b'(' => pdepth += 1,
                    b')' => pdepth = pdepth.saturating_sub(1),
                    b'<' => adepth += 1,
                    b'>' => adepth = adepth.saturating_sub(1),
                    _ => {}
                }
            }
            i += 1;
        }
    }
    Some((params, ret.to_string()))
}

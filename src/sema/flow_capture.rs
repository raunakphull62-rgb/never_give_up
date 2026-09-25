//! Klang v2 Flow capture checking (Phase 7).
//!
//! Design note (chosen rule): reads of **immutable** outer bindings need
//! no `dep=`; reads of **mutable** outer bindings must list the name in
//! `dep=`, or checking fails with `E-FLOW-MUTABLE-CAPTURE` naming the
//! missing dependency and its body span. Adding a new mutable read is a
//! compile error, never an automatic signature rewrite. Unknown names are
//! ignored (dynamic/later passes own them).

use crate::ast::flow::FlowExpr;
use crate::ast::resonance::QualifiedType;
use crate::diagnostics::Diagnostic;
use crate::lexer::lex;
use crate::lexer::resonance::TokenKind;
use std::collections::HashSet;

/// Outer environment split by mutability.
#[derive(Debug, Clone, Default)]
pub struct FlowCheckEnv {
    /// Mutable outer bindings (require `dep=`).
    pub mutable: HashSet<String>,
    /// Immutable outer bindings (free to read).
    pub immutable: HashSet<String>,
}

impl FlowCheckEnv {
    /// Build an environment from name lists.
    pub fn new(mutable: &[&str], immutable: &[&str]) -> Self {
        Self {
            mutable: mutable.iter().map(|s| s.to_string()).collect(),
            immutable: immutable.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// One outer read in a Flow body: `(name, start, end)` with body-relative
/// byte spans.
pub fn body_reads(body: &str) -> Vec<(String, usize, usize)> {
    let toks = match lex(body) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for t in toks {
        if let TokenKind::Ident(n) = &t.kind {
            // `fn` is syntax, not a read; everything else counts.
            if n == "fn" {
                continue;
            }
            out.push((n.clone(), t.start, t.end));
        }
    }
    out
}

/// Check one Flow: signature first, then explicit-dependency capture.
pub fn check_flow(flow: &FlowExpr, env: &FlowCheckEnv, file: &str) -> Result<(), Diagnostic> {
    check_signature(flow, file)?;
    let locals: HashSet<&str> = flow
        .params
        .iter()
        .map(|p| p.name.as_str())
        .chain(flow.deps.iter().map(|d| d.name.as_str()))
        .collect();
    for (name, s, e) in body_reads(&flow.body) {
        if locals.contains(name.as_str()) {
            continue;
        }
        if flow.has_dep(&name) {
            continue;
        }
        if env.mutable.contains(&name) {
            return Err(crate::ai_safety::diagnostics::flow_mutable_capture(
                file, s, e, &name,
            ));
        }
        // Immutable and unknown names need no dependency.
    }
    Ok(())
}

/// Signature checks: non-empty names/types, no duplicate dependencies.
fn check_signature(flow: &FlowExpr, file: &str) -> Result<(), Diagnostic> {
    let mut seen: HashSet<&str> = HashSet::new();
    for d in &flow.deps {
        if !seen.insert(d.name.as_str()) {
            return Err(crate::ai_safety::diagnostics::flow_duplicate_dependency(
                file, 0, 0, &d.name,
            ));
        }
    }
    for p in &flow.params {
        type_is_well_formed(&p.ty, file)?;
    }
    for d in &flow.deps {
        type_is_well_formed(&d.ty, file)?;
    }
    if let Some(r) = &flow.return_ty {
        type_is_well_formed(r, file)?;
    }
    Ok(())
}

fn type_is_well_formed(ty: &str, file: &str) -> Result<(), Diagnostic> {
    let q = QualifiedType::parse(ty);
    if q.base.is_empty() {
        return Err(Diagnostic::error(
            "E-RESONANCE-MISMATCH",
            &format!("flow type `{ty}` names no base type"),
            file,
            0,
            0,
            "flow argument and return types must name a type",
            &["write a type such as i32, str, ?Data, or !Data"],
            "resonance/mismatch",
        ));
    }
    Ok(())
}

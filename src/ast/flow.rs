//! Klang v2 Flow syntax nodes (Phase 3).
//!
//! A Flow has an explicit parameter/dependency environment: mutable outer
//! state is visible only when listed as `dep=name: Type`. No semantic
//! checking lives here; that arrives in `sema::flow_capture` (Phase 7).

use super::NodeId;

/// One ordinary Flow parameter (`score: i32`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowParam {
    /// Parameter name.
    pub name: String,
    /// Declared type name.
    pub ty: String,
}

/// One explicit dependency (`dep=threshold: i32`).
///
/// Stored separately from [`FlowParam`] so later passes and diagnostics
/// can name the missing dependency instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowDependency {
    /// Outer binding being imported.
    pub name: String,
    /// Expected type of the outer binding.
    pub ty: String,
}

/// A Flow value expression: `flow(params, dep=...) -> Ret { body }`.
///
/// `body` is the raw body source in Phase 3; the Phase 4 parser fills it
/// from real syntax and Phase 7 checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowExpr {
    /// Stable identity of the Flow literal.
    pub id: NodeId,
    /// Ordinary parameters.
    pub params: Vec<FlowParam>,
    /// Explicit dependencies (never implicit).
    pub deps: Vec<FlowDependency>,
    /// Declared return type, if any.
    pub return_ty: Option<String>,
    /// Body source (parsed into statements starting in Phase 4).
    pub body: String,
}

/// A named Flow binding (`let classify = flow(...) ...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowDecl {
    /// Stable identity of the declaration.
    pub id: NodeId,
    /// Bound name, if let-bound.
    pub name: Option<String>,
    /// The Flow value.
    pub expr: FlowExpr,
}

impl FlowExpr {
    /// Dependency names in declaration order.
    pub fn dep_names(&self) -> Vec<&str> {
        self.deps.iter().map(|d| d.name.as_str()).collect()
    }

    /// True when `name` is an explicit dependency.
    pub fn has_dep(&self, name: &str) -> bool {
        self.deps.iter().any(|d| d.name == name)
    }
}

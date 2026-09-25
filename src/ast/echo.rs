//! Klang v2 Echo syntax nodes (Phase 3).
//!
//! `echo fn` declares background work and returns a typed handle; `listen`
//! retrieves it. Ownership (`created`/`listened`/`transferred`/`joined`) is
//! tracked by `sema::echo_lifetime` (Phase 8); this module only defines the
//! syntax shapes.

use super::NodeId;

/// How an Echo handle is owned at a program point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EchoOwnership {
    /// Just created by calling an `echo fn`.
    Created,
    /// Consumed by `listen`.
    Listened,
    /// Moved to an owner that will listen.
    Transferred,
    /// Joined by a structured scope.
    Joined,
}

/// `echo fn fetch(id: i64) -> !Data { ... }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoDecl {
    /// Stable identity of the declaration.
    pub id: NodeId,
    /// Function name being echoed.
    pub name: String,
    /// Parameter list (`name: Type` pairs).
    pub params: Vec<(String, String)>,
    /// Declared return type name.
    pub return_ty: String,
}

/// The typed handle `Echo<T>` produced by calling an `echo fn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoHandleType {
    /// Inner value type name (e.g. `!Data`).
    pub inner: String,
}

impl EchoHandleType {
    /// Render `Echo<inner>`.
    pub fn display(&self) -> String {
        format!("Echo<{}>", self.inner)
    }
}

/// `listen(handle)` expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenExpr {
    /// Stable identity of the listen site.
    pub id: NodeId,
    /// Handle being listened to.
    pub handle: String,
}

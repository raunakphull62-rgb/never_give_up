//! Klang v2 semantic analysis (Phase 6).
//!
//! Resonance checking ([`tuner`]), schema boundaries ([`schema_check`]);
//! Flow capture and Echo lifetimes arrive in Phases 7-8.

/// Echo handle lifetimes: created/listened/transferred/joined.
pub mod echo_lifetime;
/// Flow explicit-dependency capture checking.
pub mod flow_capture;
/// Schema-boundary checking.
pub mod schema_check;
/// `?T`/`T`/`!T` compatibility and `tune` proof creation.
pub mod tuner;

use crate::diagnostics::Diagnostic;

/// v2 semantic error: one or more structured diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemaError {
    /// Structured diagnostics explaining the failure.
    pub diagnostics: Vec<Diagnostic>,
}

/// v2 semantic result placeholder.
pub type Result<T> = std::result::Result<T, SemaError>;

//! Klang v2 heap policy and reference counting (Phase 8).
//!
//! [`rc`] cells release deterministically; [`CyclePolicy::Report`] is the
//! standing rule: cycles are surfaced via counters/diagnostics, never
//! hidden.

/// Reference-counted heap cells.
pub mod rc;

/// Heap-cycle policy for v2 reference counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CyclePolicy {
    /// Report cycles via diagnostics/debug counters; never hide them.
    Report,
}

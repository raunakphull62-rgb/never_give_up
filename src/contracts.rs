//! Stage 6: AI-legible diagnostics, contracts, fix-it protocol (foundation).
//!
//! Diagnostics are already structured objects (`diagnostics.rs`). Here:
//! executable preconditions, a machine-applicable fix protocol, and a
//! bounded repair loop.

use crate::diagnostics::Diagnostic;

/// One executable precondition on a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub on_function: String,
    pub precondition: String,
    pub message: String,
}

impl Contract {
    pub fn check(&self, arg_summary: &str) -> Result<(), Diagnostic> {
        // v0.1: only `non-empty` precondition is executable; everything else
        // passes and is recorded for later stages.
        if self.precondition == "non-empty" && arg_summary.is_empty() {
            Err(Diagnostic::error(
                "E-CONTRACT",
                &self.message,
                "input.warden",
                0,
                0,
                "contract precondition violated",
                &["provide a non-empty argument"],
                "contracts/precondition",
            ))
        } else {
            Ok(())
        }
    }
}

/// One machine-applicable fix for a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixIt {
    pub for_code: String,
    pub label: String,
    pub edit_hint: String,
}

impl FixIt {
    pub fn for_diagnostic(d: &Diagnostic) -> Vec<Self> {
        d.fixes
            .iter()
            .map(|f| Self {
                for_code: d.code.clone(),
                label: f.label.clone(),
                edit_hint: format!("apply fix for {}: {}", d.code, f.label),
            })
            .collect()
    }
}

/// Bounded repair loop: run `attempt` at most `max_iters` times, stopping
/// at the first `Ok`. Returns the last error when the budget is exhausted.
/// A zero budget tries nothing and succeeds vacuously.
pub fn repair_loop<E, F>(max_iters: u32, mut attempt: F) -> Result<(), E>
where
    F: FnMut(u32) -> Result<(), E>,
{
    let mut last: Option<E> = None;
    for i in 0..max_iters {
        match attempt(i) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

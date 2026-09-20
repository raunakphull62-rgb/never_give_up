//! Stage 7: gradual ownership + optimization modes (foundation).
//!
//! v0.1 ships Managed mode only. Other modes are explicit, checked, and
//! rejected with a diagnostic until their stage lands.

use crate::diagnostics::Diagnostic;

/// Execution/memory mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Managed,
    Value,
    Owned,
    UnsafeFfi,
}

impl Mode {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Value => "value",
            Self::Owned => "owned",
            Self::UnsafeFfi => "unsafe",
        }
    }
}

/// Check a resource type declaration. Only Managed passes in v0.1.
pub fn check_resource(name: &str, mode: Mode) -> Result<(), Diagnostic> {
    match mode {
        Mode::Managed => Ok(()),
        other => Err(Diagnostic::error(
            "E-OWNERSHIP-MODE",
            &format!(
                "resource `{name}` uses `{}`, only `managed` ships in v0.1",
                other.name()
            ),
            "input.warden",
            0,
            0,
            "ownership mode deferred past v0.1",
            &["use managed mode"],
            "ownership/mode",
        )),
    }
}

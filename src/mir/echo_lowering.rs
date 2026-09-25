//! Klang v2 Echo MIR lowering (Phase 9).
//!
//! Echoes lower to explicit state-machine nodes — never to an opaque
//! `async`/`await` construct. Both terminal paths (`Completed`, `Failed`)
//! end in `Cleaned`, which releases or transfers the result cell, so no
//! path leaks ownership.

/// One explicit Echo lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EchoState {
    /// Handle created, work not yet started.
    Created,
    /// Work running.
    Running,
    /// Work suspended at an explicit checkpoint.
    Suspended,
    /// Work finished with a value (pre-cleanup terminal).
    Completed,
    /// Work finished with a failure (pre-cleanup terminal).
    Failed,
    /// Resources released or transferred (final on every path).
    Cleaned,
}

impl EchoState {
    /// True for the two pre-cleanup terminals.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// One explicit MIR operation on an Echo handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EchoMirOp {
    /// Allocate the handle and result cell.
    Create {
        /// Handle name.
        handle: String,
        /// Inner value type (e.g. `!Data`).
        inner: String,
    },
    /// Start the work.
    Start {
        /// Handle name.
        handle: String,
    },
    /// Suspend at an explicit checkpoint.
    Suspend {
        /// Handle name.
        handle: String,
    },
    /// Resume after suspension.
    Resume {
        /// Handle name.
        handle: String,
    },
    /// Store the success value.
    Complete {
        /// Handle name.
        handle: String,
    },
    /// Store the failure cause (lossless).
    Fail {
        /// Handle name.
        handle: String,
        /// Failure cause.
        cause: String,
    },
    /// Retrieve the outcome (value or failure).
    Listen {
        /// Handle name.
        handle: String,
    },
    /// Release or transfer the cell (final on every path).
    Cleanup {
        /// Handle name.
        handle: String,
    },
}

/// One lowered node: the state after applying `op`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoMirNode {
    /// State after `op`.
    pub state: EchoState,
    /// Operation applied.
    pub op: EchoMirOp,
}

fn node(state: EchoState, op: EchoMirOp) -> EchoMirNode {
    EchoMirNode { state, op }
}

/// Lower the success path for `handle` carrying `inner`.
pub fn lower_success(handle: &str, inner: &str) -> Vec<EchoMirNode> {
    vec![
        node(
            EchoState::Created,
            EchoMirOp::Create {
                handle: handle.to_string(),
                inner: inner.to_string(),
            },
        ),
        node(
            EchoState::Running,
            EchoMirOp::Start {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Suspended,
            EchoMirOp::Suspend {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Running,
            EchoMirOp::Resume {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Completed,
            EchoMirOp::Complete {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Completed,
            EchoMirOp::Listen {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Cleaned,
            EchoMirOp::Cleanup {
                handle: handle.to_string(),
            },
        ),
    ]
}

/// Lower the failure path: the cause is stored and delivered, then the
/// cell is still cleaned up.
pub fn lower_failure(handle: &str, inner: &str, cause: &str) -> Vec<EchoMirNode> {
    vec![
        node(
            EchoState::Created,
            EchoMirOp::Create {
                handle: handle.to_string(),
                inner: inner.to_string(),
            },
        ),
        node(
            EchoState::Running,
            EchoMirOp::Start {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Suspended,
            EchoMirOp::Suspend {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Running,
            EchoMirOp::Resume {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Failed,
            EchoMirOp::Fail {
                handle: handle.to_string(),
                cause: cause.to_string(),
            },
        ),
        node(
            EchoState::Failed,
            EchoMirOp::Listen {
                handle: handle.to_string(),
            },
        ),
        node(
            EchoState::Cleaned,
            EchoMirOp::Cleanup {
                handle: handle.to_string(),
            },
        ),
    ]
}

/// True when the path ends in `Cleaned` (cleanup or ownership transfer).
pub fn ends_with_cleanup(nodes: &[EchoMirNode]) -> bool {
    matches!(
        nodes.last(),
        Some(EchoMirNode {
            state: EchoState::Cleaned,
            op: EchoMirOp::Cleanup { .. }
        })
    )
}

/// Verify a lowered path is lossless: every `Fail` must reach a later
/// `Listen` for the same handle, and the path must end with `Cleanup`.
/// Violations are `E-ECHO-FAILURE-LOST` naming the handle.
pub fn verify_lossless(
    nodes: &[EchoMirNode],
    file: &str,
) -> Result<(), crate::diagnostics::Diagnostic> {
    for (i, n) in nodes.iter().enumerate() {
        if let EchoMirOp::Fail { handle, .. } = &n.op {
            let heard = nodes[i..].iter().any(|m| match &m.op {
                EchoMirOp::Listen { handle: h } => h == handle,
                _ => false,
            });
            if !heard {
                return Err(crate::ai_safety::diagnostics::echo_failure_lost(
                    file, 0, 0, handle,
                ));
            }
        }
    }
    if !ends_with_cleanup(nodes) {
        let handle = nodes
            .iter()
            .find_map(|n| match &n.op {
                EchoMirOp::Create { handle, .. } => Some(handle.clone()),
                _ => None,
            })
            .unwrap_or_default();
        return Err(crate::ai_safety::diagnostics::echo_failure_lost(
            file, 0, 0, &handle,
        ));
    }
    Ok(())
}

/// Render a path for snapshot tests (contains no `async`/`await`).
pub fn render(nodes: &[EchoMirNode]) -> String {
    nodes
        .iter()
        .map(|n| format!("{:?}/{:?}", n.state, n.op))
        .collect::<Vec<_>>()
        .join("\n")
}

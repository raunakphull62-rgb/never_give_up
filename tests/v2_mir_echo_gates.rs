//! Phase 9 — v2 MIR echo gates: explicit states, cleanup, determinism.

use klang::mir::echo_lowering::{
    ends_with_cleanup, lower_failure, lower_success, render, EchoMirOp, EchoState,
};

#[test]
fn v2_mir_echo_success_states() {
    let nodes = lower_success("pending", "!Data");
    let states: Vec<EchoState> = nodes.iter().map(|n| n.state).collect();
    assert_eq!(
        states,
        vec![
            EchoState::Created,
            EchoState::Running,
            EchoState::Suspended,
            EchoState::Running,
            EchoState::Completed,
            EchoState::Completed,
            EchoState::Cleaned,
        ]
    );
    // Every terminal state is followed by cleanup/transfer.
    assert!(ends_with_cleanup(&nodes));
    // No opaque async/await construct anywhere in v2 Echo MIR.
    let text = render(&nodes);
    assert!(!text.contains("async"));
    assert!(!text.contains("await"));
    assert!(text.contains("Cleanup"));
}

#[test]
fn v2_mir_echo_failure_lossless() {
    let nodes = lower_failure("pending", "?Data", "boom");
    assert!(nodes.iter().any(|n| n.state == EchoState::Failed));
    assert!(ends_with_cleanup(&nodes));
    // The cause survives lowering.
    let fail = nodes
        .iter()
        .find_map(|n| match &n.op {
            EchoMirOp::Fail { cause, .. } => Some(cause.clone()),
            _ => None,
        })
        .expect("failure node present");
    assert_eq!(fail, "boom");
}

#[test]
fn v2_mir_echo_deterministic() {
    assert_eq!(lower_success("p", "!Data"), lower_success("p", "!Data"));
    assert_eq!(
        lower_failure("p", "?Data", "x"),
        lower_failure("p", "?Data", "x")
    );
    assert_ne!(
        lower_success("p", "!Data"),
        lower_failure("p", "!Data", "x")
    );
}

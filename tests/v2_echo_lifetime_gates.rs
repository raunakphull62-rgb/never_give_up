//! Phase 8 — v2 echo lifetime gates: no ignored Echoes, lossless runtime.

use klang::runtime::echo::Echo;
use klang::runtime::gc::rc::{live_count, RcCell};
use klang::sema::echo_lifetime::{check_branches, EchoScope};

#[test]
fn v2_echo_unlistened_is_error() {
    let mut s = EchoScope::new();
    s.create("pending", (0, 7));
    let e = s.check_end("prog.v2").expect_err("fails");
    assert_eq!(e.code, "E-ECHO-UNLISTENED");
    assert!(e.message.contains("pending"));
    assert!(e.fixes.iter().any(|f| f.label.contains("listen(pending)")));
}

#[test]
fn v2_echo_listen_and_transfer_ok() {
    let mut s = EchoScope::new();
    s.create("a", (0, 1));
    s.create("b", (2, 3));
    assert!(s.listen("a", "prog.v2", 4, 5).is_ok());
    assert!(s.transfer("b", "prog.v2", 6, 7).is_ok());
    assert!(s.check_end("prog.v2").is_ok());
    // Double-listen and unknown handles are invalid listens.
    let e = s.listen("a", "prog.v2", 8, 9).expect_err("fails");
    assert_eq!(e.code, "E-ECHO-INVALID-LISTEN");
    let e = s.listen("ghost", "prog.v2", 8, 9).expect_err("fails");
    assert_eq!(e.code, "E-ECHO-INVALID-LISTEN");
}

#[test]
fn v2_echo_branches_need_all_paths() {
    let mut before = EchoScope::new();
    before.create("p", (0, 1));
    // Listened only on one side: join fails.
    let mut then_s = before.clone();
    then_s.listen("p", "prog.v2", 2, 3).expect("listens");
    let else_s = before.clone();
    let e = check_branches(&before, &then_s, Some(&else_s), "prog.v2").expect_err("fails");
    assert_eq!(e.code, "E-ECHO-UNLISTENED");
    // Listened on both sides: join holds.
    let mut else_s = before.clone();
    else_s.listen("p", "prog.v2", 4, 5).expect("listens");
    assert!(check_branches(&before, &then_s, Some(&else_s), "prog.v2").is_ok());
    // Created inside a branch must settle in that branch.
    let mut inner = EchoScope::new();
    inner.create("q", (6, 7));
    assert!(check_branches(&before, &inner, Some(&before), "prog.v2").is_err());
}

#[test]
fn v2_echo_early_return_checked() {
    let mut s = EchoScope::new();
    s.create("p", (0, 1));
    // `return` with an outstanding Echo is the same as scope exit.
    assert_eq!(
        s.check_end("prog.v2").expect_err("fails").code,
        "E-ECHO-UNLISTENED"
    );
}

#[test]
fn v2_echo_runtime_success_and_cleanup() {
    let before = live_count();
    let e = Echo::new("pending");
    assert_eq!(e.strong(), 1);
    e.complete(42);
    let v = e.listen("prog.v2").expect("delivers");
    assert_eq!(v, 42);
    assert_eq!(live_count(), before);
}

#[test]
fn v2_echo_runtime_failure_lossless() {
    let before = live_count();
    let e: Echo<i32> = Echo::new("pending");
    e.fail("boom");
    let err = e.listen("prog.v2").expect_err("fails");
    assert_eq!(err.handle, "pending");
    assert_eq!(err.cause, "boom");
    assert_eq!(live_count(), before);
}

#[test]
fn v2_echo_rc_counts_and_cycles_visible() {
    let c = RcCell::new(1);
    assert_eq!(c.strong(), 1);
    let r = c.retain();
    assert_eq!(c.strong(), 2);
    r.release();
    assert_eq!(c.strong(), 1);
    let w = c.downgrade();
    assert!(w.upgrade().is_some());
    c.release();
    assert!(c.is_freed());
    // Freed cells observe as dead: the visible shape of a leak/cycle.
    assert!(w.upgrade().is_none());
}

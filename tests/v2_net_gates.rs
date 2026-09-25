//! Phase 10 — v2 network gates: `echo_get` behind a mock transport.

use klang::ai_safety::schema::DataValue;
use klang::stdlib::net::{echo_get, MockTransport, Transport};

#[test]
fn v2_net_echo_get_is_dissonant_echo() {
    let p = echo_get("https://example.com/profile");
    assert_eq!(p.url, "https://example.com/profile");
    // The boundary type is `Echo<?Data>`: dissonant until tuned.
    assert_eq!(p.inner_ty, "?Data");
    assert_eq!(p.strong(), 1);
}

#[test]
fn v2_net_mock_success_delivers() {
    let mut t = MockTransport::new();
    t.when("https://example.com/a", "hello");
    let echo = echo_get("https://example.com/a").fulfill(&t);
    let v = echo.listen("prog.v2").expect("delivers");
    assert_eq!(v, DataValue::Str("hello".to_string()));
}

#[test]
fn v2_net_mock_failure_lossless() {
    let mut t = MockTransport::new();
    t.when_fail("https://example.com/down", "dns error");
    let echo = echo_get("https://example.com/down").fulfill(&t);
    let e = echo.listen("prog.v2").expect_err("fails");
    assert_eq!(e.cause, "dns error");
    // Unregistered URLs fail without any I/O: pure in-memory lookup.
    let echo = echo_get("https://example.com/unknown").fulfill(&t);
    assert!(echo.listen("prog.v2").is_err());
    assert!(t.get("https://example.com/unknown").is_err());
}

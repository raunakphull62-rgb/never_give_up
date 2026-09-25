//! Phase 0 — v2 baseline boundary.
//!
//! Pins the v1 baseline before any v2 modules exist: existing parser/HIR
//! behavior is unchanged, diagnostics keep their JSON contract, and no
//! model/network path is introduced.

use klang::hir::TypedHIR;
use klang::parser::Parser;

#[test]
fn v2_baseline_v1_milestone_still_clean() {
    let src =
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(20, 22) }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("v1 baseline parses");
    assert!(TypedHIR::check(prog).is_ok(), "v1 baseline checks clean");
}

#[test]
fn v2_baseline_diagnostic_json_contract() {
    let mut p = Parser::new("fn main() -> i32 { return nope }");
    let prog = p.parse_program().expect("parses");
    let diags = TypedHIR::check(prog).expect_err("fails");
    let j = diags[0].to_json();
    for field in [
        "\"code\"",
        "\"severity\"",
        "\"message\"",
        "\"primary_span\"",
        "\"cause\"",
        "\"fixes\"",
        "\"rule\"",
    ] {
        assert!(j.contains(field), "diagnostic JSON keeps {field}");
    }
}

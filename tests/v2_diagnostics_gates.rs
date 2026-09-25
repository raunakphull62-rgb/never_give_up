//! Phase 11 — v2 diagnostics gates: every code, spans, fixes, JSON shape.

use klang::ai_safety::diagnostics as v2;
use klang::ai_safety::schema::{DataValue, FieldTy, Schema, SchemaField};
use klang::ast::resonance::QualifiedType;
use klang::sema::tuner;

fn codes() -> Vec<(String, String)> {
    let err = || klang::ai_safety::schema::VerifyError {
        schema: klang::ai_safety::schema::SchemaId("S@1:00".to_string()),
        path: "a".to_string(),
        expected: "int".to_string(),
        found: "str".to_string(),
        message: "m".to_string(),
    };
    vec![
        (
            v2::schema_invalid("f", 0, 1, &err()).code,
            "E-SCHEMA-INVALID".to_string(),
        ),
        (
            v2::schema_not_found("f", 0, 1, "S").code,
            "E-SCHEMA-NOT-FOUND".to_string(),
        ),
        (
            v2::resonance_unproven("f", 0, 1, "?Data", "!Data").code,
            "E-RESONANCE-UNPROVEN".to_string(),
        ),
        (
            v2::resonance_mismatch("f", 0, 1, "Data", "!Data").code,
            "E-RESONANCE-MISMATCH".to_string(),
        ),
        (
            v2::flow_mutable_capture("f", 0, 1, "t").code,
            "E-FLOW-MUTABLE-CAPTURE".to_string(),
        ),
        (
            v2::flow_duplicate_dependency("f", 0, 1, "t").code,
            "E-FLOW-DUPLICATE-DEPENDENCY".to_string(),
        ),
        (
            v2::echo_unlistened("f", 0, 1, "p").code,
            "E-ECHO-UNLISTENED".to_string(),
        ),
        (
            v2::echo_invalid_listen("f", 0, 1, "p", "why").code,
            "E-ECHO-INVALID-LISTEN".to_string(),
        ),
        (
            v2::echo_outlives_scope("f", 0, 1, "p").code,
            "E-ECHO-OUTLIVES-SCOPE".to_string(),
        ),
        (
            v2::echo_failure_lost("f", 0, 1, "p").code,
            "E-ECHO-FAILURE-LOST".to_string(),
        ),
        (
            v2::parse_flow("f", 0, 1, "m").code,
            "E-PARSE-FLOW".to_string(),
        ),
        (
            v2::parse_echo("f", 0, 1, "m").code,
            "E-PARSE-ECHO".to_string(),
        ),
    ]
}

#[test]
fn v2_diagnostics_all_codes_present() {
    let all = codes();
    assert_eq!(all.len(), 12);
    for (code, want) in &all {
        assert_eq!(code, want);
    }
}

#[test]
fn v2_diagnostics_shape_and_json() {
    // Every diagnostic carries span, cause, rule, and at least one fix.
    let err = klang::ai_safety::schema::VerifyError {
        schema: klang::ai_safety::schema::SchemaId("S@1:00".to_string()),
        path: "user.address.zip".to_string(),
        expected: "str".to_string(),
        found: "int".to_string(),
        message: "wrong type".to_string(),
    };
    let d = v2::schema_invalid("prog.v2", 3, 9, &err);
    assert_eq!((d.primary_span.start, d.primary_span.end), (3, 9));
    assert!(!d.cause.is_empty() && !d.rule.is_empty() && !d.fixes.is_empty());
    let j = d.to_json();
    for field in [
        "\"code\"",
        "\"severity\"",
        "\"message\"",
        "\"primary_span\"",
        "\"cause\"",
        "\"expected\"",
        "\"found\"",
        "\"fixes\"",
        "\"rule\"",
        "\"related\"",
    ] {
        assert!(j.contains(field), "json keeps {field}");
    }
    // Deterministic: same diagnostic renders identically twice.
    assert_eq!(d.to_json(), d.to_json());
}

#[test]
fn v2_diagnostics_mismatch_sides() {
    let e = tuner::check_call(
        &QualifiedType::parse("?Data"),
        &QualifiedType::parse("!Data"),
        "prog.v2",
        0,
        5,
    )
    .expect_err("fails");
    assert_eq!(e.expected.as_deref(), Some("!Data"));
    assert_eq!(e.found.as_deref(), Some("?Data"));
    assert!(e.to_json().contains("\"expected\": \"!Data\""));
}

#[test]
fn v2_diagnostics_move_out_and_lossless() {
    // Outlives-scope fires when a Created handle would escape untracked.
    let mut s = klang::sema::echo_lifetime::EchoScope::new();
    s.create("p", (0, 1));
    let e = s.move_out("p", "prog.v2", 2, 3).expect_err("fails");
    assert_eq!(e.code, "E-ECHO-OUTLIVES-SCOPE");
    assert!(!e.fixes.is_empty());
    s.listen("p", "prog.v2", 2, 3).expect("listens");
    assert!(s.move_out("p", "prog.v2", 2, 3).is_ok());
    // Lossless verifier accepts lowered paths, rejects dropped failures.
    use klang::mir::echo_lowering::{verify_lossless, EchoMirNode, EchoMirOp, EchoState};
    let good = klang::mir::echo_lowering::lower_failure("p", "?Data", "boom");
    assert!(verify_lossless(&good, "prog.v2").is_ok());
    let bad = vec![
        EchoMirNode {
            state: EchoState::Created,
            op: EchoMirOp::Create {
                handle: "p".to_string(),
                inner: "?Data".to_string(),
            },
        },
        EchoMirNode {
            state: EchoState::Failed,
            op: EchoMirOp::Fail {
                handle: "p".to_string(),
                cause: "boom".to_string(),
            },
        },
    ];
    let e = verify_lossless(&bad, "prog.v2").expect_err("fails");
    assert_eq!(e.code, "E-ECHO-FAILURE-LOST");
}

#[test]
fn v2_diagnostics_v1_compat() {
    // v1 diagnostics carry no mismatch sides (null in JSON, None in Rust).
    let mut p = klang::parser::Parser::new("fn main() -> i32 { return nope }");
    let prog = p.parse_program().expect("parses");
    let diags = klang::hir::TypedHIR::check(prog).expect_err("fails");
    assert_eq!(diags[0].expected, None);
    assert!(diags[0].to_json().contains("\"expected\": null"));
    let _s = Schema::new("S", "1", vec![SchemaField::new("x", FieldTy::Int)]);
    let _v = DataValue::Int(1);
}

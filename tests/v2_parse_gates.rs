//! Phase 4 — v2 parser gates: valid Flow/Echo parse, invalid is coded.

use klang::parser::{echo, flow};

#[test]
fn v2_parse_valid_flow() {
    let d = flow::parse_flow_decl(
        "flow(score: i32, dep=threshold: i32) -> str { if score >= threshold { return 1 } }",
        Some("classify"),
    )
    .expect("valid flow parses");
    assert_eq!(d.name.as_deref(), Some("classify"));
    assert_eq!(d.expr.params.len(), 1);
    assert_eq!(d.expr.params[0].name, "score");
    assert_eq!(d.expr.deps.len(), 1);
    assert_eq!(d.expr.deps[0].name, "threshold");
    assert_eq!(d.expr.return_ty.as_deref(), Some("str"));
    // Parent-child containment: expr id extends decl id.
    assert!(d.expr.id.starts_with(&d.id));
    assert_ne!(d.expr.id, d.id);
}

#[test]
fn v2_parse_flow_without_return() {
    let d = flow::parse_flow_decl("flow(x: i32) { return x }", None).expect("parses");
    assert_eq!(d.expr.return_ty, None);
    assert!(d.expr.body.contains("return x"));
}

#[test]
fn v2_parse_flow_errors_coded() {
    let e = flow::parse_flow_decl("flow(score i32) { return 1 }", None).expect_err("fails");
    assert_eq!(e.code, "E-PARSE-FLOW");
    assert!(!e.fixes.is_empty());
    let e = flow::parse_flow_decl("flow(x: i32, dep=a: i32, dep=a: i32) { return 1 }", None)
        .expect_err("duplicate dep fails");
    assert_eq!(e.code, "E-PARSE-FLOW");
    assert!(e.message.contains("duplicate dependency"));
    let e = flow::parse_flow_decl("flow(x: i32) { return 1", None).expect_err("unclosed fails");
    assert_eq!(e.code, "E-PARSE-FLOW");
}

#[test]
fn v2_parse_valid_echo_and_listen() {
    let d = echo::parse_echo_decl("echo fn fetch(id: i64) -> !Data { return id }")
        .expect("valid echo parses");
    assert_eq!(d.name, "fetch");
    assert_eq!(d.params, vec![("id".to_string(), "i64".to_string())]);
    assert_eq!(d.return_ty, "!Data");
    assert_eq!(
        echo::initial_ownership(),
        klang::ast::echo::EchoOwnership::Created
    );
    let l = echo::parse_listen("listen(pending)").expect("listen parses");
    assert_eq!(l.handle, "pending");
}

#[test]
fn v2_parse_echo_errors_coded() {
    let e = echo::parse_echo_decl("echo fn fetch(").expect_err("truncated fails");
    assert_eq!(e.code, "E-PARSE-ECHO");
    assert!(!e.fixes.is_empty());
    let e = echo::parse_listen("listen()").expect_err("empty listen fails");
    assert_eq!(e.code, "E-PARSE-ECHO");
    // Spans stay inside the source.
    assert!(e.primary_span.end <= "listen()".len());
}

#[test]
fn v2_parse_v1_regression() {
    let mut p = klang::parser::Parser::new("fn main() -> i32 { return 42 }");
    let prog = p.parse_program().expect("v1 parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}

//! Phase 7 — v2 flow gates: explicit deps, capture errors, lowering.

use klang::ast::flow::{FlowDependency, FlowExpr, FlowParam};
use klang::ast::NodeId;
use klang::mir::flow_lowering::{lower_flow, FlowStep};
use klang::parser::flow::parse_flow_decl;
use klang::sema::flow_capture::{check_flow, FlowCheckEnv};

fn expr(params: Vec<(&str, &str)>, deps: Vec<(&str, &str)>, body: &str) -> FlowExpr {
    FlowExpr {
        id: NodeId::new(vec![0, 0]),
        params: params
            .into_iter()
            .map(|(n, t)| FlowParam {
                name: n.to_string(),
                ty: t.to_string(),
            })
            .collect(),
        deps: deps
            .into_iter()
            .map(|(n, t)| FlowDependency {
                name: n.to_string(),
                ty: t.to_string(),
            })
            .collect(),
        return_ty: Some("str".to_string()),
        body: body.to_string(),
    }
}

#[test]
fn v2_flow_explicit_dep_compiles() {
    let f = expr(
        vec![("score", "i32")],
        vec![("threshold", "i32")],
        "score >= threshold",
    );
    let env = FlowCheckEnv::new(&["threshold"], &[]);
    assert!(check_flow(&f, &env, "prog.v2").is_ok());
}

#[test]
fn v2_flow_mutable_capture_fails() {
    let f = expr(vec![("score", "i32")], vec![], "score >= threshold");
    let env = FlowCheckEnv::new(&["threshold"], &[]);
    let e = check_flow(&f, &env, "prog.v2").expect_err("fails");
    assert_eq!(e.code, "E-FLOW-MUTABLE-CAPTURE");
    assert!(e.message.contains("threshold"));
    assert!(e.fixes.iter().any(|x| x.label.contains("dep=threshold")));
    // Span points inside the body.
    assert!(e.primary_span.end <= "score >= threshold".len());
}

#[test]
fn v2_flow_immutable_capture_allowed() {
    // Documented rule: immutable constants need no `dep=`.
    let f = expr(vec![("score", "i32")], vec![], "score >= LIMIT");
    let env = FlowCheckEnv::new(&[], &["LIMIT"]);
    assert!(check_flow(&f, &env, "prog.v2").is_ok());
}

#[test]
fn v2_flow_duplicate_dep_rejected() {
    let f = expr(vec![("x", "i32")], vec![("a", "i32"), ("a", "i32")], "x");
    let env = FlowCheckEnv::new(&[], &[]);
    let e = check_flow(&f, &env, "prog.v2").expect_err("fails");
    assert_eq!(e.code, "E-FLOW-DUPLICATE-DEPENDENCY");
}

#[test]
fn v2_flow_lowering_deterministic() {
    let d = parse_flow_decl(
        "flow(score: i32, dep=threshold: i32) -> str { score >= threshold }",
        Some("classify"),
    )
    .expect("parses");
    let a = lower_flow(&d.expr, d.name.as_deref());
    let b = lower_flow(&d.expr, d.name.as_deref());
    assert_eq!(a, b);
    assert_eq!(a.env, vec!["score".to_string(), "threshold".to_string()]);
    assert!(matches!(a.steps[0], FlowStep::BindParam { .. }));
    assert!(matches!(a.steps[1], FlowStep::BindDep { .. }));
    assert!(matches!(a.steps[2], FlowStep::RunBody { .. }));
    assert!(matches!(a.steps[3], FlowStep::Return { .. }));
    // Args and return type survive lowering.
    match &a.steps[3] {
        FlowStep::Return { ty } => assert_eq!(ty.as_deref(), Some("str")),
        _ => panic!("last step returns"),
    }
}

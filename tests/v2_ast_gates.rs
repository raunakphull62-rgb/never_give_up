//! Phase 3 — v2 AST gates: qualifiers stay distinct, Flow deps explicit.

use klang::ast::echo::{EchoDecl, EchoHandleType, EchoOwnership, ListenExpr};
use klang::ast::flow::{FlowDecl, FlowDependency, FlowExpr, FlowParam};
use klang::ast::resonance::{QualifiedType, ResonanceQualifier};
use klang::ast::NodeId;

fn nid(n: u32) -> NodeId {
    NodeId::new(vec![n])
}

#[test]
fn v2_ast_qualifiers_distinguishable() {
    let q = QualifiedType::parse("?Data");
    let c = QualifiedType::parse("Data");
    let h = QualifiedType::parse("!Data");
    assert_eq!(q.qualifier, ResonanceQualifier::Dissonant);
    assert_eq!(c.qualifier, ResonanceQualifier::Consonant);
    assert_eq!(h.qualifier, ResonanceQualifier::Harmonic);
    assert_ne!(q, c);
    assert_ne!(c, h);
    assert_ne!(q, h);
    // Qualifier is data, not string-sniffing: bases match, markers differ.
    assert_eq!(q.base, "Data");
    assert_eq!(h.base, "Data");
    // Round-trip is stable.
    assert_eq!(q.display(), "?Data");
    assert_eq!(c.display(), "Data");
    assert_eq!(h.display(), "!Data");
    assert!(q.is_dissonant());
    assert!(h.is_harmonic());
    assert!(!c.is_dissonant() && !c.is_harmonic());
}

#[test]
fn v2_ast_flow_deps_explicit() {
    let expr = FlowExpr {
        id: nid(0),
        params: vec![FlowParam {
            name: "score".to_string(),
            ty: "i32".to_string(),
        }],
        deps: vec![FlowDependency {
            name: "threshold".to_string(),
            ty: "i32".to_string(),
        }],
        return_ty: Some("str".to_string()),
        body: "if score >= dep { return \"high\" }".to_string(),
    };
    assert_eq!(expr.dep_names(), vec!["threshold"]);
    assert!(expr.has_dep("threshold"));
    assert!(!expr.has_dep("other"));
    let decl = FlowDecl {
        id: nid(1),
        name: Some("classify".to_string()),
        expr: expr.clone(),
    };
    // Debug snapshot stays stable and names the dependency.
    let snap = format!("{decl:?}");
    assert!(snap.contains("classify"));
    assert!(snap.contains("threshold"));
    assert!(snap.contains("score"));
}

#[test]
fn v2_ast_echo_nodes() {
    let decl = EchoDecl {
        id: nid(2),
        name: "fetch".to_string(),
        params: vec![("id".to_string(), "i64".to_string())],
        return_ty: "!Data".to_string(),
    };
    let handle = EchoHandleType {
        inner: "!Data".to_string(),
    };
    assert_eq!(handle.display(), "Echo<!Data>");
    let listen = ListenExpr {
        id: nid(3),
        handle: "pending".to_string(),
    };
    assert_eq!(listen.handle, "pending");
    // Ownership markers are four distinct states.
    assert_ne!(EchoOwnership::Created, EchoOwnership::Listened);
    assert_ne!(EchoOwnership::Transferred, EchoOwnership::Joined);
    let snap = format!("{decl:?} {handle:?} {listen:?}");
    assert!(snap.contains("fetch"));
    assert!(snap.contains("pending"));
}

#[test]
fn v2_ast_v1_untouched() {
    let mut p = klang::parser::Parser::new("fn main() -> i32 { return 1 }");
    let prog = p.parse_program().expect("v1 parses");
    assert_eq!(prog.functions.len(), 1);
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}

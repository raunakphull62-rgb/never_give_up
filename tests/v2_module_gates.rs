//! Phase 1 — v2 module scaffold gates.
//!
//! Every required v2 top-level module from PRD §5.1 must import. Converted
//! flat modules (`ast`, `parser`, `mir`, `runtime`, `package`) keep their v1
//! paths; brand-new boundaries (`lexer`, `sema`, `ai_safety`, `stdlib`,
//! `runtime::gc`) expose only minimal placeholder types until later phases.

#[test]
fn v2_modules_import() {
    // v1 paths still resolve after the flat-file → directory moves.
    let _ = std::any::type_name::<klang::parser::Parser>();
    let _ = std::any::type_name::<klang::ast::NodeId>();
    let _ = std::any::type_name::<klang::mir::MirModule>();
    let _ = std::any::type_name::<klang::runtime::Value>();
    let _ = std::any::type_name::<klang::package::Manifest>();
    // New v2 boundaries expose their scaffold types.
    let ok_lex: klang::lexer::Result<()> = Ok(());
    let ok_sema: klang::sema::Result<()> = Ok(());
    let _ = (ok_lex, ok_sema);
    let _ = klang::ai_safety::SchemaId("s".to_string());
    let _ = klang::stdlib::StdlibError {
        message: "scaffold".to_string(),
    };
    let _ = klang::runtime::gc::CyclePolicy::Report;
}

#[test]
fn v2_scaffold_keeps_v1_behavior() {
    let src = "fn main() -> i32 { return 40 + 2 }";
    let mut p = klang::parser::Parser::new(src);
    let prog = p.parse_program().expect("v1 parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &std::collections::HashMap::new())
            .expect("v1 runs");
    assert_eq!(v, 42);
}

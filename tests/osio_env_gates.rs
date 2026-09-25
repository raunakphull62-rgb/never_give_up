//! STDLIB-OSIO Phase 4 — Env gates.
//!
//! `env(name)` is the flat spelling of PRD `env::get(name)` (same
//! `::`-vs-flat rationale as Phases 1–3). Klang has no `Option` type,
//! so an unset variable is `""`, never an error — pinned here, with
//! the set-path and builtin checking alongside.

use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new())
}

#[test]
fn osio_env_set_var_returns_value() {
    std::env::set_var("KLANG_OSIO_ENV_PROBE", "hello-env");
    let src = "fn main() -> i32 { print(env(\"KLANG_OSIO_ENV_PROBE\")) return 42 }";
    let (v, out) = run_src(src, "main").expect("runs");
    assert_eq!(v, 42);
    assert_eq!(out, vec!["hello-env".to_string()]);
    std::env::remove_var("KLANG_OSIO_ENV_PROBE");
}

#[test]
fn osio_env_unset_var_returns_empty_not_error() {
    // Klang has no Option type: unset is "", never a diagnostic.
    std::env::remove_var("KLANG_OSIO_ENV_MISSING_XYZ_12345");
    let src = "fn main() -> i32 { let v = env(\"KLANG_OSIO_ENV_MISSING_XYZ_12345\") print(v) if v == \"\" { return 42 } return 0 }";
    let (v, out) = run_src(src, "main").expect("unset must still be Ok");
    assert_eq!(v, 42);
    assert_eq!(out, vec!["".to_string()]);
}

#[test]
fn osio_env_arg_checked() {
    let mut p = Parser::new("fn main() -> i32 { return env(42) }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { return env(\"a\", \"b\") }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
}

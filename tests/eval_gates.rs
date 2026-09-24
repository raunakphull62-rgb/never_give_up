//! Phase 2 Session 4 — multi-feature dogfood gates.
//!
//! `examples/eval.klang` is a small expression evaluator that genuinely
//! needs generics (`Res<T>`, `expect<T>`), enums (`Expr`, `Res`), modules
//! (`lexer` / `parser` / `eval` with `pub`/private boundaries), and
//! effects (`throws` from `lookup` through `run` to `main`) together.
//! These tests pin its contract: clean compile, exact runtime outputs,
//! a load-bearing effect chain, and the F11 eager-`&&` limitation found
//! while building it.

use std::collections::HashMap;

use klang::parser::Parser;

const EVAL_SRC: &str = include_str!("../examples/eval.klang");

fn check_prog(src: &str) -> Result<klang::hir::TypedHIR, Vec<klang::diagnostics::Diagnostic>> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    klang::hir::TypedHIR::check(prog)
}

#[test]
fn eval_dogfood_checks_clean() {
    assert!(
        check_prog(EVAL_SRC).is_ok(),
        "dogfood must compile clean: {:?}",
        check_prog(EVAL_SRC).err().map(|ds| ds
            .iter()
            .map(|d| d.to_json())
            .collect::<Vec<_>>())
    );
}

#[test]
fn eval_dogfood_runs_to_expected_outputs() {
    // Precedence, parens, variables, left-assoc division, then every
    // error code the evaluator defines (2000s = eval, 1000s = parse),
    // then the generic-`expect` default path.
    let mut p = Parser::new(EVAL_SRC);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 0);
    assert_eq!(
        out,
        vec![
            "14", "20", "31", "4", "2004", "2005", "1001", "1002", "1003", "42",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>(),
        "exact evaluator outputs"
    );
}

#[test]
fn eval_dogfood_effect_chain_is_load_bearing() {
    // The `throws` chain (lookup <- run <- main) is structural, not
    // decorative: dropping `throws` from `main` must fail with
    // E-EFFECT-MISMATCH, not silently pass.
    let stripped = EVAL_SRC.replace("fn main() -> i32 throws {", "fn main() -> i32 {");
    assert_ne!(stripped, EVAL_SRC, "the dogfood must actually declare main throws");
    let mut p = Parser::new(&stripped);
    let prog = p.parse_program().expect("still parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail without throws");
    assert!(
        err.iter().any(|d| d.code == "E-EFFECT-MISMATCH"),
        "want E-EFFECT-MISMATCH, got {:?}",
        err.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn eval_dogfood_survives_fmt_roundtrip() {
    // Same discipline as toolchain_gates: formatted output re-parses,
    // re-checks, and formatting is a fixpoint.
    let once = klang::fmt::fmt_program(&Parser::new(EVAL_SRC).parse_program().expect("parses"));
    let mut q = Parser::new(&once);
    let prog2 = q.parse_program().expect("fmt output re-parses");
    assert!(klang::hir::TypedHIR::check(prog2.clone()).is_ok());
    let twice = klang::fmt::fmt_program(&prog2);
    assert_eq!(once, twice, "fmt must be a fixpoint on the dogfood");
}

// ---------------------------------------------------------------------
// F11 (triaged): `&&` / `||` evaluate both sides eagerly — no
// short-circuit. A bounds guard in `&&` position therefore fails at
// runtime instead of skipping the RHS. This pins the LOUD failure mode
// (E-RUNTIME, never a silent wrong answer) and the nested-`if`
// workaround the dogfood itself uses.
// ---------------------------------------------------------------------

#[test]
fn eager_and_guard_fails_loudly_never_silently() {
    // `i < n && s[i] == ...` with `i == n` is well-typed (checker sees
    // Bool && Bool) but evaluates `s[n]` anyway: E-RUNTIME, exit via Err.
    let src = "fn main() -> i32 { let s = \"ab\" let i = 2 if i < 2 && s[i] == \"x\" { return 1 } return 0 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok(), "checker blesses the guard shape");
    let mir = klang::mir::lower(&prog);
    let err = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new())
        .expect_err("eager RHS must fail loudly");
    assert_eq!(err.code, "E-RUNTIME", "{}", err.to_json());
}

#[test]
fn nested_if_guard_runs_correctly() {
    // The F11 workaround: nested `if`s evaluate the guard first, so the
    // same program shape runs to the right answer with no diagnostics.
    let src = "fn main() -> i32 { let s = \"ab\" let i = 2 if i < 2 { if s[i] == \"x\" { return 1 } } return 0 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 0);
}

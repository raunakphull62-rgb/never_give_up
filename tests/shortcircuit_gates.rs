//! Phase 1a — `&&` / `||` short-circuit gates.
//!
//! Before Phase 1 both sides always evaluated (eager): `false && boom()`
//! still ran `boom()`. Now the right-hand side lowers inside the taken
//! branch only, so its effects (prints, runtime errors) never happen when
//! the left side already decides the result. Values stay `Int` 0/1, so
//! every program that evaluated both sides anyway sees identical results.

use std::collections::HashMap;

use klang::parser::Parser;

fn parse(src: &str) -> klang::ast::Program {
    let mut p = Parser::new(src);
    p.parse_program().expect("parses")
}

fn check(src: &str) -> klang::ast::Program {
    let prog = parse(src);
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    prog
}

/// Interpreter only (used where the JIT cannot follow: strings, or
/// divisions the JIT documents as traps instead of `E-RUNTIME`).
fn run_interp(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let prog = check(src);
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new())
}

/// Both backends; assert identical value + output (int-only programs).
fn diff(src: &str, entry: &str) -> (i32, Vec<String>) {
    let prog = check(src);
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("interp runs");
    let (jv, jout) = klang::jit::run_jit(&mir, entry, &[]).expect("jit runs");
    assert_eq!(jv as i32, v, "jit value == interp value");
    assert_eq!(jout, out, "jit output == interp output");
    (v, out)
}

const RHS: &str = "fn rhs() -> i32 { print(99) return 1 }";

#[test]
fn and_skips_rhs_when_left_false() {
    // The RHS print never happens on either backend; the value is still 0.
    let (v, out) = diff(&format!("{RHS} fn main() -> i32 {{ let x = false && rhs() print(x) return 0 }}"), "main");
    assert_eq!((v, out), (0, vec!["0".to_string()]));
    // Int zero behaves the same (conditions accept `Int`).
    let (v, out) = diff(&format!("{RHS} fn main() -> i32 {{ let x = 0 && rhs() print(x) return 0 }}"), "main");
    assert_eq!((v, out), (0, vec!["0".to_string()]));
    println!("and-skip OK");
}

#[test]
fn or_skips_rhs_when_left_true() {
    let (v, out) = diff(&format!("{RHS} fn main() -> i32 {{ let x = true || rhs() print(x) return 0 }}"), "main");
    assert_eq!((v, out), (0, vec!["1".to_string()]));
    let (v, out) = diff(&format!("{RHS} fn main() -> i32 {{ let x = 5 || rhs() print(x) return 0 }}"), "main");
    assert_eq!((v, out), (0, vec!["1".to_string()]));
    println!("or-skip OK");
}

#[test]
fn and_evaluates_rhs_when_left_true() {
    // Taken path: the RHS print happens exactly once on both backends.
    let (v, out) = diff(&format!("{RHS} fn main() -> i32 {{ let x = true && rhs() print(x) return 0 }}"), "main");
    assert_eq!((v, out), (0, vec!["99".to_string(), "1".to_string()]));
    println!("and-taken OK");
}

#[test]
fn or_evaluates_rhs_when_left_false() {
    let (v, out) = diff(&format!("{RHS} fn main() -> i32 {{ let x = false || rhs() print(x) return 0 }}"), "main");
    assert_eq!((v, out), (0, vec!["99".to_string(), "1".to_string()]));
    println!("or-taken OK");
}

#[test]
fn short_circuit_skips_runtime_error() {
    // `1 / 0` is `E-RUNTIME` when evaluated — so reaching `print` proves
    // the RHS never ran. (Interpreter only: the JIT documents div-by-zero
    // as a trap instead of `E-RUNTIME`, so it cannot run this program.)
    let (v, out) = run_interp(
        "fn boom() -> i32 { return 1 / 0 } fn main() -> i32 { let x = false && boom() print(7) return 0 }",
        "main",
    )
    .expect("skipped RHS never divides");
    assert_eq!((v, out), (0, vec!["7".to_string()]));
    let (v, out) = run_interp(
        "fn boom() -> i32 { return 1 / 0 } fn main() -> i32 { let x = true || boom() print(8) return 0 }",
        "main",
    )
    .expect("skipped RHS never divides");
    assert_eq!((v, out), (0, vec!["8".to_string()]));
    // Taken paths still fail loudly — skipping did not swallow the error.
    let err = run_interp(
        "fn boom() -> i32 { return 1 / 0 } fn main() -> i32 { let x = true && boom() return 0 }",
        "main",
    )
    .expect_err("taken RHS still divides");
    assert_eq!(err.code, "E-RUNTIME", "{}", err.to_json());
    let err = run_interp(
        "fn boom() -> i32 { return 1 / 0 } fn main() -> i32 { let x = false || boom() return 0 }",
        "main",
    )
    .expect_err("taken RHS still divides");
    assert_eq!(err.code, "E-RUNTIME", "{}", err.to_json());
    println!("skip-error OK");
}

#[test]
fn logic_results_unchanged() {
    // Truth table, int and bool operands, both backends.
    let cases = [
        ("1 && 2", 1),
        ("0 && 9", 0),
        ("9 && 0", 0),
        ("0 && 0", 0),
        ("5 || 0", 1),
        ("0 || 5", 1),
        ("0 || 0", 0),
        ("true && true", 1),
        ("true && false", 0),
        ("false && true", 0),
        ("false || false", 0),
        ("true || false", 1),
        ("(1 < 2) && (3 < 4)", 1),
        ("(1 < 2) && (3 > 4)", 0),
        ("!(0 || 0)", 1),
        ("(false && true) || (true && true)", 1),
        ("(true || false) && (false || 0)", 0),
    ];
    for (expr, want) in cases {
        // `&&` / `||` yield `Bool`, so the probe mains return `bool`
        // (`main` may return any type; the runner maps it to 0/1).
        let src = format!("fn main() -> bool {{ return {expr} }}");
        let (v, _) = diff(&src, "main");
        assert_eq!(v, want, "value of `{expr}` unchanged");
    }
    // Conditions built on `&&` / `||` still branch the same way.
    let (v, _) = diff(
        "fn main() -> i32 { if (1 < 2) && (2 < 3) { return 42 } return 0 }",
        "main",
    );
    assert_eq!(v, 42);
    let (v, _) = diff(
        "fn main() -> i32 { let i = 0 while i < 3 || false { i = i + 1 } return i }",
        "main",
    );
    assert_eq!(v, 3);
    println!("truth-table OK");
}

#[test]
fn non_bool_truthiness_unchanged() {
    // Statically-known strings cannot `&&`/`||` (the checker wants
    // Bool/Int), but dynamically-typed values (`Unknown`, e.g. map
    // lookups holding strings) coerce by emptiness at runtime, exactly as
    // before (interpreter only: string constants are outside the int-only
    // JIT).
    let (v, _) = run_interp(
        "fn main() -> bool { let m = {\"h\": \"hi\", \"n\": 5} return m[\"h\"] && m[\"n\"] }",
        "main",
    )
    .expect("runs");
    assert_eq!(v, 1);
    let (v, _) = run_interp(
        "fn main() -> bool { let m = {\"e\": \"\", \"n\": 5} return m[\"e\"] && m[\"n\"] }",
        "main",
    )
    .expect("runs");
    assert_eq!(v, 0);
    let (v, _) = run_interp(
        "fn main() -> bool { let m = {\"e\": \"\"} return m[\"e\"] || 7 }",
        "main",
    )
    .expect("runs");
    assert_eq!(v, 1);
    // Skipping applies to non-bool left sides too.
    let (v, out) = run_interp(
        "fn rhs() -> i32 { print(99) return 1 } fn main() -> i32 { let m = {\"e\": \"\"} let x = m[\"e\"] && rhs() print(x) return 0 }",
        "main",
    )
    .expect("runs");
    assert_eq!((v, out), (0, vec!["0".to_string()]));
    println!("truthiness OK");
}

#[test]
fn v2_and_or_short_circuit() {
    // The v2 tree-walking runtime short-circuits too: a skipped `1 / 0`
    // never fails, a taken one still does.
    let run_v2 = |src: &str| {
        let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
        klang::runtime::v2::run_v2_program(&prog, "main")
    };
    let (v, _) = run_v2("fn boom() -> i32 { return 1 / 0 } fn main() -> i32 { let x = false && boom() return x }")
        .expect("v2 skips and-RHS");
    assert_eq!(v, 0);
    let (v, _) = run_v2("fn boom() -> i32 { return 1 / 0 } fn main() -> i32 { let x = true || boom() return x }")
        .expect("v2 skips or-RHS");
    assert_eq!(v, 1);
    let err = run_v2("fn boom() -> i32 { return 1 / 0 } fn main() -> i32 { let x = true && boom() return x }")
        .expect_err("v2 taken and-RHS still fails");
    assert_eq!(err.code, "E-RUNTIME", "{}", err.to_json());
    let (v, _) = run_v2("fn main() -> i32 { return 1 < 2 && 3 < 4 }").expect("v2 truth table runs");
    assert_eq!(v, 1);
    println!("v2 short-circuit OK");
}

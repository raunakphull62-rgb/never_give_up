//! BUG-A regression: `if/else` as the tail expression of a function.
//!
//! Root cause was the v1 interpreter's fall-off-the-end `last` value not
//! being updated by value-defining ops (`Const`, `ConstFloat`, `ConstStr`,
//! `ArrayNew`, `MapNew`, `StructNew`, `Len`), so a tail `if` whose branches
//! were bare constants returned the stale condition value instead. The v2
//! interpreter had the mirror bug: `If` as tail never propagated the taken
//! branch's `__last` to the outer block. The JIT already updated its
//! `last_var` on every defining op and was correct; these tests pin all
//! three paths together.

use std::collections::HashMap;

use klang::parser::Parser;

fn run_v1(src: &str, entry: &str) -> (i32, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("runs")
}

fn run_v1_jit_parity(src: &str, entry: &str) -> (i32, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("interp runs");
    let (jv, jout) = klang::jit::run_jit(&mir, entry, &[]).expect("jit runs");
    assert_eq!(jv as i32, v, "jit value == interp value");
    assert_eq!(jout, out, "jit output == interp output");
    (v, out)
}

fn run_v2(src: &str) -> (i32, Vec<String>) {
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    klang::runtime::v2::run_v2_program(&prog, "main").expect("runs")
}

#[test]
fn tail_if_recursion_counts_correctly() {
    // Exact BUG-A repro: f(3) must be 3, not 4 (base 0 + three increments).
    let src = "fn f(n: i32) -> i32 { if n == 0 { 0 } else { 1 + f(n - 1) } } fn main() -> i32 { return f(3) }";
    let (v, _) = run_v1(src, "main");
    assert_eq!(v, 3, "tail if/else recursion: f(3) == 3");
}

#[test]
fn tail_if_recursion_depth_10() {
    // Companion from the report: depth(10) == 10, not 11.
    let src = "fn depth(n: i32) -> i32 { if n == 0 { 0 } else { 1 + depth(n - 1) } } fn main() -> i32 { return depth(10) }";
    let (v, _) = run_v1(src, "main");
    assert_eq!(v, 10);
}

#[test]
fn tail_if_without_recursion() {
    // Proves the fix is not a recursion special-case: pure tail constants.
    let src = "fn g(n: i32) -> i32 { if n == 0 { 5 } else { 7 } } fn main() -> i32 { return g(0) * 100 + g(1) }";
    let (v, _) = run_v1(src, "main");
    assert_eq!(v, 507, "5*100+7");
}

#[test]
fn tail_if_len_branch() {
    // `Len` / array tail branches also skipped `last` before the fix.
    let src = "fn pick(n: i32) -> i32 { if n == 0 { len([1, 2, 3]) } else { len([1, 2]) } } fn main() -> i32 { return pick(0) * 10 + pick(1) }";
    let (v, _) = run_v1(src, "main");
    assert_eq!(v, 32, "3*10+2");
}

#[test]
fn tail_if_explicit_return_still_correct() {
    let src = "fn f(n: i32) -> i32 { if n == 0 { return 0 } else { return 1 + f(n - 1) } } fn main() -> i32 { return f(3) }";
    let (v, _) = run_v1(src, "main");
    assert_eq!(v, 3);
}

#[test]
fn tail_if_early_return_fallthrough_still_correct() {
    let src = "fn f(n: i32) -> i32 { if n == 0 { return 0 } return 1 + f(n - 1) } fn main() -> i32 { return f(3) }";
    let (v, _) = run_v1(src, "main");
    assert_eq!(v, 3);
}

#[test]
fn tail_if_jit_parity_recursion() {
    let src = "fn f(n: i32) -> i32 { if n == 0 { 0 } else { 1 + f(n - 1) } } fn main() -> i32 { return f(3) }";
    let (v, _) = run_v1_jit_parity(src, "main");
    assert_eq!(v, 3);
}

#[test]
fn tail_if_jit_parity_simple() {
    let src = "fn g(n: i32) -> i32 { if n == 0 { 5 } else { 7 } } fn main() -> i32 { return g(0) * 100 + g(1) }";
    let (v, _) = run_v1_jit_parity(src, "main");
    assert_eq!(v, 507);
}

#[test]
fn tail_if_v2_simple() {
    let src = "fn g(n: i32) -> i32 {\n  if n == 0 {\n    5\n  } else {\n    7\n  }\n}\nfn main() -> i32 {\n  return g(0) * 100 + g(1)\n}\n";
    let (v, _) = run_v2(src);
    assert_eq!(v, 507);
}

#[test]
fn tail_if_v2_recursion() {
    let src = "fn f(n: i32) -> i32 {\n  if n == 0 {\n    0\n  } else {\n    1 + f(n - 1)\n  }\n}\nfn main() -> i32 {\n  return f(3)\n}\n";
    let (v, _) = run_v2(src);
    assert_eq!(v, 3);
}

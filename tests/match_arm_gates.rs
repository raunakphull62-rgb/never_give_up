//! KLANG-FOUNDATION-2 Step 1 — multi-statement match arms.
//!
//! `Pattern => { stmts...; trailing }`: setup statements run in order
//! (same semantics as an `if` block body) and the trailing expression
//! yields the arm's value. Single-expression arms are unchanged.
//!
//! Backend coverage note: the v2 language has no `enum`/`match` syntax at
//! all, so "v2 backend" coverage here is the documented exclusion (v2
//! rejects `match`, mirroring `try_catch_gates::v2_has_no_try_catch`) plus
//! a positive v2 run proving the shared pipeline is unaffected. The JIT
//! is int-only and enums lower through struct ops, so every enum `match`
//! is loudly rejected there (never silent wrong code).

use std::collections::HashMap;

use klang::parser::Parser;

fn parse(src: &str) -> klang::ast::Program {
    Parser::new(src).parse_program().expect("parses")
}

fn run_src(src: &str, entry: &str) -> (i32, Vec<String>) {
    let prog = parse(src);
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("runs")
}

fn check_err_code(src: &str, want: &str) {
    let prog = parse(src);
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(
        codes.contains(&want),
        "want {want} in {codes:?}: {src}"
    );
}

fn parse_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let err = p.parse_program().expect_err("must fail parse");
    assert_eq!(err.code, want, "{}", err.to_json());
}

#[test]
fn block_arm_two_stmts_plus_trailing() {
    // Basic case: setup statements run, trailing expression is the value.
    let (v, out) = run_src(
        "enum Opt { Some(x: i32), None } fn pick(v: Opt) -> i32 { return match v { Opt::Some(n) => { let y = n + 1 print(y) y * 2 }, Opt::None => 7 } } fn main() -> i32 { print(pick(Opt::Some(20))) print(pick(Opt::None())) return pick(Opt::Some(20)) + pick(Opt::None()) }",
        "main",
    );
    assert_eq!(out, vec!["21".to_string(), "42".to_string(), "7".to_string(), "21".to_string()]);
    assert_eq!(v, 49);
}

#[test]
fn setup_let_shadow_restored_after_taken_arm() {
    // A setup `let` shadowing an outer binding is arm-local: post-match
    // code still sees the outer value. (Payload shadowing was already
    // pinned; this pins setup `let`s, which need their own save/restore
    // entries — without them the final `n` silently read 5, not 100.)
    let (v, out) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let n = 100 let v = Opt::Some(1) let r = match v { Opt::Some(m) => { let n = 5 n * 10 }, Opt::None => 0 } print(r) print(n) return r + n }",
        "main",
    );
    assert_eq!(out, vec!["50".to_string(), "100".to_string()]);
    assert_eq!(v, 150);
}

#[test]
fn single_expr_arms_still_work() {
    // Regression: the old shape is untouched (mirrors enum_gates).
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(41) return match v { Opt::Some(n) => n + 1, Opt::None => 0 } }",
        "main",
    );
    assert_eq!(v, 42);
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::None() return match v { Opt::Some(n) => n + 1, Opt::None => 7 } }",
        "main",
    );
    assert_eq!(v, 7);
}

#[test]
fn arm_shadow_does_not_clobber_outer() {
    // BUG-A class (silent wrong answer): an arm-local `let` shadowing an
    // outer binding must not change the outer value after the match.
    let (v, out) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let n = 100 let v = Opt::Some(1) let r = match v { Opt::Some(n) => { let n = n + 1 n * 10 }, Opt::None => 0 } print(r) print(n) return r + n }",
        "main",
    );
    assert_eq!(out, vec!["20".to_string(), "100".to_string()]);
    assert_eq!(v, 120, "arm r=20, outer n still 100");
}

#[test]
fn arm_assign_to_outer_persists() {
    // The mirror edge: assigning (not shadowing) an outer variable inside
    // the arm must persist — same as an `if` block body. If the
    // save/restore around bindings were over-eager, `total` would read 0.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let total = 0 let v = Opt::Some(5) let r = match v { Opt::Some(n) => { total = total + n n * 2 }, Opt::None => { total = 99 0 } } return r * 100 + total }",
        "main",
    );
    assert_eq!(v, 1005, "r=10 persists total=5 -> 10*100+5");
}

#[test]
fn arm_with_if_and_loop_inside() {
    // Control flow nested inside the arm block works (self-contained).
    // (`if` is a statement, so the trailing value stays an expression.)
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(3) return match v { Opt::Some(n) => { let t = 0 for i in 0..n { t = t + i } let r = 0 if t == 3 { r = 10 } r * t }, Opt::None => 0 } }",
        "main",
    );
    assert_eq!(v, 30, "0+1+2=3, so r=10 and 10*3");
}

#[test]
fn return_inside_arm_returns_function() {
    // `return` inside arm setup exits the whole function immediately
    // (skipping the arm value and the `print(r)` after the match).
    let (v, out) = run_src(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { let r = match v { Opt::Some(n) => { print(n) return 99 n }, Opt::None => 0 } print(r) return r } fn main() -> i32 { print(f(Opt::Some(1))) print(f(Opt::None())) return 0 }",
        "main",
    );
    assert_eq!(out, vec!["1".to_string(), "99".to_string(), "0".to_string(), "0".to_string()]);
    assert_eq!(v, 0);
}

#[test]
fn map_literal_shape_still_parse_error() {
    // `{ ... }` after `=>` is an arm block, never a map literal: map
    // shapes fail with E-PARSE exactly as before (SPEC §2b documented
    // that shape as a parse failure, and it stays one).
    parse_err_code(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(1) return match v { Opt::Some(n) => {\"a\": 1}, Opt::None => 0 } }",
        "E-PARSE",
    );
}

#[test]
fn empty_and_stmts_only_blocks_are_parse_errors() {
    // An arm block must end with a value expression: `{}` and
    // statements-only blocks have no value, so they are E-PARSE.
    parse_err_code(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::None() return match v { Opt::None => {}, _ => 0 } }",
        "E-PARSE",
    );
    parse_err_code(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::None() return match v { Opt::None => { let x = 1 }, _ => 0 } }",
        "E-PARSE",
    );
}

#[test]
fn break_crossing_arm_is_eloop() {
    // `break` targeting a loop outside the arm has no lowering to land
    // on (arms lower into the branch run): loud E-LOOP at check time,
    // never a runtime jump failure. A loop fully inside the arm is fine.
    check_err_code(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let total = 0 for i in 0..3 { let v = Opt::Some(i) total = total + match v { Opt::Some(n) => { break n }, Opt::None => 0 } } return total }",
        "E-LOOP",
    );
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(9) return match v { Opt::Some(n) => { let t = 0 for i in 0..3 { if i == 2 { break } t = t + i } t }, Opt::None => 0 } }",
        "main",
    );
    assert_eq!(v, 1, "break inside the arm's own loop works: 0+1");
}

#[test]
fn v2_has_no_match() {
    // The v2 grammar has no `match`: a v2 program using it fails to parse.
    // run-v2 is unaffected by the v1 feature (documented exclusion).
    let src = "fn main() -> i32 {\n  return match x { _ => 1 }\n}\n";
    assert!(
        klang::parser::v2::parse_v2_program(src).is_err(),
        "v2 must not accept match"
    );
}

#[test]
fn v2_block_programs_still_run() {
    // Positive v2 control: multi-statement `if` blocks (whose semantics
    // arms mirror) still evaluate correctly through run-v2.
    let src = "fn f(n: i32) -> i32 {\n  if n == 0 {\n    5\n  } else {\n    7\n  }\n}\nfn main() -> i32 {\n  return f(0) * 100 + f(1)\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    let (v, _) = klang::runtime::v2::run_v2_program(&prog, "main").expect("runs");
    assert_eq!(v, 507);
}

#[test]
fn jit_rejects_enum_match() {
    // Enums lower through struct ops, outside the int-only JIT: loud
    // rejection (never silent wrong code), block arms included.
    let prog = parse(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(1) return match v { Opt::Some(n) => { let y = n + 1 y * 2 }, Opt::None => 0 } }",
    );
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported"), "{err}");
}

#[test]
fn fmt_round_trips_block_arm() {
    // Formatted block arms re-parse to the same program and run the same.
    let src = "enum Opt { Some(x: i32), None } fn pick(v: Opt) -> i32 { return match v { Opt::Some(n) => { let y = n + 1 print(y) y * 2 }, Opt::None => 7 } } fn main() -> i32 { return pick(Opt::Some(20)) + pick(Opt::None()) }";
    let once = klang::fmt::fmt_program(&parse(src));
    assert!(once.contains("=> {"), "fmt keeps block arm:\n{once}");
    let twice = klang::fmt::fmt_program(&parse(&once));
    assert_eq!(once, twice, "fmt idempotent");
    let prog = parse(&once);
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(out, vec!["21".to_string()]);
    assert_eq!(v, 49);
}

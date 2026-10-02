//! KLANG-FOUNDATION-2 Step 2 — match guards.
//!
//! `Pattern if condition => body`: taken only when the pattern matches
//! AND the guard is truthy; a matching pattern with a false guard falls
//! through to the next arm (not a crash, not a silent wrong match). The
//! guard is never evaluated when the pattern does not match.
//! Exhaustiveness is strict: a guarded arm never covers its variant (a
//! guard may be false), so every variant still needs an unguarded arm or
//! a wildcard — checked programs therefore always land, and total guard
//! failure is reachable only through unchecked MIR, where it is a loud
//! `E-RUNTIME`, never a silent default.
//!
//! Backend coverage note: the v2 language has no `enum`/`match`, so "v2
//! backend" coverage is the documented exclusion plus a positive control;
//! the int-only JIT loudly rejects every enum `match`, guards included.

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

#[test]
fn guard_false_falls_through_to_later_arm() {
    // A matching pattern with a false guard is NOT a crash and NOT a
    // wrong match: it falls through to the next arm.
    let (v, out) = run_src(
        "enum Opt { Some(x: i32), None } fn classify(v: Opt) -> i32 { return match v { Opt::Some(n) if n > 10 => 1, Opt::Some(n) => 2, Opt::None => 3 } } fn main() -> i32 { print(classify(Opt::Some(20))) print(classify(Opt::Some(5))) print(classify(Opt::None())) return classify(Opt::Some(20)) * 100 + classify(Opt::Some(5)) * 10 + classify(Opt::None()) }",
        "main",
    );
    assert_eq!(out, vec!["1".to_string(), "2".to_string(), "3".to_string()]);
    assert_eq!(v, 123);
}

#[test]
fn guard_true_takes_arm_over_later_arms() {
    let (v, _) = run_src(
        "enum E { A(x: i32), B(x: i32) } fn f(v: E) -> i32 { return match v { E::A(n) if n == 1 => 10, E::A(n) if n == 2 => 20, E::A(n) => 30, E::B(n) => 40 } } fn main() -> i32 { return f(E::A(1)) * 1000 + f(E::A(2)) * 100 + f(E::A(9)) * 10 + f(E::B(0)) }",
        "main",
    );
    assert_eq!(v, 12340, "10*1000+20*100+30*10+40");
}

#[test]
fn guard_never_evaluated_on_nonmatching_pattern() {
    // The easy bug: evaluating the guard against a non-matching pattern.
    // Here the guard divides by the binding; an outer `n = 0` makes the
    // bug LOUD (division by zero) if the guard runs on the `None` path.
    // Correct: the guard is skipped, the wildcard takes it, no error.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let n = 0 let v = Opt::None() return match v { Opt::Some(n) if 10 / n > 0 => 1, _ => 7 } }",
        "main",
    );
    assert_eq!(v, 7);
}

#[test]
fn guard_sees_bindings_and_setup_lets() {
    // Guards observe payload bindings AND multi-statement setup lets
    // (Step 1 + Step 2 compose: setup runs before the guard is checked).
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(4) return match v { Opt::Some(n) => { let doubled = n * 2 doubled + 1 }, Opt::None => 0 } }",
        "main",
    );
    assert_eq!(v, 9);
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(4) return match v { Opt::Some(n) if n * 2 == 8 => 1, Opt::Some(n) => 2, Opt::None => 3 } }",
        "main",
    );
    assert_eq!(v, 1, "guard reads the payload binding");
}

#[test]
fn guard_false_restores_outer_bindings() {
    // BUG-A class: after a false guard, later arms must see OUTER values
    // for same-named bindings — not the first arm's payload. If the
    // fallthrough path skipped the restore, `_ => n` would read 5.
    let (v, out) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let n = 100 let v = Opt::Some(5) let r = match v { Opt::Some(n) if n > 10 => n, _ => n } print(r) print(n) return r }",
        "main",
    );
    assert_eq!(out, vec!["100".to_string(), "100".to_string()]);
    assert_eq!(v, 100);
}

#[test]
fn guard_sees_setup_let() {
    // Steps 1+2 compose for real: setup statements run BEFORE the guard
    // is checked, so the guard observes setup bindings (HIR checks the
    // same order). BUG-A class: with the guard evaluated first, it
    // silently read 0 and `f(Some(4))` answered 4 instead of 108.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) if doubled == 8 => { let doubled = n * 2 doubled + 100 }, Opt::Some(n) => n, Opt::None => 0 } } fn main() -> i32 { return f(Opt::Some(4)) * 1000 + f(Opt::Some(5)) }",
        "main",
    );
    assert_eq!(v, 108005, "108*1000+5");
}

#[test]
fn setup_let_shadow_restored_on_fallthrough() {
    // BUG-A class: a setup `let` shadowing an outer binding must not
    // leak into later arms when the guard fails. Without setup names in
    // the save/restore set, `_ => n` silently read 5 instead of 100.
    let (v, out) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let n = 100 let v = Opt::Some(5) let r = match v { Opt::Some(m) if m > 10 => { let n = 5 n }, _ => n } print(r) return r }",
        "main",
    );
    assert_eq!(out, vec!["100".to_string()]);
    assert_eq!(v, 100);
}

#[test]
fn setup_effects_persist_on_fallthrough() {
    // Documented `if`-block semantics: assignments to OUTER variables in
    // setup persist even when the guard then fails (only `let`s are
    // arm-local). The setup observably ran before the fallthrough:
    // total=0+5, then `_ => total + 1` = 6, so 6*100+5.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let total = 0 let v = Opt::Some(5) let r = match v { Opt::Some(m) if m > 10 => { total = total + m m }, _ => total + 1 } return r * 100 + total }",
        "main",
    );
    assert_eq!(v, 605);
}

#[test]
fn guarded_arm_does_not_cover_exhaustiveness() {
    // Strict rule: guarded arms never cover, so an all-guarded match
    // without a wildcard is E-MATCH-EXHAUSTIVE (loud at check, not a
    // runtime panic like Rust's unenforced fallthrough).
    check_err_code(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) if n > 0 => 1 } }",
        "E-MATCH-EXHAUSTIVE",
    );
    // ...but with a wildcard (or an unguarded same-pattern arm) it checks.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) if n > 0 => 1, _ => 0 } } fn main() -> i32 { return f(Opt::Some(5)) * 10 + f(Opt::Some(0)) }",
        "main",
    );
    assert_eq!(v, 10);
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) if n > 0 => 1, Opt::Some(n) => 2, Opt::None => 3 } } fn main() -> i32 { return f(Opt::Some(5)) * 10 + f(Opt::Some(0)) }",
        "main",
    );
    assert_eq!(v, 12);
}

#[test]
fn guarded_duplicate_rules() {
    // Guarded-then-unguarded same pattern: reachable, no diagnostic.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) if n > 0 => 1, Opt::Some(n) => 2, Opt::None => 3 } } fn main() -> i32 { return f(Opt::Some(0)) }",
        "main",
    );
    assert_eq!(v, 2);
    // Unguarded-then-guarded same pattern: the guarded arm is dead.
    check_err_code(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) => 1, Opt::Some(n) if n > 0 => 2, Opt::None => 3 } }",
        "E-MATCH-DUPLICATE",
    );
    // Plain duplicates still diagnosed (regression).
    check_err_code(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) => 1, Opt::Some(n) => 2, Opt::None => 3 } }",
        "E-MATCH-DUPLICATE",
    );
}

#[test]
fn guarded_wildcard_does_not_hide_later_arms() {
    // A guarded wildcard covers nothing: later arms still run, no
    // E-MATCH-UNREACHABLE. An UNGUARDED wildcard still hides everything.
    // (Strict exhaustiveness still applies: `Some` needs its own arm.)
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { _ if false => 1, Opt::Some(n) => 0, Opt::None => 2 } } fn main() -> i32 { return f(Opt::None()) }",
        "main",
    );
    assert_eq!(v, 2);
    check_err_code(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { _ => 1, Opt::None => 2 } }",
        "E-MATCH-UNREACHABLE",
    );
}

#[test]
fn total_guard_failure_is_loud_never_silent() {
    // Checked programs always land (strict exhaustiveness), so total
    // guard failure is reachable only via unchecked MIR. It must be a
    // loud E-RUNTIME — never a silent 0 or a hung/wrapped value.
    let prog = parse(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(5) return match v { Opt::Some(n) if n > 10 => n } }",
    );
    let mir = klang::mir::lower(&prog);
    let err =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    assert_eq!(err.code, "E-RUNTIME", "{}", err.to_json());
}

#[test]
fn guard_nonbool_condition_is_etype() {
    check_err_code(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) if \"s\" => 1, _ => 0 } }",
        "E-TYPE",
    );
}

#[test]
fn guard_on_wildcard_with_block_body() {
    // Steps 1+2 compose on wildcards too: setup, guard, trailing value.
    let (v, out) = run_src(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { let k = 3 return match v { _ if k > 1 => { let m = k * 10 print(m) m + 1 }, _ => 0 } } fn main() -> i32 { return f(Opt::None()) }",
        "main",
    );
    assert_eq!(out, vec!["30".to_string()]);
    assert_eq!(v, 31);
}

#[test]
fn v2_has_no_guards() {
    // The v2 grammar has no `match` at all, hence no guards either.
    let src = "fn main() -> i32 {\n  return match x { _ if y => 1 }\n}\n";
    assert!(
        klang::parser::v2::parse_v2_program(src).is_err(),
        "v2 must not accept match guards"
    );
}

#[test]
fn v2_if_blocks_still_run() {
    // Positive v2 control: conditional dispatch in v2 is unaffected.
    let src = "fn f(n: i32) -> i32 {\n  if n > 10 {\n    1\n  } else {\n    2\n  }\n}\nfn main() -> i32 {\n  return f(20) * 10 + f(5)\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    let (v, _) = klang::runtime::v2::run_v2_program(&prog, "main").expect("runs");
    assert_eq!(v, 12);
}

#[test]
fn jit_rejects_guarded_match() {
    // Guards lower to the same enum-dispatch MIR the int-only JIT never
    // accepts: loud rejection, never silent wrong code.
    let prog = parse(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(20) return match v { Opt::Some(n) if n > 10 => 1, _ => 0 } }",
    );
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported"), "{err}");
}

#[test]
fn fmt_round_trips_guard() {
    let src = "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) if n > 10 => 1, Opt::Some(n) => 2, Opt::None => 3 } } fn main() -> i32 { return f(Opt::Some(20)) }";
    let once = klang::fmt::fmt_program(&parse(src));
    assert!(once.contains("if (n > 10)"), "fmt keeps guard:\n{once}");
    let twice = klang::fmt::fmt_program(&parse(&once));
    assert_eq!(once, twice, "fmt idempotent");
    let prog = parse(&once);
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 1);
}

//! KLANG-FOUNDATION-2 Step 3 — closures (by-value capture).
//!
//! A closure literal `fn(params) -> type { body }` is a first-class value
//! capturing its enclosing scope **by value**: the snapshot is taken when
//! the literal runs, so later mutations — including the outer function
//! having returned — never affect the closure. Closures can be stored in
//! variables, passed as arguments, and returned from functions.
//!
//! Backend coverage note: the v2 language has no closure literals (its
//! lambdas are `flow(...)`), so "v2 backend" coverage is the documented
//! exclusion (v2 rejects the literal) plus a positive control. The
//! int-only JIT loudly rejects every closure creation (never silent
//! wrong code).
//!
//! BUG-A discipline: the worst failure here is a silent wrong answer
//! (by-reference capture reading a later value, or dynamic dispatch
//! landing on the wrong function), so several tests pin exact values
//! where a capture/dispatch bug would compute something else without
//! any error.

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

fn run_src_value(src: &str, entry: &str) -> (klang::runtime::Value, Vec<String>) {
    let prog = parse(src);
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output_value(&mir, entry, &[], &HashMap::new()).expect("runs")
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
fn capture_and_call_later() {
    // Basic case: store in a variable, call later (twice).
    let (v, _) = run_src(
        "fn main() -> i32 { let n = 20 let addn = fn(x: i32) -> i32 { return x + n } let a = addn(1) let b = addn(2) return a + b }",
        "main",
    );
    assert_eq!(v, 43, "21 + 22");
}

#[test]
fn capture_is_snapshot_not_reference() {
    // BUG-A class: if the capture were by reference, reassigning `n`
    // after creation would change the answer 10 -> 99 with no error.
    let (v, _) = run_src(
        "fn main() -> i32 { let n = 10 let f = fn() -> i32 { return n } n = 99 return f() }",
        "main",
    );
    assert_eq!(v, 10, "snapshot at creation, not 99");
}

#[test]
fn mutation_inside_closure_does_not_escape() {
    // The mirror direction: assigning a captured name inside the closure
    // mutates only the call frame, never the outer binding. Aliasing
    // would silently yield 1111 instead of 1110.
    let (v, _) = run_src(
        "fn main() -> i32 { let n = 10 let f = fn() -> i32 { n = n + 1 return n } let a = f() return a * 100 + n }",
        "main",
    );
    assert_eq!(v, 1110, "a=11, outer n still 10");
}

#[test]
fn closure_outlives_definer() {
    // The closure is returned and called after the outer function has
    // returned: a dangling frame reference would read 0 (the unchecked
    // default) and silently answer 5 instead of 105.
    let (v, _) = run_src(
        "fn make() -> fn(i32) -> i32 { let base = 100 return fn(x: i32) -> i32 { return x + base } } fn main() -> i32 { let f = make() return f(5) }",
        "main",
    );
    assert_eq!(v, 105);
}

#[test]
fn closure_passed_as_argument() {
    // A literal passed directly as an argument dispatches correctly.
    let (v, _) = run_src(
        "fn apply(f: fn(i32) -> i32, x: i32) -> i32 { return f(x) } fn main() -> i32 { return apply(fn(x: i32) -> i32 { return x * 2 }, 21) }",
        "main",
    );
    assert_eq!(v, 42);
    // ...and so does a stored closure passed by variable.
    let (v, _) = run_src(
        "fn apply(f: fn(i32) -> i32, x: i32) -> i32 { return f(x) } fn main() -> i32 { let d = fn(x: i32) -> i32 { return x * 2 } return apply(d, 21) }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn distinct_closures_keep_distinct_captures() {
    // Two closures from the same maker close over different values; a
    // shared/global capture slot would silently mix them (611 vs 612).
    let (v, _) = run_src(
        "fn make_adder(a: i32) -> fn(i32) -> i32 { return fn(x: i32) -> i32 { return x + a } } fn main() -> i32 { let add5 = make_adder(5) let add10 = make_adder(10) return add5(1) * 100 + add10(1) }",
        "main",
    );
    assert_eq!(v, 611, "6*100+11");
}

#[test]
fn closure_with_no_captures() {
    let (v, _) = run_src(
        "fn main() -> i32 { let sq = fn(x: i32) -> i32 { return x * x } return sq(6) }",
        "main",
    );
    assert_eq!(v, 36);
}

#[test]
fn nested_closures_see_both_scopes() {
    // The inner literal captures the outer literal's parameter `x` (bound
    // in the lifted outer frame) and the grand-outer `a` alike.
    let (v, _) = run_src(
        "fn outer() -> i32 { let a = 1 let f = fn(x: i32) -> i32 { let g = fn(y: i32) -> i32 { return a + x + y } return g(10) } return f(100) } fn main() -> i32 { return outer() }",
        "main",
    );
    assert_eq!(v, 111, "1 + 100 + 10");
}

#[test]
fn closure_binding_shadows_same_named_function() {
    // Values-first, checked and runtime alike: the binding wins over the
    // static function. Statics-first dispatch would silently answer 4100.
    let (v, _) = run_src(
        "fn f(x: i32) -> i32 { return x * 100 } fn main() -> i32 { let f = fn(x: i32) -> i32 { return x + 1 } return f(41) }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn closure_param_shadow_outer_and_compose() {
    // Params shadow outer bindings inside the body only; the outer value
    // is unchanged afterwards (silent-wrong-answer edge for the lifter).
    let (v, out) = run_src(
        "fn main() -> i32 { let x = 1000 let f = fn(x: i32) -> i32 { return x + 1 } let r = f(41) print(r) print(x) return r + x }",
        "main",
    );
    assert_eq!(out, vec!["42".to_string(), "1000".to_string()]);
    assert_eq!(v, 1042);
}

#[test]
fn closure_variable_reassigned() {
    // `let` carries no annotation (types flow from the value); a closure
    // variable can be reassigned to another closure of the same shape,
    // and each call dispatches to the CURRENT value (stale-dispatch
    // would silently answer 1 instead of 200).
    let (v, _) = run_src(
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x + 1 } let a = f(0) f = fn(x: i32) -> i32 { return x * 100 } return a + f(2) }",
        "main",
    );
    assert_eq!(v, 201, "a=1, f(2)=200");
}

#[test]
fn closure_captures_match_binding() {
    // Steps 1+3 compose: a literal created in an arm captures the arm's
    // payload binding.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(21) return match v { Opt::Some(n) => { let f = fn() -> i32 { return n * 2 } f() }, Opt::None => 0 } }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn closure_called_in_guard() {
    // Steps 2+3 compose: a guard calls a closure over the payload.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(21) let big = fn(n: i32) -> bool { return n > 10 } return match v { Opt::Some(n) if big(n) => 1, _ => 0 } }",
        "main",
    );
    assert_eq!(v, 1);
}

#[test]
fn print_closure_renders_placeholder() {
    // Opaque by design: captures never leak through rendering.
    let (_, out) = run_src(
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x } print(f) return 0 }",
        "main",
    );
    assert_eq!(out, vec!["<closure>".to_string()]);
}

#[test]
fn calling_nonclosure_variable_is_etype() {
    // Calling a bound non-closure is rejected at check time, never a
    // runtime jump failure.
    check_err_code("fn main() -> i32 { let x = 5 return x(1) }", "E-TYPE");
}

#[test]
fn closure_arity_mismatch_is_earity() {
    check_err_code(
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x } return f(1, 2) }",
        "E-ARITY",
    );
    check_err_code(
        "fn apply(f: fn(i32) -> i32, x: i32) -> i32 { return f(x) } fn main() -> i32 { let g = fn(x: i32, y: i32) -> i32 { return x } return apply(g, 1) }",
        "E-TYPE",
    );
}

#[test]
fn closure_arg_type_mismatch_is_etype() {
    check_err_code(
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x } return f(\"hi\") }",
        "E-TYPE",
    );
}

#[test]
fn closure_return_type_checked() {
    check_err_code(
        "fn main() -> i32 { let f = fn(x: i32) -> str { return x } return 0 }",
        "E-TYPE",
    );
}

#[test]
fn closure_arithmetic_is_etype() {
    // A closure in arithmetic would silently read 0/capture-count at
    // runtime; the checker rejects it loudly instead.
    check_err_code(
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x } return f + 1 }",
        "E-TYPE",
    );
}

#[test]
fn self_reference_by_let_name_is_undefined() {
    // A closure cannot name its own `let` binding (bound only after the
    // literal checks): loud E-UNDEFINED, never a hang or a self-call
    // past the depth guard via a half-initialized slot.
    check_err_code(
        "fn main() -> i32 { let f = fn(n: i32) -> i32 { return f(n) } return f(1) }",
        "E-UNDEFINED",
    );
}

#[test]
fn spawn_closure_value_rejected() {
    // Spawning a closure value is E-TYPE at check time (the child frame
    // could not see the snapshots).
    check_err_code(
        "fn one() -> i32 { return 1 } fn main() -> i32 { task_group { let f = fn() -> i32 { return 1 } let h = spawn f() return await h } }",
        "E-TYPE",
    );
}

#[test]
fn spawn_inside_closure_body_rejected() {
    // A bare `spawn` inside a closure body has no group (closures check
    // with a fresh task context, since they may escape).
    check_err_code(
        "fn one() -> i32 { return 1 } fn main() -> i32 { let f = fn() -> i32 { let h = spawn one() return 1 } return f() }",
        "E-SPAWN-OUTSIDE-GROUP",
    );
}

#[test]
fn await_inside_closure_body_rejected() {
    // Same for `await`: the closure may outlive the group, so awaiting
    // inside is E-AWAIT-OUTSIDE-GROUP even when textually inside one.
    check_err_code(
        "fn one() -> i32 { return 1 } fn main() -> i32 { task_group { let h = spawn one() let f = fn() -> i32 { return await h } return await h } }",
        "E-AWAIT-OUTSIDE-GROUP",
    );
}

#[test]
fn closure_with_own_task_group_runs() {
    // A closure that enters and leaves its OWN group is sound (nothing
    // escapes) and runs.
    let (v, _) = run_src(
        "fn one() -> i32 { return 1 } fn main() -> i32 { let f = fn() -> i32 { task_group { let h = spawn one() return await h } } return f() + 41 }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn v2_has_no_closure_literals() {
    // The v2 grammar has no `fn(...)` expression: a v2 program using one
    // fails to parse (documented exclusion).
    let src = "fn main() -> i32 {\n  let f = fn(x: i32) -> i32 { return x }\n  return 0\n}\n";
    assert!(
        klang::parser::v2::parse_v2_program(src).is_err(),
        "v2 must not accept closure literals"
    );
}

#[test]
fn v2_higher_order_flow_still_runs() {
    // Positive v2 control: v2's own higher-order construct (`flow`) is
    // unaffected by the v1 feature.
    let src = "fn main() -> i32 {\n  let base = 10 + 5\n  let classify = flow(score: i32, dep=base: i32) -> i32 { score + base }\n  return classify(27)\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    let (v, _) = klang::runtime::v2::run_v2_program(&prog, "main").expect("runs");
    assert_eq!(v, 42, "27 + computed dep 15");
}

#[test]
fn jit_rejects_closure_creation() {
    // Closures lower through snapshots + dynamic dispatch, outside the
    // int-only JIT: loud rejection (never silent wrong code).
    let prog = parse(
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x + 1 } return f(41) }",
    );
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported"), "{err}");
}

#[test]
fn fmt_round_trips_closure() {
    // Formatted closures re-parse to the same program and run the same.
    let src = "fn main() -> i32 { let n = 20 let addn = fn(x: i32) -> i32 { return x + n } return addn(22) }";
    let once = klang::fmt::fmt_program(&parse(src));
    assert!(once.contains("fn(x: i32) -> i32"), "fmt keeps closure:\n{once}");
    let twice = klang::fmt::fmt_program(&parse(&once));
    assert_eq!(once, twice, "fmt idempotent");
    let prog = parse(&once);
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn returned_closure_value_renders_and_compares() {
    // A returned closure is a real value: it survives the value channel
    // (`run_with_output_value`) and renders opaquely.
    let (v, _) = run_src_value(
        "fn make() -> fn(i32) -> i32 { let k = 7 return fn(x: i32) -> i32 { return x + k } } fn main() -> fn(i32) -> i32 { return make() }",
        "main",
    );
    assert_eq!(v.render(), "<closure>");
}

//! KLANG-FOUNDATION-3 Step 1 — recursive/nested enum payloads.
//!
//! A variant payload may name the enum itself (directly or mutually), e.g.
//! `Cons(x: i32, xs: List)`. No new allocation strategy was needed: the
//! runtime `Value` is already heap-indirected through `Vec`, and MIR
//! lowers enums to struct ops, so nesting is naturally unbounded. The
//! actual fix is in the checker, which now seeds all enum names before
//! resolving payloads (previously a self/mutual reference silently
//! erased to `Unknown`). Recursive traversal still honors the 1024-frame
//! call-depth cap with a loud `E-RUNTIME`, never a native overflow.
//!
//! Backend coverage note: the v2 language has no enums/`match` at all,
//! so "v2 backend" coverage is the documented exclusion (v2 rejects the
//! declaration) plus a positive control. The int-only JIT loudly rejects
//! every enum lowering (tags are strings), recursive or not.
//!
//! BUG-A discipline: the silent failure here is a mistyped payload being
//! accepted (erased `Unknown` unifies with anything), so several tests
//! pin exact `E-TYPE` rejections and exact computed values.

//! Stack discipline for the deep tests below: interpreter frames are
//! large in debug builds, so even modest recursion depths overflow the
//! 2 MiB test threads (this predates recursive enums — a non-recursive
//! `match` countdown aborts the same way). The CLI always runs on its
//! 256 MiB deep-stack worker, so deep tests run through the same worker
//! (`klang::with_deep_stack`); in-process tests stay shallow (<= 25).

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
fn list_build_and_length() {
    // Basic case: a real linked list built and traversed recursively.
    let (v, _) = run_src(
        "enum List { Cons(x: i32, xs: List), Nil } fn llen(l: List) -> i32 { return match l { List::Cons(x, xs) => 1 + llen(xs), List::Nil => 0 } } fn main() -> i32 { return llen(List::Cons(1, List::Cons(2, List::Cons(3, List::Nil())))) }",
        "main",
    );
    assert_eq!(v, 3);
}

#[test]
fn tree_inorder_sum() {
    // Basic case: a real binary tree with two recursive payloads.
    let (v, _) = run_src(
        "enum Tree { Node(l: Tree, r: Tree), Leaf(v: i32) } fn tsum(t: Tree) -> i32 { return match t { Tree::Node(l, r) => tsum(l) + tsum(r), Tree::Leaf(v) => v } } fn main() -> i32 { return tsum(Tree::Node(Tree::Leaf(1), Tree::Node(Tree::Leaf(2), Tree::Leaf(3)))) }",
        "main",
    );
    assert_eq!(v, 6, "1 + (2 + 3)");
}

#[test]
fn recursive_expr_eval() {
    // A recursive payload feeding arithmetic: every level dispatches.
    let (v, _) = run_src(
        "enum Expr { Add(l: Expr, r: Expr), Lit(v: i32) } fn eval(e: Expr) -> i32 { return match e { Expr::Add(l, r) => eval(l) + eval(r), Expr::Lit(v) => v } } fn main() -> i32 { return eval(Expr::Add(Expr::Lit(20), Expr::Add(Expr::Lit(2), Expr::Lit(20)))) }",
        "main",
    );
    assert_eq!(v, 42, "20 + (2 + 20)");
}

#[test]
fn self_payload_is_nominal_not_erased() {
    // BUG-A class: before name seeding, `xs: List` erased to `Unknown`
    // and `Cons(1, 2)` checked clean. Now the mistyped payload is a loud
    // `E-TYPE` naming the expected type.
    check_err_code(
        "enum List { Cons(x: i32, xs: List), Nil } fn main() -> i32 { let l = List::Cons(1, 2) return 0 }",
        "E-TYPE",
    );
    // And a well-typed construction still checks.
    let (v, _) = run_src(
        "enum List { Cons(x: i32, xs: List), Nil } fn main() -> i32 { let l = List::Cons(1, List::Nil()) return 1 }",
        "main",
    );
    assert_eq!(v, 1);
}

#[test]
fn mutual_recursion_resolves() {
    // Two enums naming each other: both payloads nominal, program runs.
    let (v, _) = run_src(
        "enum A { X(y: B) } enum B { Y(z: A), Z } fn depth(b: B) -> i32 { return match b { B::Y(z) => 1, B::Z => 0 } } fn main() -> i32 { return depth(B::Z) + depth(B::Y(A::X(B::Z))) }",
        "main",
    );
    assert_eq!(v, 1, "0 + 1");
    // Mistyped across the cycle is loud, not erased.
    check_err_code(
        "enum A { X(y: B) } enum B { Y(z: A), Z } fn main() -> i32 { let a = A::X(1) return 0 }",
        "E-TYPE",
    );
}

#[test]
fn forward_reference_resolves() {
    // An enum naming a LATER-declared enum: same silent-erasure class,
    // now nominal.
    let (v, _) = run_src(
        "enum A { X(y: B) } enum B { Y(n: i32), Z } fn get(a: A) -> i32 { return match a { A::X(y) => 7 } } fn main() -> i32 { return get(A::X(B::Y(1))) }",
        "main",
    );
    assert_eq!(v, 7);
}

#[test]
fn deep_recursion_fails_loud() {
    // A 1500-long list built iteratively (no deep source nesting), then
    // traversed recursively past the 1024-frame cap: loud `E-RUNTIME`,
    // never a hang or native stack overflow. Runs on the deep-stack
    // worker exactly like the CLI (see header note).
    let code = klang::with_deep_stack(|| {
        let src = "enum List { Cons(x: i32, xs: List), Nil } fn llen(l: List) -> i32 { return match l { List::Cons(x, xs) => 1 + llen(xs), List::Nil => 0 } } fn main() -> i32 { let l = List::Nil() for i in 0..1500 { l = List::Cons(i, l) } return llen(l) }";
        let prog = parse(src);
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
        let mir = klang::mir::lower(&prog);
        match klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()) {
            Ok((v, _)) => format!("OK:{v}"),
            Err(e) => e.code.clone(),
        }
    });
    assert_eq!(code, "E-RUNTIME", "depth cap must fire loudly");
}

#[test]
fn moderate_depth_runs_exact() {
    // At depth 500 (under the cap, on the deep-stack worker like the
    // CLI): exact length, proving traversal is faithful at scale — an
    // off-by-one in tag dispatch would show here, not just crash.
    let v = klang::with_deep_stack(|| {
        let (v, _) = run_src(
            "enum List { Cons(x: i32, xs: List), Nil } fn llen(l: List) -> i32 { return match l { List::Cons(x, xs) => 1 + llen(xs), List::Nil => 0 } } fn main() -> i32 { let l = List::Nil() for i in 0..500 { l = List::Cons(i, l) } return llen(l) }",
            "main",
        );
        v
    });
    assert_eq!(v, 500);
    // Shallow twin runs in-process on the 2 MiB test thread.
    let (v, _) = run_src(
        "enum List { Cons(x: i32, xs: List), Nil } fn llen(l: List) -> i32 { return match l { List::Cons(x, xs) => 1 + llen(xs), List::Nil => 0 } } fn main() -> i32 { let l = List::Nil() for i in 0..25 { l = List::Cons(i, l) } return llen(l) }",
        "main",
    );
    assert_eq!(v, 25);
}

#[test]
fn recursive_payload_in_guard() {
    // Steps compose: a guard over a recursively-bound payload.
    let (v, _) = run_src(
        "enum List { Cons(x: i32, xs: List), Nil } fn head_big(l: List) -> i32 { return match l { List::Cons(x, xs) if x > 10 => 1, List::Cons(x, xs) => 2, List::Nil => 3 } } fn main() -> i32 { return head_big(List::Cons(20, List::Nil())) * 10 + head_big(List::Cons(1, List::Nil())) }",
        "main",
    );
    assert_eq!(v, 12, "1*10+2");
}

#[test]
fn recursive_value_captured_by_closure() {
    // A closure snapshotting a recursive value sees the whole structure.
    let (v, _) = run_src(
        "enum List { Cons(x: i32, xs: List), Nil } fn llen(l: List) -> i32 { return match l { List::Cons(x, xs) => 1 + llen(xs), List::Nil => 0 } } fn main() -> i32 { let l = List::Cons(7, List::Nil()) let f = fn() -> i32 { return llen(l) } return f() * 10 + llen(l) }",
        "main",
    );
    assert_eq!(v, 11, "1*10+1");
}

#[test]
fn duplicate_enums_and_variants_still_rejected() {
    // Regression on the restructured table build: duplicate diagnostics
    // are preserved exactly.
    check_err_code(
        "enum Opt { Some(x: i32), None } enum Opt { Some(x: i32), None } fn main() -> i32 { return 0 }",
        "E-DUPLICATE",
    );
    check_err_code(
        "enum Opt { Some(x: i32), Some(y: i32) } fn main() -> i32 { return 0 }",
        "E-DUPLICATE",
    );
}

#[test]
fn non_recursive_enums_unchanged() {
    // The seeding pass must not alter plain enums: exact dispatch,
    // exhaustiveness, and ctor arity all behave as before.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(41) return match v { Opt::Some(n) => n + 1, Opt::None => 0 } }",
        "main",
    );
    assert_eq!(v, 42);
    check_err_code(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) => n } }",
        "E-MATCH-EXHAUSTIVE",
    );
}

#[test]
fn v2_has_no_enums() {
    // The v2 language has no enums or `match` (documented exclusion from
    // FOUNDATION-2, unchanged by recursion support).
    let src = "enum List { Cons(x: i32) }\nfn main() -> i32 {\n  return 0\n}\n";
    assert!(
        klang::parser::v2::parse_v2_program(src).is_err(),
        "v2 must not accept enum declarations"
    );
}

#[test]
fn v2_recursive_shapes_still_run() {
    // Positive v2 control: v2 recursion over its own shapes is
    // unaffected (shallow countdown; deep v2 recursion needs the
    // deep-stack worker like everything else in debug builds).
    let v = klang::with_deep_stack(|| {
        let src = "fn down(n: i32) -> i32 {\n  if n == 0 {\n    0\n  } else {\n    down(n - 1)\n  }\n}\nfn main() -> i32 {\n  return down(42)\n}\n";
        let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
        let _mir = klang::mir::v2_lowering::lower_v2_program(
            &prog.schemas,
            &prog.echo_fns,
            &prog.echo_bodies,
            &prog.flows,
            &prog.functions,
        );
        let (v, _) = klang::runtime::v2::run_v2_program(&prog, "main").expect("runs");
        v
    });
    assert_eq!(v, 0);
}

#[test]
fn jit_rejects_recursive_enum() {
    // Tags are strings: recursive or not, enum lowering is outside the
    // int-only JIT. Loud rejection, never silent wrong code.
    let prog = parse(
        "enum List { Cons(x: i32, xs: List), Nil } fn main() -> i32 { let l = List::Cons(1, List::Nil()) return 1 }",
    );
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported"), "{err}");
}

#[test]
fn fmt_round_trips_recursive_enum() {
    let src = "enum List { Cons(x: i32, xs: List), Nil } fn main() -> i32 { return 0 }";
    let once = klang::fmt::fmt_program(&parse(src));
    assert!(once.contains("Cons(x: i32, xs: List)"), "fmt keeps payload:\n{once}");
    let twice = klang::fmt::fmt_program(&parse(&once));
    assert_eq!(once, twice, "fmt idempotent");
    let prog = parse(&once);
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}

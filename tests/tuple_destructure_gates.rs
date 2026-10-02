//! KLANG-FOUNDATION-3 Step 2 — tuple variants + partial destructuring.
//!
//! Two orthogonal additions to the enum feature:
//! - Tuple-variant declarations: `Variant(i32, str)` bare-type payloads
//!   (fields synthesize `f0..fN` names matching the positional storage;
//!   mixing named and bare fields in one variant is `E-PARSE`).
//!   Construction was always positional, so nothing changes there.
//! - Partial destructuring in patterns: each binding position is `_`
//!   (ignore, binds nothing), `name` (bind), or a nested
//!   `Enum::Variant(sub...)` pattern (tag check with fallthrough on
//!   mismatch, then recursive destructure). A bare identifier ALWAYS
//!   binds — even one spelling a variant — so nested patterns must be
//!   qualified. Count mismatch is still `E-ARITY` (no silent prefix
//!   binding); a nested pattern on a statically non-matching field type
//!   is `E-TYPE`; nested arms never satisfy exhaustiveness alone
//!   (conservative, like guards).
//!
//! Backend coverage note: the v2 language has no `match` at all, so "v2
//! backend" coverage is the documented exclusion plus a positive
//! control. The int-only JIT loudly rejects every enum `match`,
//! destructuring included.
//!
//! BUG-A discipline: the silent failures here are binding the WRONG
//! field (order confusion), a failed nested match taking the wrong arm,
//! and an ignored field leaking into scope — all pinned by exact values
//! and exact diagnostics below.

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
fn tuple_variant_decl_and_construct() {
    // Basic case: bare-type payloads declare, construct positionally,
    // and read back exactly.
    let (v, _) = run_src(
        "enum R { Rect(i32, i32) } fn main() -> i32 { let r = R::Rect(20, 22) return match r { R::Rect(w, h) => w + h } }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn named_payload_partial_ignore() {
    // The PRD's example: existing named syntax with `_` for the rest.
    let (v, _) = run_src(
        "enum Shape { Rect(w: i32, h: i32), Point } fn main() -> i32 { let r = Shape::Rect(40, 2) return match r { Shape::Rect(w, _) => w, Shape::Point => 0 } }",
        "main",
    );
    assert_eq!(v, 40);
}

#[test]
fn partial_ignore_binds_first_position() {
    // Order matters: position 0 of 3, not "the named one" (tuple fields
    // have no names to confuse).
    let (v, _) = run_src(
        "enum R { Rect(i32, i32, i32) } fn main() -> i32 { let r = R::Rect(10, 20, 30) return match r { R::Rect(w, _, _) => w } }",
        "main",
    );
    assert_eq!(v, 10);
}

#[test]
fn partial_ignore_binds_last_position() {
    // A silent order swap would answer 10 here instead of 30.
    let (v, _) = run_src(
        "enum R { Rect(i32, i32, i32) } fn main() -> i32 { let r = R::Rect(10, 20, 30) return match r { R::Rect(_, _, h) => h } }",
        "main",
    );
    assert_eq!(v, 30);
}

#[test]
fn underscore_binds_nothing() {
    // An ignored field must not leak into scope: reading `_` as a value
    // is `E-UNDEFINED` (previously `_` bound a variable of that name).
    check_err_code(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(1) return match v { Opt::Some(_) => _, Opt::None => 0 } }",
        "E-UNDEFINED",
    );
    // ...while the arm itself still runs and yields its body.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(1) return match v { Opt::Some(_) => 7, Opt::None => 0 } }",
        "main",
    );
    assert_eq!(v, 7);
}

#[test]
fn arity_still_enforced() {
    // Partial means `_` placeholders, not elision: count mismatch is
    // still loud `E-ARITY` both ways.
    check_err_code(
        "enum R { Rect(i32, i32) } fn main() -> i32 { let r = R::Rect(1, 2) return match r { R::Rect(w) => w } }",
        "E-ARITY",
    );
    check_err_code(
        "enum R { Rect(i32, i32) } fn main() -> i32 { let r = R::Rect(1, 2) return match r { R::Rect(a, b, c) => a } }",
        "E-ARITY",
    );
}

#[test]
fn mixed_decl_rejected() {
    // Named and bare fields in one variant would bind ambiguously:
    // loud `E-PARSE`, never a silent choice.
    parse_err_code(
        "enum R { Rect(w: i32, i32) } fn main() -> i32 { return 0 }",
        "E-PARSE",
    );
    parse_err_code(
        "enum R { Rect(i32, h: i32) } fn main() -> i32 { return 0 }",
        "E-PARSE",
    );
}

#[test]
fn nested_two_level_partial() {
    // A variant containing another variant, partially destructured at
    // both levels: outer binds nothing but the nested head, inner binds
    // one of one... here inner binds its only field, outer ignores right.
    let (v, _) = run_src(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn main() -> i32 { let t = T::Node(T::Leaf(40), T::Node(T::Leaf(1), T::Leaf(1))) return match t { T::Node(T::Leaf(a), _) => a + 2, _ => 0 } }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn nested_mismatch_falls_through() {
    // A nested tag mismatch is a miss, not an error: matching falls
    // through to the next arm. Taking the first arm would silently
    // answer 100.
    let (v, _) = run_src(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn main() -> i32 { let t = T::Node(T::Leaf(1), T::Leaf(2)) return match t { T::Node(T::Node(a, b), _) => 100, T::Node(T::Leaf(a), _) => a + 10, _ => 0 } }",
        "main",
    );
    assert_eq!(v, 11);
}

#[test]
fn nested_on_wrong_static_type_is_etype() {
    // A field statically known to be `i32` can never match a nested
    // variant pattern: loud `E-TYPE`, mirroring cross-enum arms.
    check_err_code(
        "enum T { Node(n: i32, r: T), Leaf(v: i32) } fn main() -> i32 { let t = T::Leaf(1) return match t { T::Node(T::Leaf(a), _) => a, _ => 0 } }",
        "E-TYPE",
    );
}

#[test]
fn nested_unknown_variant_is_undefined() {
    check_err_code(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn main() -> i32 { let t = T::Leaf(1) return match t { T::Node(T::Bogus(a), _) => a, _ => 0 } }",
        "E-UNDEFINED",
    );
}

#[test]
fn nested_exhaustiveness_conservative() {
    // A nested arm may miss (tag mismatch), so it never covers its
    // variant alone: without a fallback this is `E-MATCH-EXHAUSTIVE`.
    check_err_code(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn f(t: T) -> i32 { return match t { T::Node(T::Leaf(a), _) => a, T::Leaf(v) => v } }",
        "E-MATCH-EXHAUSTIVE",
    );
    // ...with a wildcard it checks and runs.
    let (v, _) = run_src(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn f(t: T) -> i32 { return match t { T::Node(T::Leaf(a), _) => a, _ => 99 } } fn main() -> i32 { return f(T::Node(T::Leaf(1), T::Leaf(2))) * 100 + f(T::Leaf(5)) }",
        "main",
    );
    assert_eq!(v, 199, "1*100+99");
}

#[test]
fn nested_duplicate_rules() {
    // Plain-then-nested same shape: the nested arm is dead (`E-MATCH-DUPLICATE`).
    check_err_code(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn f(t: T) -> i32 { return match t { T::Node(a, b) => 1, T::Node(T::Leaf(x), _) => 2, T::Leaf(v) => 3 } }",
        "E-MATCH-DUPLICATE",
    );
    // Nested-then-plain: reachable (the nested arm may miss), no diagnostic.
    // First call takes the nested arm (2), second misses it and takes
    // the plain arm (1): a wrong fallthrough would answer 22 or 11.
    let (v, _) = run_src(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn f(t: T) -> i32 { return match t { T::Node(T::Leaf(x), _) => 2, T::Node(a, b) => 1, T::Leaf(v) => 3 } } fn main() -> i32 { return f(T::Node(T::Leaf(0), T::Leaf(0))) * 10 + f(T::Node(T::Node(T::Leaf(0), T::Leaf(0)), T::Leaf(0))) }",
        "main",
    );
    assert_eq!(v, 21, "2*10+1");
}

#[test]
fn nested_shadow_restored_on_fallthrough() {
    // BUG-A class: a nested binding shadowing an outer variable must be
    // restored when matching falls through (guard-false here). Without
    // nested names in the save/restore set, `_ => a` reads 1, not 100.
    let (v, _) = run_src(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn f(t: T) -> i32 { let a = 100 return match t { T::Node(T::Leaf(a), _) if a > 10 => a, _ => a } } fn main() -> i32 { return f(T::Node(T::Leaf(1), T::Leaf(2))) * 1000 + f(T::Node(T::Leaf(40), T::Leaf(2))) }",
        "main",
    );
    assert_eq!(v, 100040, "100*1000+40");
}

#[test]
fn nested_bindings_captured_by_closure() {
    // A closure created in an arm captures nested-bound names.
    let (v, _) = run_src(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn main() -> i32 { let t = T::Node(T::Leaf(40), T::Leaf(1)) return match t { T::Node(T::Leaf(a), _) => { let f = fn() -> i32 { return a + 2 } f() }, _ => 0 } }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn guard_sees_nested_binding() {
    // Guards observe nested-bound names (setup runs first, then the
    // guard — the FOUNDATION-2 order — all after the full pattern).
    let (v, _) = run_src(
        "enum T { Node(l: T, r: T), Leaf(v: i32) } fn main() -> i32 { let t = T::Node(T::Leaf(20), T::Leaf(1)) return match t { T::Node(T::Leaf(a), _) if a > 10 => 1, _ => 0 } }",
        "main",
    );
    assert_eq!(v, 1);
}

#[test]
fn bare_ident_always_binds() {
    // A bare identifier in binding position always binds — even one that
    // spells a variant — so nested patterns must be qualified. Here
    // `Nil` binds the tail (a `Nil` value), and `x` is 5.
    let (v, _) = run_src(
        "enum L { Cons(x: i32, xs: L), Nil } fn main() -> i32 { let l = L::Cons(5, L::Nil()) return match l { L::Cons(x, Nil) => x, L::Nil => 0 } }",
        "main",
    );
    assert_eq!(v, 5);
}

#[test]
fn module_nested_pattern() {
    // Qualified nested patterns resolve through modules like top-level ones.
    let (v, _) = run_src(
        "mod m { pub enum E { Pair(x: i32, y: E), Nil } } fn main() -> i32 { let v = m::E::Pair(20, m::E::Nil()) return match v { m::E::Pair(a, m::E::Nil) => a + 22, _ => 0 } }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn tuple_fmt_round_trip() {
    // Bare payloads print bare and stay stable; patterns print back.
    let src = "enum R { Rect(i32, i32, i32) } fn main() -> i32 { let r = R::Rect(1, 2, 3) return match r { R::Rect(w, _, _) => w } }";
    let once = klang::fmt::fmt_program(&parse(src));
    assert!(once.contains("Rect(i32, i32, i32)"), "fmt keeps tuple decl:\n{once}");
    assert!(once.contains("Rect(w, _, _)"), "fmt keeps pattern:\n{once}");
    let twice = klang::fmt::fmt_program(&parse(&once));
    assert_eq!(once, twice, "fmt idempotent");
    let prog = parse(&once);
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 1);
}

#[test]
fn v2_has_no_destructuring() {
    // The v2 grammar has no `match` at all, hence no destructuring
    // either (documented exclusion, unchanged).
    let src = "fn main() -> i32 {\n  return match x { _ => 1 }\n}\n";
    assert!(
        klang::parser::v2::parse_v2_program(src).is_err(),
        "v2 must not accept match"
    );
}

#[test]
fn v2_if_blocks_still_run() {
    // Positive v2 control: v2 conditional dispatch is unaffected.
    let src = "fn f(n: i32) -> i32 {\n  if n == 0 {\n    5\n  } else {\n    7\n  }\n}\nfn main() -> i32 {\n  return f(0) * 10 + f(1)\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    let (v, _) = klang::runtime::v2::run_v2_program(&prog, "main").expect("runs");
    assert_eq!(v, 57, "5*10+7");
}

#[test]
fn jit_rejects_destructuring_match() {
    // Destructuring lowers to the same enum-dispatch MIR (tags are
    // strings) the int-only JIT never accepts: loud rejection, never
    // silent wrong code.
    let prog = parse(
        "enum R { Rect(i32, i32) } fn main() -> i32 { let r = R::Rect(1, 2) return match r { R::Rect(w, _) => w } }",
    );
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported"), "{err}");
}

//! Phase 2 Session 2 — HIR / type-checker adversarial gates.
//!
//! Hand-constructed programs designed to break the checker's assumptions,
//! in the style of `robustness_gates.rs` and the F1–F3 regressions: every
//! fixture is tried against the real checker, and every finding is pinned
//! here. Findings F9 (missing struct fields) and F10 (struct-signature
//! erasure) were found by this session; the remaining tests pin correct
//! behavior the session verified by probing (per-site generic
//! substitution, functions-as-values, effect propagation through
//! match/nesting, qualified generics — F15 closed the gap, so
//! `module::Type<T>` now parses; see
//! `qualified_generic_annotation_syntax_is_parse_error`) plus the F7
//! diagnostic-span inventory.

use std::collections::HashMap;

use klang::parser::Parser;

fn check_err_code(src: &str, code: &str) -> Vec<klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(
        err.iter().any(|d| d.code == code),
        "want {code}, got {:?}",
        err.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    err
}

fn check_clean(src: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog).is_ok(),
        "must check clean: {src}"
    );
}

fn run_src(src: &str, entry: &str) -> (i32, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("runs")
}

// ---------------------------------------------------------------------
// F9 — struct literals must provide every declared field.
// ---------------------------------------------------------------------

#[test]
fn struct_literal_missing_field_is_diagnostic() {
    // Regression (F9): `P { x: 1 }` for a two-field `P` used to check
    // clean and fail only at runtime (`E-RUNTIME unknown struct field`).
    let err = check_err_code(
        "struct P { x: i32, y: i32 } fn main() -> i32 { let p = P { x: 1 } return p.x }",
        "E-ARITY",
    );
    let d = err.iter().find(|d| d.code == "E-ARITY").unwrap();
    assert!(d.message.contains("`P`"), "{}", d.to_json());
    assert!(d.message.contains("y"), "names the missing field: {}", d.to_json());
}

#[test]
fn struct_literal_missing_all_fields_lists_each() {
    let err = check_err_code(
        "struct P { x: i32, y: i32 } fn main() -> i32 { let p = P { } return 0 }",
        "E-ARITY",
    );
    let d = err.iter().find(|d| d.code == "E-ARITY").unwrap();
    assert!(d.message.contains("x") && d.message.contains("y"), "{}", d.to_json());
}

#[test]
fn struct_literal_missing_field_in_module() {
    // Qualified construction goes through the same check.
    let err = check_err_code(
        "mod lexer { pub struct Token { kind: i32, extra: i32 } } fn main() -> i32 { let t = lexer::Token { kind: 40 } return t.kind }",
        "E-ARITY",
    );
    let d = err.iter().find(|d| d.code == "E-ARITY").unwrap();
    assert!(d.message.contains("extra"), "{}", d.to_json());
}

#[test]
fn struct_literal_missing_and_extra_reports_both() {
    // Unknown fields were always E-UNDEFINED; missing fields are now
    // E-ARITY. A literal with both reports both, neither masking the other.
    let mut p = Parser::new(
        "struct P { x: i32, y: i32 } fn main() -> i32 { let p = P { x: 1, z: 2 } return p.x }",
    );
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    let codes: Vec<&str> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&"E-ARITY"), "missing y, got {codes:?}");
    assert!(codes.contains(&"E-UNDEFINED"), "extra z, got {codes:?}");
}

#[test]
fn struct_literal_full_still_runs() {
    // The fix must not reject complete literals.
    let (v, _) = run_src(
        "struct P { x: i32, y: i32 } fn main() -> i32 { let p = P { x: 20, y: 22 } return p.x + p.y }",
        "main",
    );
    assert_eq!(v, 42);
}

// ---------------------------------------------------------------------
// F10 — struct/enum annotations are nominal in signatures, not Unknown.
// ---------------------------------------------------------------------

#[test]
fn struct_param_wrong_type_is_diagnostic() {
    // Regression (F10): struct-typed parameters erased to `Unknown` in
    // call signatures, so `f(99)` checked clean and failed only at
    // runtime. Enum parameters were always nominal — structs now match.
    let err = check_err_code(
        "struct T { kind: i32 } fn f(t: T) -> i32 { return t.kind } fn main() -> i32 { return f(99) }",
        "E-TYPE",
    );
    let d = err.iter().find(|d| d.code == "E-TYPE").unwrap();
    assert!(d.message.contains("want T"), "{}", d.to_json());
    assert!(d.message.contains("got i32"), "{}", d.to_json());
}

#[test]
fn struct_param_qualified_checked() {
    // Same hole through a module-qualified annotation: wrong arg fails,
    // right arg still runs.
    check_err_code(
        "mod lexer { pub struct Token { kind: i32 } } fn id(t: lexer::Token) -> i32 { return t.kind } fn main() -> i32 { return id(42) }",
        "E-TYPE",
    );
    let (v, _) = run_src(
        "mod lexer { pub struct Token { kind: i32 } } fn id(t: lexer::Token) -> i32 { return t.kind } fn main() -> i32 { let t = lexer::Token { kind: 40 } return id(t) + 2 }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn struct_return_mismatch_is_diagnostic() {
    check_err_code(
        "struct Token { kind: i32 } fn f() -> Token { return 42 } fn main() -> i32 { return 0 }",
        "E-TYPE",
    );
    // And a correct struct return stays clean.
    check_clean(
        "struct Token { kind: i32 } fn f() -> Token { return Token { kind: 1 } } fn main() -> i32 { return f().kind }",
    );
}

#[test]
fn struct_param_different_struct_is_diagnostic() {
    check_err_code(
        "struct A { x: i32 } struct B { y: i32 } fn f(a: A) -> i32 { return a.x } fn main() -> i32 { let b = B { y: 1 } return f(b) }",
        "E-TYPE",
    );
}

#[test]
fn nested_struct_literal_and_field_access_checked() {
    // A struct-typed field used to erase to `Unknown`: wrong nested
    // literals and field typos through it both checked clean.
    check_err_code(
        "struct Inner { x: i32 } struct Outer { inner: Inner } fn main() -> i32 { let o = Outer { inner: 42 } return 0 }",
        "E-TYPE",
    );
    check_err_code(
        "struct Inner { x: i32 } struct Outer { inner: Inner } fn main() -> i32 { let o = Outer { inner: Inner { x: 1 } } return o.inner.y }",
        "E-UNDEFINED",
    );
    // Correct nesting still checks and runs.
    let (v, _) = run_src(
        "struct Inner { x: i32 } struct Outer { inner: Inner } fn main() -> i32 { let o = Outer { inner: Inner { x: 41 } } return o.inner.x + 1 }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn enum_typed_field_checked() {
    // Non-generic struct fields used bare `parse_ty`, erasing even
    // enum-typed fields: wrong literals passed and `match` on the field
    // skipped exhaustiveness. Both now diagnose.
    check_err_code(
        "enum O { A, B } struct S { v: O } fn main() -> i32 { let s = S { v: 42 } return 0 }",
        "E-TYPE",
    );
    check_err_code(
        "enum O { A, B } struct S { v: O } fn f(s: S) -> i32 { return match s.v { O::A => 1 } } fn main() -> i32 { return 0 }",
        "E-MATCH-EXHAUSTIVE",
    );
    // Exhaustive match through the field is clean and runs.
    let (v, _) = run_src(
        "enum O { A, B } struct S { v: O } fn f(s: S) -> i32 { return match s.v { O::A => 1, O::B => 2 } } fn main() -> i32 { let s = S { v: O::B() } return f(s) + 40 }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn struct_param_dynamic_arg_stays_lenient() {
    // `Unknown` (map lookup, dynamic index) is still a wildcard by design
    // (SPEC §2): making structs nominal must not turn dynamic producers
    // into false positives.
    check_clean(
        "struct Token { kind: i32 } fn id(t: Token) -> i32 { return 0 } fn main() -> i32 { let m = {\"a\": 1} return id(m[\"a\"]) }",
    );
}

#[test]
fn generic_struct_param_checked() {
    check_err_code(
        "struct Box<T> { value: T } fn f(b: Box) -> i32 { return 0 } fn main() -> i32 { return f(42) }",
        "E-TYPE",
    );
    let (v, _) = run_src(
        "struct Box<T> { value: T } fn f(b: Box) -> i32 { return 0 } fn main() -> i32 { let b = Box { value: 41 } return f(b) + 42 }",
        "main",
    );
    assert_eq!(v, 42);
}

// ---------------------------------------------------------------------
// Session 2 pins: probed adversarially, behavior verified correct.
// ---------------------------------------------------------------------

#[test]
fn generic_per_site_substitution_stays_independent() {
    // Two call sites in one function instantiate independently: the int
    // site must not constrain the str site (subst is fresh per call).
    let (v, out) = run_src(
        "fn same<T>(a: T, b: T) -> T { return a } fn main() -> i32 { let x = same(20, 22) let y = same(\"a\", \"b\") print(y) return x + 22 }",
        "main",
    );
    assert_eq!(v, 42);
    assert_eq!(out, vec!["a".to_string()]);
    // And a conflicting single site still fails.
    check_err_code(
        "fn same<T>(a: T, b: T) -> T { return a } fn main() -> i32 { return same(1, \"hi\") }",
        "E-TYPE",
    );
}

#[test]
fn generic_fn_as_value_is_rejected() {
    // Functions are not values: passing a generic function itself as an
    // argument is E-UNDEFINED, not a crash and not silent acceptance.
    check_err_code(
        "fn id<T>(x: T) -> T { return x } fn main() -> i32 { return id(id) }",
        "E-UNDEFINED",
    );
}

#[test]
fn throws_propagates_through_match_and_nesting() {
    // A `throws` call inside a match arm, and through for/if nesting, is
    // seen: missing `throws` on the caller is E-EFFECT-MISMATCH, and the
    // annotated versions check clean.
    check_err_code(
        "enum Opt { Some(x: i32), None } fn risky(x: i32) -> i32 throws { return x } fn pick(v: Opt) -> i32 { return match v { Opt::Some(n) => risky(n), Opt::None => 0 } } fn main() -> i32 { return 0 }",
        "E-EFFECT-MISMATCH",
    );
    check_err_code(
        "fn risky(x: i32) -> i32 throws { return x } fn deep(n: i32) -> i32 { for i in 0..n { if i == 1 { return risky(i) } } return 0 } fn main() -> i32 { return 0 }",
        "E-EFFECT-MISMATCH",
    );
    check_clean(
        "enum Opt { Some(x: i32), None } fn risky(x: i32) -> i32 throws { return x } fn pick(v: Opt) -> i32 throws { return match v { Opt::Some(n) => risky(n), Opt::None => 0 } } fn main() -> i32 throws { return pick(Opt::Some(1)) }",
    );
    check_clean(
        "fn risky(x: i32) -> i32 throws { return x } fn deep(n: i32) -> i32 throws { for i in 0..n { if i == 1 { return risky(i) } } return 0 } fn main() -> i32 throws { return deep(3) }",
    );
}

#[test]
fn qualified_generic_annotation_syntax_is_parse_error() {
    // F15 closed this gap: `module::Type<T>` annotations now parse
    // (base `a::Box` + `<i32>` suffix, resolved nominally like bare
    // `Opt<i32>`). Previously this was pinned E-PARSE as the documented
    // inference-only limitation; now it must parse and check clean.
    let mut p = Parser::new(
        "mod a { pub struct Box<T> { value: T } } fn get(b: a::Box<i32>) -> i32 { return 1 } fn main() -> i32 { return 0 }",
    );
    let prog = p.parse_program().expect("qualified generics must parse since F15");
    assert!(
        klang::hir::TypedHIR::check(prog).is_ok(),
        "qualified generic annotation must check clean"
    );
}

#[test]
fn diagnostic_span_inventory() {
    // F7 inventory, pinned: which codes carry real spans vs `0,0`.
    // Real spans (must stay real — scope attribution depends on them):
    // E-EFFECT-MISMATCH carries the caller's name span.
    let err = check_err_code(
        "fn risky() -> i32 throws { return 1 } fn main() -> i32 { return risky() }",
        "E-EFFECT-MISMATCH",
    );
    let d = err.iter().find(|d| d.code == "E-EFFECT-MISMATCH").unwrap();
    assert_ne!((d.primary_span.start, d.primary_span.end), (0, 0), "{}", d.to_json());
    // E-TASK-CANCEL carries the `let` binding span.
    let err = check_err_code(
        "fn g() -> i32 { return 1 } fn f() -> i32 async { task_group { let a = spawn g() return 1 } } fn main() -> i32 { return 0 }",
        "E-TASK-CANCEL",
    );
    let d = err.iter().find(|d| d.code == "E-TASK-CANCEL").unwrap();
    assert_ne!((d.primary_span.start, d.primary_span.end), (0, 0), "{}", d.to_json());
    // E-SPAWN-OUTSIDE-GROUP carries the binding span.
    let err = check_err_code(
        "fn g() -> i32 { return 1 } fn main() -> i32 { let a = spawn g() return 0 }",
        "E-SPAWN-OUTSIDE-GROUP",
    );
    let d = err.iter().find(|d| d.code == "E-SPAWN-OUTSIDE-GROUP").unwrap();
    assert_ne!((d.primary_span.start, d.primary_span.end), (0, 0), "{}", d.to_json());
    // Known `0,0` debt (F7): HIR type/name/match/arity diagnostics carry
    // no span. Pinned so any future span threading shows up here as a
    // deliberate diff, not silent drift.
    for (src, code) in [
        ("fn main() -> i32 { return \"hi\" - 1 }", "E-TYPE"),
        ("fn main() -> i32 { return nope + 1 }", "E-UNDEFINED"),
        ("fn f(a: i32) -> i32 { return a } fn main() -> i32 { return f(1, 2) }", "E-ARITY"),
        (
            "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) => n } } fn main() -> i32 { return 0 }",
            "E-MATCH-EXHAUSTIVE",
        ),
        ("fn main() -> i32 { return 1 } fn main() -> i32 { return 2 }", "E-DUPLICATE"),
        ("fn main() -> i32 { break return 0 }", "E-LOOP"),
    ] {
        let err = check_err_code(src, code);
        let d = err.iter().find(|d| d.code == code).unwrap();
        assert_eq!(
            (d.primary_span.start, d.primary_span.end),
            (0, 0),
            "{code} unexpectedly gained a span: {}",
            d.to_json()
        );
    }
}

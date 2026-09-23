use std::collections::HashMap;

use klang::parser::Parser;

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

#[test]
fn enum_data_payload_destructures() {
    // Variant carrying data must bind its payload, not return a default.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::Some(41) return match v { Opt::Some(n) => n + 1, Opt::None => 0 } }",
        "main",
    );
    assert_eq!(v, 42);
    // The other variant must take the other arm (proves dispatch, not fallthrough).
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::None() return match v { Opt::Some(n) => n + 1, Opt::None => 7 } }",
        "main",
    );
    assert_eq!(v, 7);
}

#[test]
fn enum_wildcard_covers_missing_variants() {
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } fn pick(v: Opt) -> i32 { return match v { Opt::Some(n) => n, _ => 99 } } fn main() -> i32 { let a = pick(Opt::None()) let b = pick(Opt::Some(1)) return a + b }",
        "main",
    );
    assert_eq!(v, 100);
}

#[test]
fn enum_nonexhaustive_match_is_diagnostic() {
    // Must be a compile-time Err with a real error code, not a panic and
    // not a silently-accepted program.
    let mut p = Parser::new(
        "enum Opt { Some(x: i32), None } fn main() -> i32 { let v = Opt::None() return match v { Opt::Some(n) => n } }",
    );
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("non-exhaustive match must fail");
    assert!(
        err.iter().any(|d| d.code == "E-MATCH-EXHAUSTIVE"),
        "want E-MATCH-EXHAUSTIVE, got {:?}",
        err.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    // The diagnostic must name the missing variant and render as JSON.
    let diag = err
        .iter()
        .find(|d| d.code == "E-MATCH-EXHAUSTIVE")
        .expect("diagnostic present");
    assert!(diag.message.contains("Opt::None"), "{}", diag.to_json());
    assert_eq!(diag.severity, "error");
}

// ---------------------------------------------------------------------
// PRD §6 gap coverage: the pre-existing gates above prove dispatch +
// wildcard + one exhaustiveness case. The cases below close the rest of
// the PRD's testing requirements using the REAL implemented API:
// tuple-style `Variant(x: i32)` payloads, `E::V(bindings)` patterns,
// `E-MATCH-EXHAUSTIVE` (not the PRD-draft name), `E-MATCH-UNREACHABLE` /
// `E-MATCH-DUPLICATE`, binding-arity errors, non-enum scrutinees, and a
// repair-loop end-to-end (mock backend, mirrors repair_gates.rs).
// ---------------------------------------------------------------------

fn check_errs(src: &str) -> Vec<klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    klang::hir::TypedHIR::check(prog).expect_err("must fail")
}

fn codes(ds: &[klang::diagnostics::Diagnostic]) -> Vec<String> {
    ds.iter().map(|d| d.code.clone()).collect()
}

#[test]
fn enum_exhaustiveness_all_variants_covered_is_clean() {
    let mut p = Parser::new(
        "enum Shape { Circle(r: i32), Rect(w: i32), Point } fn area(s: Shape) -> i32 { return match s { Shape::Circle(r) => r * 3, Shape::Rect(w) => w * 2, Shape::Point => 0 } } fn main() -> i32 { return area(Shape::Point()) }",
    );
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok(), "all variants covered -> clean");
}

#[test]
fn enum_exhaustiveness_missing_variants_listed() {
    // Missing one of three: message names exactly the missing variant.
    let err = check_errs(
        "enum Shape { Circle(r: i32), Rect(w: i32), Point } fn area(s: Shape) -> i32 { return match s { Shape::Circle(r) => r, Shape::Rect(w) => w } } fn main() -> i32 { return area(Shape::Point()) }",
    );
    let d = err.iter().find(|d| d.code == "E-MATCH-EXHAUSTIVE").expect("exhaustive diag");
    assert!(d.message.contains("Shape::Point"), "{}", d.to_json());
    assert!(!d.message.contains("Shape::Circle"), "covered variant must not be listed: {}", d.message);
    // Missing all but covered none: every variant named.
    let err = check_errs(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { _ => 0, Opt::Some(n) => n } }",
    );
    assert!(err.iter().any(|d| d.code == "E-MATCH-UNREACHABLE"), "arm after wildcard unreachable, got {:?}", codes(&err));
}

#[test]
fn enum_missing_all_variants_names_everything() {
    // Parseable but covering nothing real: wildcard-free match with only
    // an unknown arm still reports every missing variant (plus E-UNDEFINED
    // for the bogus arm -- both diagnostics are expected).
    let err = check_errs(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Bogus => 1 } }",
    );
    let d = err.iter().find(|d| d.code == "E-MATCH-EXHAUSTIVE").expect("exhaustive diag");
    assert!(d.message.contains("Opt::Some") && d.message.contains("Opt::None"), "{}", d.to_json());
    // Fixes suggest arms or a wildcard; JSON renders.
    assert!(d.to_json().contains("wildcard"), "{}", d.to_json());
    assert_eq!(d.severity, "error");
}

#[test]
fn enum_wildcard_alone_suppresses_exhaustiveness() {
    let mut p = Parser::new(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { _ => 7 } } fn main() -> i32 { return f(Opt::None()) }",
    );
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok(), "bare wildcard covers everything");
}

#[test]
fn enum_duplicate_variant_arm_is_diagnostic() {
    let err = check_errs(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) => n, Opt::Some(m) => m, Opt::None => 0 } }",
    );
    assert!(err.iter().any(|d| d.code == "E-MATCH-DUPLICATE"), "duplicate arm, got {:?}", codes(&err));
    let d = err.iter().find(|d| d.code == "E-MATCH-DUPLICATE").unwrap();
    assert!(d.message.contains("Opt::Some"), "{}", d.to_json());
}

#[test]
fn enum_arm_after_wildcard_is_unreachable() {
    let err = check_errs(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) => n, _ => 0, Opt::None => 1 } }",
    );
    assert!(err.iter().any(|d| d.code == "E-MATCH-UNREACHABLE"), "got {:?}", codes(&err));
}

#[test]
fn enum_binding_arity_mismatch_is_diagnostic() {
    // Too few bindings for the payload.
    let err = check_errs(
        "enum Pair { Both(a: i32, b: i32), One(x: i32) } fn f(v: Pair) -> i32 { return match v { Pair::Both(a) => a, Pair::One(x) => x } }",
    );
    assert!(err.iter().any(|d| d.code == "E-ARITY"), "binding arity, got {:?}", codes(&err));
    // Too many bindings.
    let err = check_errs(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(a, b) => a, Opt::None => 0 } }",
    );
    assert!(err.iter().any(|d| d.code == "E-ARITY"), "got {:?}", codes(&err));
}

#[test]
fn enum_binding_types_flow_into_arm_body() {
    // `n` binds i32: arithmetic on it is clean; returning it as str is E-TYPE.
    let mut p = Parser::new(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) => n + 1, Opt::None => 0 } } fn main() -> i32 { return f(Opt::Some(1)) }",
    );
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
    let err = check_errs(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> str { return match v { Opt::Some(n) => n, Opt::None => \"z\" } }",
    );
    assert!(err.iter().any(|d| d.code == "E-TYPE"), "i32 binding as str arm, got {:?}", codes(&err));
}

#[test]
fn enum_match_on_non_enum_is_type_diagnostic() {
    // Non-enum scrutinee is E-TYPE on the scrutinee (not a parse error,
    // points at the expression with its actual type).
    let err = check_errs("fn f(x: i32) -> i32 { return match x { _ => 1 } }");
    let d = err.iter().find(|d| d.code == "E-TYPE").expect("scrutinee type diag");
    assert!(d.message.contains("match scrutinee"), "{}", d.to_json());
    assert!(d.message.contains("i32"), "actual type named: {}", d.to_json());
}

#[test]
fn enum_unknown_variant_arm_is_undefined() {
    let err = check_errs(
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Bogus => 1, Opt::Some(n) => n, Opt::None => 0 } }",
    );
    assert!(err.iter().any(|d| d.code == "E-UNDEFINED"), "got {:?}", codes(&err));
}

#[test]
fn enum_cross_enum_arm_is_type_diagnostic() {
    // Arm from a different enum than the scrutinee.
    let err = check_errs(
        "enum A { X } enum B { Y } fn f(v: A) -> i32 { return match v { B::Y => 1, A::X => 2 } }",
    );
    assert!(err.iter().any(|d| d.code == "E-TYPE"), "got {:?}", codes(&err));
}

#[test]
fn enum_nonexhaustive_repair_end_to_end() {
    // klang repair compatibility (PRD §5.4): structured diagnostics need
    // no repair-module changes -- mock backend returns the missing arm,
    // spliced function-scope repair re-checks clean.
    use klang::repair::{MockBackend, RepairCliOverrides, RepairConfig, RepairFileConfig, Scope, check_candidate, plan_scope, run_repair};
    use klang::repair::scope::RepairScope;
    let src = "enum Opt { Some(x: i32), None }\n\nfn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n }\n}\n";
    let diags = check_candidate(src).diagnostics;
    assert!(diags.iter().any(|d| d.code == "E-MATCH-EXHAUSTIVE"), "got {:?}", codes(&diags));
    let missing = diags.iter().find(|d| d.code == "E-MATCH-EXHAUSTIVE").unwrap();
    assert!(missing.message.contains("Opt::None"), "{}", missing.to_json());
    assert_eq!(plan_scope(&diags, src, false), RepairScope::Functions(vec!["pick".into()]));
    let cfg = RepairConfig::resolve_with(
        &RepairCliOverrides { endpoint: Some("http://mock/v1".into()), scope: Some(Scope::Function), max_iters: Some(2), ..Default::default() },
        &RepairFileConfig::default(),
        &[],
    )
    .expect("cfg");
    // NOTE: the mock returns the WHOLE FILE (enum decl + fixed fn): the
    // splicer treats a response containing every original function name as
    // whole-file output, but a file whose only `fn` is the target would
    // otherwise match the "all original fns" passthrough only by luck.
    // Returning the full file keeps the test honest about what the model
    // must emit (declarations are file-level context, not function scope).
    let backend = MockBackend::new(vec![
        "enum Opt { Some(x: i32), None }\n\nfn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n, Opt::None => 0 }\n}\n".into(),
    ]);
    let outcome = run_repair(src, "t.klang", &cfg, &backend, false);
    assert!(outcome.success, "repair must add the missing arm, got {:?}", outcome.final_diagnostics);
    assert!(check_candidate(&outcome.source).diagnostics.is_empty());
    assert!(outcome.source.contains("Opt::None"), "{}", outcome.source);
}

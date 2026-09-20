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

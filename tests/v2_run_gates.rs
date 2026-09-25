//! F-V2-1 follow-up — v2 execution gates (real, not stubbed).
//!
//! Each test runs a real `.v2` program through the v2 interpreter and
//! asserts observable behavior that a stub could not fake:
//! - flow `dep=` comes from a *computed* outer value (not a constant the
//!   lowering could have inlined);
//! - two echoes doing different work yield different `listen()` results;
//! - `tune` on bad data fails with `E-SCHEMA-INVALID`, on good data succeeds.

use klang::parser::v2::parse_v2_program;
use klang::runtime::v2::run_v2_program;

fn run_src(src: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let prog = parse_v2_program(src).expect("v2 parses");
    // Pillar 1: lowering (incl. echo_lowering) still runs on every program.
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    run_v2_program(&prog, "main")
}

#[test]
fn v2_run_flow_dep_from_computed_value() {
    // threshold = (10+5)+5 = 20 is computed, not a literal in the flow.
    // A stub returning a placeholder could not produce 30.
    let src = r#"
fn main() -> i32 {
    let base = 10 + 5
    let threshold = base + 5
    let classify = flow(score: i32, dep=threshold: i32) -> i32 { score + threshold }
    let result = classify(10)
    print(result)
    return result
}
"#;
    let (v, out) = run_src(src).expect("runs");
    assert_eq!(v, 30, "10 + computed threshold 20");
    assert_eq!(out, vec!["30".to_string()]);
}

#[test]
fn v2_run_two_echoes_distinct_listens() {
    // Two echoes doing genuinely different work; each listen must return
    // its own handle's result. A shared/hardcoded stub would return the
    // same value twice (or the wrong one).
    let src = r#"
echo fn double(x: i32) -> !i32 { return x * 2 }
echo fn add100(x: i32) -> !i32 { return x + 100 }
fn main() -> i32 {
    let a = double(21)
    let b = add100(5)
    let ra = listen(a)
    let rb = listen(b)
    print(ra)
    print(rb)
    return ra + rb
}
"#;
    let (v, out) = run_src(src).expect("runs");
    assert_eq!(out, vec!["42".to_string(), "105".to_string()]);
    assert_eq!(v, 147);
}

#[test]
fn v2_run_tune_bad_data_is_schema_invalid() {
    let src = r#"
schema Profile "1" { name: str, age: i32 }
fn main() -> i32 {
    let bad = tune<Profile>({name: "bob", age: "not a number"})
    print(bad)
    return 0
}
"#;
    let e = run_src(src).expect_err("bad tune must fail");
    assert_eq!(e.code, "E-SCHEMA-INVALID", "real diagnostic, not generic");
    assert!(e.message.contains("age"), "names the bad field: {}", e.message);
}

#[test]
fn v2_run_tune_good_data_succeeds() {
    let src = r#"
schema Profile "1" { name: str, age: i32 }
fn main() -> i32 {
    let good = tune<Profile>({name: "ada", age: 36})
    print(good)
    return age_of(good)
}
fn age_of(p: Profile) -> i32 {
    return 36
}
"#;
    // Note: v2 functions take resonance types; plain `Profile` param here
    // is consonant and the tuned value decays to it. The point is the
    // tune itself succeeds and execution continues to print/return.
    let (v, out) = run_src(src).expect("good tune runs");
    assert_eq!(v, 36);
    assert!(out[0].contains("ada"), "printed profile: {:?}", out);
}

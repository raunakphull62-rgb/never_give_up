//! BUGHUNT-2 (resolves HEAVY-TEST-2) — check-time `tune`/`verify` validation.
//!
//! Decision: `check-v2` validates tune/verify arguments only when they
//! are compile-time constants, with the same codes/messages the runtime
//! emits (shared `SchemaRegistry::check_boundary` — no parallel
//! validator). Dynamic values stay runtime-checked. These tests pin
//! check/run agreement per shape, never just one side.

use klang::parser::v2::parse_v2_program;

const FILE: &str = "/tmp/bh_v2_tune_check.v2";

fn check(src: &str) -> Vec<klang::diagnostics::Diagnostic> {
    klang::mcp::v2_check_source_with_file(src, FILE)
}

fn run(src: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let prog = parse_v2_program(src).expect("v2 parses");
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    klang::runtime::v2::run_v2_program(&prog, "main")
}

#[test]
fn bad_tune_literal_fails_check_like_run() {
    // The HEAVY-TEST-2 repro: previously check-clean, run-loud.
    let src = r#"
schema Profile "1" { name: str, age: i32 }
fn main() -> i32 {
    let bad = tune<Profile>({name: "bob", age: "not a number"})
    print(bad)
    return 0
}
"#;
    let diags = check(src);
    assert_eq!(diags.len(), 1, "one diagnostic: {diags:?}");
    assert_eq!(diags[0].code, "E-SCHEMA-INVALID");
    assert!(
        diags[0].message.contains("age"),
        "names the field: {}",
        diags[0].message
    );
    assert_eq!(diags[0].primary_span.file, FILE, "real file, not placeholder");
    // Same message the runtime produces — one shared validator.
    let err = run(src).expect_err("run rejects too");
    assert_eq!(err.code, "E-SCHEMA-INVALID");
    assert_eq!(diags[0].message, err.message);
}

#[test]
fn good_tune_literal_stays_clean_both_sides() {
    let src = r#"
schema Profile "1" { name: str, age: i32 }
fn main() -> i32 {
    let good = tune<Profile>({name: "ada", age: 36})
    print(good)
    return 0
}
"#;
    assert!(check(src).is_empty());
    let (_, out) = run(src).expect("runs");
    assert!(out[0].contains("ada"), "printed profile: {out:?}");
}

#[test]
fn unknown_schema_is_not_found_at_check() {
    let src = r#"
fn main() -> i32 {
    let bad = tune<Missing>({name: "bob"})
    print(bad)
    return 0
}
"#;
    let diags = check(src);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "E-SCHEMA-NOT-FOUND");
}

#[test]
fn dynamic_tune_value_stays_runtime_checked() {
    // A variable is not a constant: check stays clean by design, and a
    // good runtime value still runs.
    let src = r#"
schema Profile "1" { name: str, age: i32 }
echo fn mk_age(x: i32) -> !i32 { return x }
fn main() -> i32 {
    let h = mk_age(36)
    let grown = listen(h)
    let p = tune<Profile>({name: "ada", age: grown})
    print(p)
    return 0
}
"#;
    assert!(check(src).is_empty(), "dynamic values are not check-time");
    let (_, out) = run(src).expect("good dynamic value runs");
    assert!(out[0].contains("ada"), "{out:?}");
}

#[test]
fn bool_coercion_agrees_both_sides() {
    // `true` and `1` validate for bool fields at check AND run (the
    // interpreter encodes Bool as Int(0/1)); `2` fails identically.
    let ok_src = r#"
schema Flags "1" { name: str, on: bool }
fn main() -> i32 {
    let a = tune<Flags>({name: "x", on: true})
    print(a)
    let b = tune<Flags>({name: "y", on: 1})
    print(b)
    return 0
}
"#;
    assert!(check(ok_src).is_empty());
    run(ok_src).expect("coercions run");
    let bad_src = ok_src.replace("on: 1", "on: 2").replace("on: true", "on: true");
    let diags = check(&bad_src);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "E-SCHEMA-INVALID");
    let err = run(&bad_src).expect_err("run rejects too");
    assert_eq!(diags[0].message, err.message);
}

#[test]
fn verify_and_missing_field_match_run() {
    let verify_bad = r#"
schema Profile "1" { name: str, age: i32 }
fn main() -> i32 {
    let v = verify<Profile>({name: "bob", age: "xx"})
    print(v)
    return 0
}
"#;
    let diags = check(verify_bad);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "E-SCHEMA-INVALID");
    let missing = r#"
schema Profile "1" { name: str, age: i32 }
fn main() -> i32 {
    let m = tune<Profile>({name: "bob"})
    print(m)
    return 0
}
"#;
    let diags = check(missing);
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("missing"), "{}", diags[0].message);
    let err = run(missing).expect_err("run rejects too");
    assert_eq!(diags[0].message, err.message);
}

#[test]
fn unparseable_program_is_not_false_ok() {
    // Full-program check previously returned clean for anything that
    // merely lexed; now the parse diagnostic surfaces.
    let diags = check("hello world (((");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "E-PARSE-V2");
}

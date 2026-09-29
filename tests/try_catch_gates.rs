//! FOUNDATION-1 Phase 4 — `try { } catch e { }` recoverable errors.
//!
//! A runtime error inside the `try` body binds `e` to a
//! `{code, message}` map (mirroring the diagnostic) and runs the
//! handler; execution continues after. Success skips the handler.
//! `E-CANCELLED` is never caught (group cancellation must drain).
//! `break`/`continue` cannot cross a `try` boundary (`E-LOOP`); `return`
//! works normally. The JIT rejects `Try` loudly (int-only backend).

use std::collections::HashMap;

use klang::parser::Parser;

fn parse(src: &str) -> klang::ast::Program {
    let mut p = Parser::new(src);
    p.parse_program().expect("parses")
}

fn run_src(src: &str) -> (i32, Vec<String>) {
    let prog = parse(src);
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs")
}

fn run_err(src: &str) -> klang::diagnostics::Diagnostic {
    let prog = parse(src);
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail")
}

fn check_err_code(src: &str, want: &str) {
    let prog = parse(src);
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&want), "want {want} in {codes:?}: {src}");
}

// ---- core: catch and continue ----

#[test]
fn catch_parse_error_and_continue() {
    let (v, out) = run_src(
        "fn main() -> i32 { let n = 0 try { n = parse_int(\"abc\") } catch e { print(e[\"code\"]) n = 0 } print(n) return n }",
    );
    assert_eq!(out, vec!["E-PARSE-INT".to_string(), "0".to_string()]);
    assert_eq!(v, 0);
}

#[test]
fn success_skips_handler() {
    let (v, out) = run_src(
        "fn main() -> i32 { let n = 0 try { n = parse_int(\"41\") + 1 } catch e { print(\"handler\") n = 0 } print(n) return n }",
    );
    assert_eq!(out, vec!["42".to_string()]);
    assert_eq!(v, 42);
}

#[test]
fn catch_binding_carries_code_and_message() {
    let (v, out) = run_src(
        "fn main() -> i32 { try { parse_int(\"abc\") } catch e { print(e[\"code\"]) print(e[\"message\"]) } return 0 }",
    );
    assert_eq!(v, 0);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], "E-PARSE-INT".to_string());
    assert!(out[1].contains("abc"), "message names input: {}", out[1]);
}

#[test]
fn catch_binding_is_scoped_to_handler() {
    check_err_code(
        "fn main() -> i32 { try { parse_int(\"abc\") } catch e { print(e) } print(e) return 0 }",
        "E-UNDEFINED",
    );
}

#[test]
fn example_file_runs() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/try_catch.klang"
    ))
    .expect("example must exist");
    let prog = parse(&src);
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (code, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    // No stdin at library level: read_line sees EOF (""), parse fails,
    // handler runs — the caught-error path, deterministically.
    assert_eq!(code, 0);
    assert_eq!(
        out,
        vec!["bad input: E-PARSE-INT".to_string(), "0".to_string()]
    );
}

// ---- errors from nested calls are caught too ----

#[test]
fn nested_call_failure_is_caught() {
    let (v, out) = run_src(
        "fn risky(x: i32) -> i32 { return 10 / x } fn main() -> i32 { let r = 0 try { r = risky(0) } catch e { print(e[\"code\"]) r = 7 } print(r) return r }",
    );
    assert_eq!(out, vec!["E-RUNTIME".to_string(), "7".to_string()]);
    assert_eq!(v, 7);
}

#[test]
fn handler_failure_propagates() {
    // An error inside the handler is a fresh failure, not a re-catch.
    let d = run_err(
        "fn main() -> i32 { try { parse_int(\"abc\") } catch e { parse_int(\"also-bad\") } return 0 }",
    );
    assert_eq!(d.code, "E-PARSE-INT", "{}", d.to_json());
}

#[test]
fn uncaught_error_still_fails() {
    // Same failure without `try` aborts the program (control case:
    // `try` is what makes it recoverable).
    let d = run_err("fn main() -> i32 { print(parse_int(\"abc\")) return 0 }");
    assert_eq!(d.code, "E-PARSE-INT", "{}", d.to_json());
}

// ---- control flow across the boundary ----

#[test]
fn return_inside_body_and_handler() {
    let (v, out) = run_src(
        "fn f(x: i32) -> i32 { try { if x == 0 { return 99 } return x } catch e { return 0 } } fn main() -> i32 { print(f(0)) print(f(5)) return 0 }",
    );
    assert_eq!(out, vec!["99".to_string(), "5".to_string()]);
    assert_eq!(v, 0);
    let (v, out) = run_src(
        "fn f(x: i32) -> i32 { try { parse_int(\"abc\") } catch e { return 11 } return 0 } fn main() -> i32 { print(f(0)) return 0 }",
    );
    assert_eq!(out, vec!["11".to_string()]);
    assert_eq!(v, 0);
}

#[test]
fn break_continue_cannot_cross_try_boundary() {
    check_err_code(
        "fn main() -> i32 { for i in 0..3 { try { break } catch e { print(e) } } return 0 }",
        "E-LOOP",
    );
    check_err_code(
        "fn main() -> i32 { for i in 0..3 { try { print(i) } catch e { print(e) continue } } return 0 }",
        "E-LOOP",
    );
    // A loop fully inside the `try` is fine.
    let (v, out) = run_src(
        "fn main() -> i32 { let t = 0 try { for i in 0..3 { if i == 1 { continue } if i == 2 { break } t = t + 1 } } catch e { print(e) } print(t) return t }",
    );
    assert_eq!(out, vec!["1".to_string()]);
    assert_eq!(v, 1);
}

// ---- structured concurrency: cancellation is not catchable ----

#[test]
fn cancelled_task_is_not_caught() {
    // `failer` fails fast; `spinner` loops until the group cancels it.
    // If cancellation were catchable, the output would contain
    // "swallowed-cancel" and the group would succeed. Correct: no such
    // line, and the group's error is the original assert failure.
    let (result, out) = {
        let prog = parse(
            "fn failer() -> i32 { assert(false) return 0 } fn spinner() -> i32 { try { while true { } } catch e { print(\"swallowed-cancel\") return 99 } return 0 } fn main() -> i32 async { task_group { let a = spawn failer() let b = spawn spinner() let x = await a let y = await b return x + y } }",
        );
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
        let mir = klang::mir::lower(&prog);
        let (r, out_partial) = klang::runtime::run_with_output_value_partial(
            &mir,
            "main",
            &[],
            &HashMap::new(),
        );
        (r, out_partial)
    };
    let d = result.expect_err("group must fail");
    assert_eq!(d.code, "E-RUNTIME", "{}", d.to_json());
    assert!(
        !out.iter().any(|l| l.contains("swallowed-cancel")),
        "cancellation escaped as a value: {out:?}"
    );
}

// ---- backend boundaries: JIT loud, v2 excluded, fmt round-trips ----

#[test]
fn jit_rejects_try_builtin() {
    let prog = parse("fn main() -> i32 {\n    try {\n        print(parse_int(\"1\"))\n    } catch e {\n        print(e)\n    }\n    return 0\n}\n");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported"), "{err}");
}

#[test]
fn v2_has_no_try_catch() {
    // The v2 grammar has no `try`: a v2 program using it fails to parse.
    // run-v2 is unaffected by the v1 feature (documented exclusion).
    let src = "fn main() -> i32 {\n  try {\n    print(1)\n  } catch e {\n    print(e)\n  }\n  return 0\n}\n";
    assert!(
        klang::parser::v2::parse_v2_program(src).is_err(),
        "v2 must not accept try/catch"
    );
}

#[test]
fn fmt_round_trips_try_catch() {
    let src = "fn main() -> i32 { let n = 0 try { n = parse_int(\"1\") } catch e { print(e[\"code\"]) } print(n) return n }";
    let once = klang::fmt::fmt_program(&parse(src));
    assert!(once.contains("try {"), "fmt keeps try:\n{once}");
    assert!(once.contains("catch e {"), "fmt keeps catch:\n{once}");
    let twice = klang::fmt::fmt_program(&parse(&once));
    assert_eq!(once, twice, "fmt idempotent");
    // Formatted code runs identically.
    let prog = parse(&once);
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(out, vec!["1".to_string()]);
    assert_eq!(v, 1);
}

#[test]
fn try_catch_with_tasks_needs_self_contained_regions() {
    // Spawns inside a `try` region must be awaited in the same region
    // (loop-body discipline): leaking one out is a check-time error.
    check_err_code(
        "fn f() -> i32 { return 1 } fn main() -> i32 async { task_group { try { let h = spawn f() } catch e { print(e) } } return 0 }",
        "E-TASK-CANCEL",
    );
}

#[test]
fn spawn_before_await_after_with_try_between() {
    // The natural pattern is unaffected: handles owned outside the
    // `try` join outside it; only region-crossing is rejected.
    let (v, out) = run_src(
        "fn one() -> i32 { return 1 } fn main() -> i32 async { task_group { let h = spawn one() try { print(10 / 1) } catch e { print(e[\"code\"]) } print(await h) return 0 } }",
    );
    assert_eq!(out, vec!["10".to_string(), "1".to_string()]);
    assert_eq!(v, 0);
}

#[test]
fn catch_binding_shadows_like_block_let() {
    // Shadowing an outer variable in the handler overwrites it at
    // runtime — exactly like a `let` shadowing inside an `if` body
    // (pre-existing language semantics, not try-specific).
    let (v, out) = run_src(
        "fn main() -> i32 { let e = \"outer\" try { parse_int(\"bad\") } catch e { print(e[\"code\"]) } print(e) return 0 }",
    );
    assert_eq!(out[0], "E-PARSE-INT".to_string());
    assert!(out[1].contains("E-PARSE-INT"), "handler map won: {}", out[1]);
    assert_eq!(v, 0);
}

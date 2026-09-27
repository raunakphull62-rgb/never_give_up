//! STDLIB-TIME Phase 1 — Time gates.
//!
//! Covers `time_sleep(seconds)` / `time_now()` / `time_elapsed(since)`
//! per the PRD: sleeping ~1 second and confirming `elapsed` reports
//! close to 1 second (lower bound 0.8s proves the sleep really
//! happened; upper bound 5.0s is deliberately generous so loaded CI
//! runners don't flake), a zero-duration sleep in both float and int
//! form, and a negative-duration call (`E-TIME-INVALID`). Plus
//! `E-TIME-*` code specificity and builtin arity/type checking.

use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new())
}

#[test]
fn osio_time_sleep_and_elapsed_measures_real_second() {
    let src = "fn main() -> i32 { let t0 = time_now() time_sleep(1.0) let e = time_elapsed(t0) print(e) if e < 0.8 { return 1 } if e > 5.0 { return 2 } return 42 }";
    let (v, out) = run_src(src, "main").expect("runs");
    assert_eq!(v, 42, "elapsed must be within [0.8, 5.0], prints: {out:?}");
    assert_eq!(out.len(), 1);
    let measured: f64 = out[0].trim().parse().expect("printed elapsed parses as f64");
    assert!(
        (0.8..=5.0).contains(&measured),
        "real measured seconds in range, got {measured}"
    );
}

#[test]
fn osio_time_zero_sleep_and_now_positive() {
    // Both float and int zero durations must return immediately with
    // the success marker; now() must be a real positive timestamp.
    let src = "fn main() -> i32 { let a = time_sleep(0.0) let b = time_sleep(0) let t = time_now() print(t) if t < 0.0 { return 1 } if a != 1 { return 2 } if b != 1 { return 3 } return 42 }";
    let (v, out) = run_src(src, "main").expect("runs");
    assert_eq!(v, 42, "zero sleeps + positive now, prints: {out:?}");
    let stamp: f64 = out[0].trim().parse().expect("printed now() parses as f64");
    assert!(stamp > 1_000_000_000.0, "real epoch timestamp, got {stamp}");
}

#[test]
fn osio_time_now_advances() {
    let src = "fn main() -> i32 { let t1 = time_now() let t2 = time_now() if t2 < t1 { return 1 } return 42 }";
    let (v, _) = run_src(src, "main").expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn osio_time_negative_sleep_is_time_invalid() {
    let src = "fn main() -> i32 { time_sleep(-1.0) return 0 }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-TIME-INVALID", "got: {}", err.to_json());
    assert!(
        err.to_json().contains("negative"),
        "specific cause, not generic: {}",
        err.to_json()
    );
}

#[test]
fn osio_time_dynamic_non_number_is_time_invalid() {
    // A string smuggled through a map lookup checks as Unknown (so it
    // passes the static gate) but must fail loudly at runtime — never
    // a silent sleep-0.
    let src = "fn main() -> i32 { let m = {\"s\": \"hi\"} time_sleep(m[\"s\"]) return 0 }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-TIME-INVALID", "got: {}", err.to_json());
    assert!(
        err.to_json().contains("needs a number"),
        "specific cause, not generic: {}",
        err.to_json()
    );
}

#[test]
fn osio_time_error_codes_are_specific() {
    // Rust-level mapping: every bad duration is E-TIME-INVALID, but
    // each failure mode carries its own cause string (never one
    // shared generic message).
    let neg = klang::stdlib::time::sleep(-2.5).expect_err("negative must fail");
    assert_eq!(neg.code, "E-TIME-INVALID");
    assert!(neg.to_json().contains("-2.5"), "real value: {}", neg.to_json());
    let nan = klang::stdlib::time::sleep(f64::NAN).expect_err("NaN must fail");
    assert_eq!(nan.code, "E-TIME-INVALID");
    assert!(nan.to_json().contains("NaN"), "real cause: {}", nan.to_json());
    let inf = klang::stdlib::time::sleep(f64::INFINITY).expect_err("inf must fail");
    assert_eq!(inf.code, "E-TIME-INVALID");
    assert!(inf.to_json().contains("infinite"), "real cause: {}", inf.to_json());
    let huge = klang::stdlib::time::sleep(1e300).expect_err("huge must fail loudly, never panic");
    assert_eq!(huge.code, "E-TIME-INVALID");
    // Distinct causes per mode (never one shared generic string).
    assert_ne!(neg.cause, nan.cause);
    assert_ne!(nan.cause, inf.cause);
    assert_ne!(neg.cause, inf.cause);
    // Zero is fine and fast (guards the success path at unit level).
    klang::stdlib::time::sleep(0.0).expect("zero sleeps Ok");
    // now() is a real positive epoch timestamp.
    assert!(klang::stdlib::time::now() > 1_000_000_000.0);
    // elapsed() of an immediate now() is ~0.
    let t0 = klang::stdlib::time::now();
    assert!(klang::stdlib::time::elapsed(t0) < 1.0);
}

#[test]
fn osio_time_arg_types_checked() {
    let mut p = Parser::new("fn main() -> i32 { time_sleep(\"hi\") return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { time_sleep() return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
    let mut p = Parser::new("fn main() -> i32 { let t = time_now(1) return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
    let mut p = Parser::new("fn main() -> i32 { let e = time_elapsed() return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
    let mut p = Parser::new("fn main() -> i32 { let e = time_elapsed(\"hi\") return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
}

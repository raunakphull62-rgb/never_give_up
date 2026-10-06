//! FLOATMATH-1 — Phase 2 Batch B builtins.
//!
//! Covers `sqrt pow abs min max floor ceil sin cos tan atan2 log exp`
//! (happy paths + `E-TYPE` on wrong types, static and runtime, +
//! `E-RUNTIME` domain errors for `sqrt` of a negative / `log` of
//! non-positive), `random_int` / `random_float` (range + shape, never
//! exact values — OS-seeded), and `sha256` / `hmac_sha256` against
//! NIST/RFC vectors.

use std::collections::HashMap;

use klang::parser::Parser;

fn run(src: &str) -> (Result<klang::runtime::Value, klang::diagnostics::Diagnostic>, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    let (r, out) = klang::runtime::run_with_argv(&mir, "main", &[], &[], &HashMap::new());
    (r, out)
}

fn run_ok(src: &str) -> (klang::runtime::Value, Vec<String>) {
    let (r, out) = run(src);
    (r.expect("must run clean"), out)
}

fn run_err_code(src: &str) -> String {
    run(src).0.expect_err("must fail").code.clone()
}

fn check_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&want), "want {want} in {codes:?}: {src}");
}

fn as_f64(v: &klang::runtime::Value) -> f64 {
    match v {
        klang::runtime::Value::Float(x) => *x,
        klang::runtime::Value::Int(x) => *x as f64,
        other => panic!("not a number: {other:?}"),
    }
}

fn approx(src: &str, want: f64) {
    let (v, _) = run_ok(src);
    let got = as_f64(&v);
    assert!(
        (got - want).abs() < 1e-9,
        "{src}: got {got}, want {want}"
    );
}

// ---- float math happy paths ----

#[test]
fn float_sqrt_pow() {
    approx("fn main() -> f64 { return sqrt(16.0) }", 4.0);
    approx("fn main() -> f64 { return sqrt(9) }", 3.0);
    approx("fn main() -> f64 { return sqrt(2.0) }", std::f64::consts::SQRT_2);
    approx("fn main() -> f64 { return pow(2.0, 10.0) }", 1024.0);
    approx("fn main() -> f64 { return pow(2, 10) }", 1024.0);
    approx("fn main() -> f64 { return pow(9.0, 0.5) }", 3.0);
}

#[test]
fn float_abs_min_max() {
    // Ints stay exact.
    let (v, _) = run_ok("fn main() -> i32 { return abs(0 - 5) }");
    assert_eq!(v, klang::runtime::Value::Int(5));
    let (v, _) = run_ok("fn main() -> i32 { return min(3, 7) }");
    assert_eq!(v, klang::runtime::Value::Int(3));
    let (v, _) = run_ok("fn main() -> i32 { return max(3, 7) }");
    assert_eq!(v, klang::runtime::Value::Int(7));
    // Floats and mixed pairs come back float.
    approx("fn main() -> f64 { return abs(0 - 2.5) }", 2.5);
    approx("fn main() -> f64 { return abs(0.0 - 2.5) }", 2.5);
    approx("fn main() -> f64 { return min(3, 7.5) }", 3.0);
    approx("fn main() -> f64 { return max(3.5, 7) }", 7.0);
    approx("fn main() -> f64 { return min(2.5, 7.5) }", 2.5);
}

#[test]
fn float_floor_ceil_trig() {
    approx("fn main() -> f64 { return floor(2.9) }", 2.0);
    approx("fn main() -> f64 { return floor(0 - 2.1) }", -3.0);
    approx("fn main() -> f64 { return ceil(2.1) }", 3.0);
    approx("fn main() -> f64 { return ceil(0 - 2.9) }", -2.0);
    approx("fn main() -> f64 { return sin(0.0) }", 0.0);
    approx("fn main() -> f64 { return cos(0.0) }", 1.0);
    approx("fn main() -> f64 { return tan(0.0) }", 0.0);
    approx(
        "fn main() -> f64 { return sin(3.141592653589793 / 2.0) }",
        1.0,
    );
    approx(
        "fn main() -> f64 { return atan2(1.0, 1.0) }",
        std::f64::consts::FRAC_PI_4,
    );
    approx("fn main() -> f64 { return log(1.0) }", 0.0);
    approx(
        "fn main() -> f64 { return log(2.718281828459045) }",
        1.0,
    );
    approx("fn main() -> f64 { return exp(0.0) }", 1.0);
    approx(
        "fn main() -> f64 { return exp(1.0) }",
        std::f64::consts::E,
    );
}

// ---- static E-TYPE / E-ARITY ----

#[test]
fn float_static_type_errors() {
    check_err_code("fn main() -> f64 { return sqrt(\"x\") }", "E-TYPE");
    check_err_code("fn main() -> f64 { return pow(1.0, \"x\") }", "E-TYPE");
    check_err_code("fn main() -> i32 { return abs([1]) }", "E-TYPE");
    check_err_code("fn main() -> f64 { return min(1, \"x\") }", "E-TYPE");
    check_err_code("fn main() -> f64 { return floor([1.0]) }", "E-TYPE");
}

#[test]
fn float_static_arity_errors() {
    check_err_code("fn main() -> f64 { return sqrt(1.0, 2.0) }", "E-ARITY");
    check_err_code("fn main() -> f64 { return pow(1.0) }", "E-ARITY");
    check_err_code("fn main() -> f64 { return min(1.0) }", "E-ARITY");
    check_err_code("fn main() -> i32 { return random_int(1) }", "E-ARITY");
    check_err_code("fn main() -> f64 { return random_float(1) }", "E-ARITY");
    check_err_code("fn main() -> str { return sha256() }", "E-ARITY");
    check_err_code("fn main() -> str { return hmac_sha256(\"k\") }", "E-ARITY");
}

// ---- runtime E-TYPE (Unknown-typed values) + domain errors ----

#[test]
fn float_runtime_type_errors() {
    // Map lookups are statically Unknown, so mistyped values fail at
    // run time with E-TYPE (never a silent 0.0).
    assert_eq!(
        run_err_code("fn main() -> f64 { let m = {\"x\": \"nope\"} return sqrt(m[\"x\"]) }"),
        "E-TYPE"
    );
    assert_eq!(
        run_err_code("fn main() -> f64 { let m = {\"x\": [1]} return log(m[\"x\"]) }"),
        "E-TYPE"
    );
    assert_eq!(
        run_err_code("fn main() -> f64 { let m = {\"x\": \"s\"} return pow(m[\"x\"], 2.0) }"),
        "E-TYPE"
    );
    assert_eq!(
        run_err_code("fn main() -> f64 { let m = {\"x\": {\"y\": 1}} return min(1.0, m[\"x\"]) }"),
        "E-TYPE"
    );
    assert_eq!(
        run_err_code("fn main() -> str { let m = {\"x\": 42} return sha256(m[\"x\"]) }"),
        "E-TYPE"
    );
    assert_eq!(
        run_err_code(
            "fn main() -> str { let m = {\"x\": 42} return hmac_sha256(\"k\", m[\"x\"]) }"
        ),
        "E-TYPE"
    );
}

#[test]
fn float_domain_errors() {
    // sqrt of a negative / log of <= 0: loud E-RUNTIME, never quiet NaN.
    let mut p = Parser::new("fn main() -> f64 { return sqrt(0 - 1.0) }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (r, _) = klang::runtime::run_with_argv(&mir, "main", &[], &[], &HashMap::new());
    let err = r.expect_err("sqrt(-1) must fail");
    assert_eq!(err.code, "E-RUNTIME");
    assert!(err.message.contains("sqrt"), "{}", err.message);

    assert_eq!(run_err_code("fn main() -> f64 { return log(0.0) }"), "E-RUNTIME");
    assert_eq!(
        run_err_code("fn main() -> f64 { return log(0 - 5.0) }"),
        "E-RUNTIME"
    );
    // A quiet NaN out of pow is the same class of silence: loud instead.
    assert_eq!(
        run_err_code("fn main() -> f64 { return pow(0 - 1.0, 0.5) }"),
        "E-RUNTIME"
    );
    // Inverted random bounds are a loud error, not an empty draw.
    assert_eq!(
        run_err_code("fn main() -> i32 { return random_int(5, 1) }"),
        "E-RUNTIME"
    );
}

// ---- W-SHADOW: user functions cannot reuse builtin names ----

#[test]
fn shadow_builtin_name_fails_loudly() {
    // Builtins win every call dispatch, so a user definition with a
    // builtin name could never run. The checker rejects it with
    // W-SHADOW — surfaced as an error because the diagnostics model
    // has no passing-with-warnings channel, which is exactly what
    // guarantees the override can never happen silently.
    for name in ["max", "sqrt", "len", "sha256", "random_int", "sort", "pow"] {
        let src =
            format!("fn {name}(a: i32) -> i32 {{ return a }} fn main() -> i32 {{ return 0 }}");
        let mut p = Parser::new(&src);
        let prog = p.parse_program().expect("parses");
        let err = klang::hir::TypedHIR::check(prog).expect_err("shadowing must fail");
        let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.contains(&"W-SHADOW"),
            "want W-SHADOW for `{name}`, got {codes:?}"
        );
        let msg = &err
            .iter()
            .find(|d| d.code == "W-SHADOW")
            .expect("W-SHADOW present")
            .message;
        assert!(msg.contains(name), "{msg}");
        assert!(msg.contains("never run"), "{msg}");
    }
    // Distinct names still check clean (the `mymax` convention used
    // across the test suite and examples).
    let mut p = Parser::new(
        "fn mymax(a: i32, b: i32) -> i32 { if a < b { return b } else { return a } } fn main() -> i32 { return mymax(1, 2) }",
    );
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}

// ---- pow magnitude overflow is loud ----

#[test]
fn pow_overflow_is_loud() {
    // 10^1000 overflows f64: E-RUNTIME, never quiet inf (same rule as
    // the NaN case).
    assert_eq!(
        run_err_code("fn main() -> f64 { return pow(10.0, 1000.0) }"),
        "E-RUNTIME"
    );
    // atan2 is bounded by pi: no overflow possible, still computes.
    approx(
        "fn main() -> f64 { return atan2(1.0, 1.0) }",
        std::f64::consts::FRAC_PI_4,
    );
}

// ---- random: shape + range, never exact values ----

#[test]
fn random_bounds_hold() {
    for _ in 0..50 {
        let (v, _) = run_ok("fn main() -> i32 { return random_int(1, 6) }");
        match v {
            klang::runtime::Value::Int(n) => assert!((1..=6).contains(&n), "{n}"),
            other => panic!("not int: {other:?}"),
        }
    }
    // Degenerate range always draws the endpoint.
    for _ in 0..10 {
        let (v, _) = run_ok("fn main() -> i32 { return random_int(7, 7) }");
        assert_eq!(v, klang::runtime::Value::Int(7));
    }
    // Negative ranges work.
    for _ in 0..20 {
        let (v, _) = run_ok("fn main() -> i32 { return random_int(0 - 10, 0 - 1) }");
        match v {
            klang::runtime::Value::Int(n) => assert!((-10..=-1).contains(&n), "{n}"),
            other => panic!("not int: {other:?}"),
        }
    }
    for _ in 0..20 {
        let (v, _) = run_ok("fn main() -> f64 { return random_float() }");
        let x = as_f64(&v);
        assert!((0.0..1.0).contains(&x), "{x}");
    }
}

#[test]
fn random_draws_vary() {
    // OS-seeded: 20 draws are not all identical (flakes at 6^-20 /
    // 2^-1060 — if this ever fails, the seeder is broken, not the test).
    let mut ints = std::collections::HashSet::new();
    for _ in 0..20 {
        let (v, _) = run_ok("fn main() -> i32 { return random_int(1, 1000000) }");
        ints.insert(as_f64(&v) as i64);
    }
    assert!(ints.len() > 1, "draws never vary");
}

// ---- sha256 / hmac_sha256 vectors ----

#[test]
fn hash_vectors() {
    // NIST vectors for SHA-256.
    let (v, _) = run_ok("fn main() -> str { return sha256(\"\") }");
    assert_eq!(
        v,
        klang::runtime::Value::Str(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string()
        )
    );
    let (v, _) = run_ok("fn main() -> str { return sha256(\"abc\") }");
    assert_eq!(
        v,
        klang::runtime::Value::Str(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string()
        )
    );
    // RFC 4231 case 2: key "Jefe", msg "what do ya want for nothing?".
    let (v, _) = run_ok(
        "fn main() -> str { return hmac_sha256(\"Jefe\", \"what do ya want for nothing?\") }",
    );
    assert_eq!(
        v,
        klang::runtime::Value::Str(
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843".to_string()
        )
    );
    // Deterministic: same input, same output.
    let (a, _) = run_ok("fn main() -> str { return sha256(\"klang\") }");
    let (b, _) = run_ok("fn main() -> str { return sha256(\"klang\") }");
    assert_eq!(a, b);
    // Avalanche: different input, different output, still 64 hex chars.
    let (c, _) = run_ok("fn main() -> str { return sha256(\"klanG\") }");
    assert_ne!(a, c);
    match &c {
        klang::runtime::Value::Str(s) => {
            assert_eq!(s.len(), 64);
            assert!(s.chars().all(|ch| ch.is_ascii_hexdigit()));
        }
        other => panic!("not str: {other:?}"),
    }
}

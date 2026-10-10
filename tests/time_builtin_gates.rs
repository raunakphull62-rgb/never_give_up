//! S6 time + env builtins: `now_ms()` / `now_iso()` / `sleep_ms(n)` / `get_env(name)`.
//!
//! `now_ms` is millis since the Unix epoch (Int, far past i32 like
//! `file_size`); `now_iso` is RFC 3339 UTC; `sleep_ms` needs `>= 0`
//! (`E-TIME-INVALID` otherwise); `get_env` is `""` when unset (same
//! absence convention as `env`, since Klang has no null).

use std::collections::HashMap;

use klang::parser::Parser;

fn test_dir(tag: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "klang-s6-time-{tag}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_src(src: &str) -> Result<(klang::runtime::Value, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    let (r, out) = klang::runtime::run_with_argv(&mir, "main", &[], &[], &HashMap::new());
    r.map(|v| (v, out))
}

fn run_ok(src: &str) -> (klang::runtime::Value, Vec<String>) {
    run_src(src).expect("must run clean").into()
}

fn int_of(v: &klang::runtime::Value) -> i64 {
    match v {
        klang::runtime::Value::Int(n) => *n,
        other => panic!("want int, got {other:?}"),
    }
}

fn str_of(v: &klang::runtime::Value) -> String {
    match v {
        klang::runtime::Value::Str(s) => s.clone(),
        other => panic!("want str, got {other:?}"),
    }
}

#[test]
fn s6_now_ms_monotonic_and_plausible() {
    let _dir = test_dir("now-ms");
    // Monotonic inside Klang (no large literals: i32 literals only).
    let (v, _) = run_ok(
        "fn main() -> i32 { let t1 = now_ms() let t2 = now_ms() if t2 < t1 { return 1 } return 42 }",
    );
    assert_eq!(int_of(&v), 42, "t2>=t1 (monotonic-ish)");
    // Plausible range checked on the Rust side (raw Int, no i32 literal).
    // D3: epoch millis are far past i32, so the raw-value probes declare
    // `-> i64` (returning them from `-> i32` is now a loud E-RUNTIME).
    let (raw1, _) = run_ok("fn main() -> i64 { return now_ms() }");
    let t1 = int_of(&raw1);
    let (raw2, _) = run_ok("fn main() -> i64 { return now_ms() }");
    let t2 = int_of(&raw2);
    assert!(t2 >= t1, "raw monotonic: {t1} <= {t2}");
    for t in [t1, t2] {
        assert!(
            (1_000_000_000_000..=10_000_000_000_000).contains(&t),
            "plausible millis (2001..2286), got {t}"
        );
    }
}

#[test]
fn s6_now_ms_raw_value_plausible() {
    let _dir = test_dir("now-ms-raw");
    // Raw Int value must be plausible millis (2020-01-01 .. 2286-11-20).
    // Checked in Rust: Klang i32 literals cannot spell 1e12.
    // D3: `-> i64`, since epoch millis exceed the `i32` return boundary.
    let (v, _) = run_ok("fn main() -> i64 { return now_ms() }");
    let t = int_of(&v);
    assert!(
        (1_577_836_800_000..=10_000_000_000_000).contains(&t),
        "plausible millis since epoch, got {t}"
    );
}

#[test]
fn s6_now_iso_rfc3339_shape() {
    let _dir = test_dir("now-iso");
    let (v, _) = run_ok("fn main() -> str { return now_iso() }");
    let s = str_of(&v);
    // Simple parser: YYYY-MM-DDTHH:MM:SS.mmmZ (24 chars).
    assert_eq!(s.len(), 24, "want 24-char RFC3339, got {s:?}");
    assert_eq!(&s[4..5], "-");
    assert_eq!(&s[7..8], "-");
    assert_eq!(&s[10..11], "T");
    assert_eq!(&s[13..14], ":");
    assert_eq!(&s[16..17], ":");
    assert_eq!(&s[19..20], ".");
    assert_eq!(&s[23..24], "Z");
    let num = |lo: usize, hi: usize| -> u32 {
        s[lo..hi].parse::<u32>().unwrap_or_else(|_| panic!("digits {lo}..{hi} in {s:?}"))
    };
    let (year, mon, day, hour, minu, sec, ms) =
        (num(0, 4), num(5, 7), num(8, 10), num(11, 13), num(14, 16), num(17, 19), num(20, 23));
    assert!((2020..=2100).contains(&year), "year {year} in {s:?}");
    assert!((1..=12).contains(&mon), "month {mon}");
    assert!((1..=31).contains(&day), "day {day}");
    assert!(hour <= 23 && minu <= 59 && sec <= 59 && ms <= 999);
    // Two reads are ordered (lexicographic UTC works for same-second+ms).
    let (v2, _) = run_ok("fn main() -> str { return now_iso() }");
    let s2 = str_of(&v2);
    assert!(s2 >= s, "second read {s2:?} >= first {s:?}");
}

#[test]
fn s6_sleep_ms_50_takes_at_least_50ms() {
    let _dir = test_dir("sleep");
    let start = std::time::Instant::now();
    let (v, _) = run_ok("fn main() -> i32 { return sleep_ms(50) }");
    let elapsed = start.elapsed();
    assert_eq!(int_of(&v), 1);
    assert!(
        elapsed.as_millis() >= 50,
        "sleep_ms(50) must take >=50ms, took {elapsed:?}"
    );
    assert!(
        elapsed.as_secs() < 5,
        "sleep_ms(50) must not hang, took {elapsed:?}"
    );
}

#[test]
fn s6_sleep_ms_zero_ok_negative_rejected() {
    let _dir = test_dir("sleep-neg");
    let (v, _) = run_ok("fn main() -> i32 { return sleep_ms(0) }");
    assert_eq!(int_of(&v), 1);
    let err = run_src("fn main() -> i32 { return sleep_ms(0 - 1) }")
        .expect_err("negative must fail")
        .code
        .clone();
    assert_eq!(err, "E-TIME-INVALID");
    let e2 = run_src("fn main() -> i32 { return sleep_ms(0 - 50) }").expect_err("must fail");
    assert_eq!(e2.code, "E-TIME-INVALID");
    assert!(
        e2.to_json().contains("negative"),
        "clear negative message: {}",
        e2.to_json()
    );
    // Dynamic non-number through a map (Unknown statics) is loud, never sleep-0.
    let e3 = run_src("fn main() -> i32 { let m = {\"s\": \"hi\"} return sleep_ms(m[\"s\"]) }")
        .expect_err("must fail");
    assert_eq!(e3.code, "E-TIME-INVALID");
}

#[test]
fn s6_get_env_found_and_not_found() {
    let _dir = test_dir("getenv");
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let name = format!("KLANG_S6_GETENV_{}_{n}", std::process::id());
    std::env::set_var(&name, "hello-s6");
    let src = format!("fn main() -> str {{ return get_env(\"{name}\") }}");
    let (v, _) = run_ok(&src);
    assert_eq!(str_of(&v), "hello-s6");
    std::env::remove_var(&name);
    // Absence is "" (no null in the language; same as `env`).
    let src2 = format!("fn main() -> str {{ return get_env(\"{name}\") }}");
    let (v2, out2) = run_ok(&src2);
    assert_eq!(str_of(&v2), "");
    assert_eq!(out2, Vec::<String>::new());
    let src3 = format!(
        "fn main() -> i32 {{ let v = get_env(\"{name}\") if v == \"\" {{ return 42 }} return 1 }}"
    );
    let (v3, _) = run_ok(&src3);
    assert_eq!(int_of(&v3), 42);
    // Missing var with a fresh unique name is also "" (never an error).
    let missing = format!("KLANG_S6_MISSING_{}_{n}", std::process::id());
    std::env::remove_var(&missing);
    let src4 = format!("fn main() -> str {{ return get_env(\"{missing}\") }}");
    let (v4, _) = run_ok(&src4);
    assert_eq!(str_of(&v4), "");
}

#[test]
fn s6_time_env_arity_and_types_checked() {
    let _dir = test_dir("checks");
    for (src, want) in [
        ("fn main() -> i32 { return now_ms(1) }", "E-ARITY"),
        ("fn main() -> i32 { return now_iso(1) }", "E-ARITY"),
        ("fn main() -> i32 { return sleep_ms() }", "E-ARITY"),
        ("fn main() -> i32 { return get_env() }", "E-ARITY"),
        ("fn main() -> i32 { return sleep_ms(\"hi\") }", "E-TYPE"),
        ("fn main() -> i32 { return get_env(42) }", "E-TYPE"),
    ] {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses {src}");
        let err = klang::hir::TypedHIR::check(prog).expect_err("must fail {src}");
        assert!(
            err.iter().any(|d| d.code == want),
            "want {want} for {src}, got {:?}",
            err.iter().map(|d| &d.code).collect::<Vec<_>>()
        );
    }
}

#[test]
fn s6_now_ms_and_get_env_shadow_as_builtins() {
    let _dir = test_dir("wshadow");
    for name in ["now_ms", "now_iso", "sleep_ms", "get_env"] {
        let src = format!("fn {name}() -> i32 {{ return 1 }} fn main() -> i32 {{ return 0 }}");
        let mut p = Parser::new(&src);
        let prog = p.parse_program().expect("parses {name}");
        let err = klang::hir::TypedHIR::check(prog).expect_err("must fail {name}");
        assert!(
            err.iter().any(|d| d.code == "W-SHADOW"),
            "want W-SHADOW for `{name}`, got {:?}",
            err.iter().map(|d| &d.code).collect::<Vec<_>>()
        );
    }
}

//! B1 stdlib gap fill gates (Lane B, stage B1).
//!
//! Covers the nine new flat builtins (`regex_find_all`, `regex_replace`,
//! `uuid_v4`, `path_absolute`, `file_mtime`, `sprintf`, `random_seed`,
//! `random_seeded_int`, `random_seeded_float`) and the four new string
//! methods (`pad_start`, `pad_end`, `repeat`, `lines`): happy paths, every
//! new error code (`E-REGEX-TOO-MANY`, `E-REGEX-TOO-LARGE`,
//! `E-FMT-INVALID`, `E-FMT-ARG-MISMATCH`, `E-STR-INVALID`,
//! `E-STR-TOO-LARGE`, `E-RANDOM-BOUNDS`, plus reused
//! `E-REGEX-INVALID-PATTERN` / `E-IO-*` / `E-TYPE` / `E-ARITY` /
//! `E-RUNTIME`), no-panic hostile inputs, and the JIT/v2 loud rejections.
//! File tests use real temp-dir paths (inside `temp_dir()` so the
//! unsafe-path guard does not fire) and clean up after themselves.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use klang::parser::Parser;

fn run_value(
    src: &str,
) -> Result<(klang::runtime::Value, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new())
}

fn run_str(src: &str) -> String {
    let (v, _) = run_value(src).expect("runs");
    v.render()
}

fn run_err(src: &str) -> klang::diagnostics::Diagnostic {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new())
        .expect_err("must fail")
}

fn check_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(
        codes.contains(&want),
        "want {want} in {codes:?}: {src}"
    );
}

static TMP_CTR: AtomicU64 = AtomicU64::new(0);

fn tmp(name: &str) -> String {
    let n = TMP_CTR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("klang-b1-test");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!(
        "{}-{}-{}",
        std::process::id(),
        n,
        name.replace('/', "_")
    ))
    .to_string_lossy()
    .replace('\\', "/")
}

// ---- regex_find_all ----

#[test]
fn b1_find_all_basic() {
    let got = run_str(
        "fn main() -> str { return regex_find_all(\"\\\\d+\", \"a1b22c333\").join(\",\") }",
    );
    assert_eq!(got, "1,22,333");
}

#[test]
fn b1_find_all_no_match_is_empty() {
    let got = run_str("fn main() -> str { return str(len(regex_find_all(\"xyz\", \"hello\"))) }");
    assert_eq!(got, "0");
}

#[test]
fn b1_find_all_bad_pattern() {
    let err = run_err("fn main() -> str { return regex_find_all(\"([\", \"hi\").join(\",\") }");
    assert_eq!(err.code, "E-REGEX-INVALID-PATTERN", "got: {}", err.to_json());
    assert!(err.message.contains("(["), "names the pattern: {}", err.message);
}

#[test]
fn b1_find_all_too_many() {
    let big = "a".repeat(10_001);
    let src = format!("fn main() -> str {{ return regex_find_all(\"a\", \"{big}\").join(\",\") }}");
    let err = run_err(&src);
    assert_eq!(err.code, "E-REGEX-TOO-MANY", "got: {}", err.to_json());
}

// ---- regex_replace ----

#[test]
fn b1_replace_basic_groups() {
    let got = run_str(
        "fn main() -> str { return regex_replace(\"(\\\\w+)@(\\\\w+)\", \"bob@example\", \"$1 at $2\") }",
    );
    assert_eq!(got, "bob at example");
}

#[test]
fn b1_replace_no_match_passthrough() {
    let got = run_str("fn main() -> str { return regex_replace(\"xyz\", \"hello\", \"Q\") }");
    assert_eq!(got, "hello");
}

#[test]
fn b1_replace_bad_pattern() {
    let err = run_err("fn main() -> str { return regex_replace(\"([\", \"hi\", \"x\") }");
    assert_eq!(err.code, "E-REGEX-INVALID-PATTERN", "got: {}", err.to_json());
}

#[test]
fn b1_replace_too_large() {
    let big = "a".repeat(600_000);
    let src = format!("fn main() -> str {{ return regex_replace(\"a\", \"{big}\", \"bb\") }}");
    let err = run_err(&src);
    assert_eq!(err.code, "E-REGEX-TOO-LARGE", "got: {}", err.to_json());
}

// ---- uuid_v4 ----

#[test]
fn b1_uuid_shape_and_uniqueness() {
    let a = run_str("fn main() -> str { return uuid_v4() }");
    let b = run_str("fn main() -> str { return uuid_v4() }");
    for u in [&a, &b] {
        assert_eq!(u.len(), 36, "uuid len: {u}");
        let cs: Vec<char> = u.chars().collect();
        assert_eq!(&cs[8], &'-');
        assert_eq!(&cs[13], &'-');
        assert_eq!(&cs[18], &'-');
        assert_eq!(&cs[23], &'-');
        assert_eq!(cs[14], '4', "version nibble: {u}");
        assert!(matches!(cs[19], '8' | '9' | 'a' | 'b'), "variant: {u}");
        assert!(u.chars().all(|c| c.is_ascii_hexdigit() || c == '-'), "{u}");
    }
    assert_ne!(a, b, "two draws differ");
}

#[test]
fn b1_uuid_arity() {
    check_err_code("fn main() -> str { return uuid_v4(1) }", "E-ARITY");
}

// ---- path_absolute ----

#[test]
fn b1_path_absolute_relative() {
    let got = run_str("fn main() -> str { return path_absolute(\"a/b\") }");
    let cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
    assert_eq!(got, format!("{cwd}/a/b"), "cwd-joined");
}

#[test]
fn b1_path_absolute_normalizes_and_empty() {
    let got = run_str("fn main() -> str { return path_absolute(\"/tmp/x/../y\") }");
    assert_eq!(got, "/tmp/y");
    let got = run_str("fn main() -> str { return path_absolute(\"\") }");
    let cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
    assert_eq!(got, cwd);
    // Clamped past root, never panics.
    let got = run_str("fn main() -> str { return path_absolute(\"/../../..\") }");
    assert_eq!(got, "/");
}

// ---- file_mtime ----

#[test]
fn b1_file_mtime_roundtrip() {
    let path = tmp("mtime.txt");
    std::fs::write(&path, "hey").unwrap();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let src = format!("fn main() -> str {{ print(str(file_mtime(\"{path}\"))) return str(file_mtime(\"{path}\")) }}");
    let (v, out) = run_value(&src).expect("runs");
    let got: f64 = v.render().parse().expect("float renders");
    let delta = (before - got).abs();
    assert!(delta < 60.0, "mtime {got} near now {before}");
    assert_eq!(out.len(), 1);
    std::fs::remove_file(&path).ok();
}

#[test]
fn b1_file_mtime_errors() {
    let missing = tmp("mtime-missing-xyz.txt");
    let _ = std::fs::remove_file(&missing);
    let err = run_err(&format!(
        "fn main() -> f64 {{ return file_mtime(\"{missing}\") }}"
    ));
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
    assert!(err.message.contains(&missing), "names the path: {}", err.message);
    // F1: plain mode allows `..`; missing target is E-IO-NOT-FOUND.
    let err = run_err("fn main() -> f64 { return file_mtime(\"../evil\") }");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

// ---- sprintf ----

#[test]
fn b1_sprintf_basic() {
    let got = run_str("fn main() -> str { return sprintf(\"hi {}!\", [\"bob\"]) }");
    assert_eq!(got, "hi bob!");
}

#[test]
fn b1_sprintf_multi_and_escapes() {
    let got = run_str(
        "fn main() -> str { return sprintf(\"{} + {} = {{{}}}\", [1, 2, 3]) }",
    );
    assert_eq!(got, "1 + 2 = {3}");
    let got = run_str("fn main() -> str { return sprintf(\"no holes\", [1]) }");
    assert_eq!(got, "no holes");
}

#[test]
fn b1_sprintf_arg_mismatch() {
    let err = run_err("fn main() -> str { return sprintf(\"{} {}\", [\"only\"]) }");
    assert_eq!(err.code, "E-FMT-ARG-MISMATCH", "got: {}", err.to_json());
    assert!(err.message.contains("{} {}"), "names the template: {}", err.message);
}

#[test]
fn b1_sprintf_lone_braces() {
    for src in [
        "fn main() -> str { return sprintf(\"hi {x}\", [\"v\"]) }",
        "fn main() -> str { return sprintf(\"hi }\", []) }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-FMT-INVALID", "{src}: {}", err.to_json());
    }
}

#[test]
fn b1_sprintf_static_checks() {
    check_err_code("fn main() -> str { return sprintf(\"hi\") }", "E-ARITY");
    check_err_code("fn main() -> str { return sprintf(42, []) }", "E-TYPE");
    check_err_code("fn main() -> str { return sprintf(\"{}\", \"nope\") }", "E-TYPE");
}

// ---- seeded RNG ----

#[test]
fn b1_seeded_deterministic() {
    let src = "fn main() -> i32 { random_seed(42) let a = random_seeded_int(1, 100) let f = random_seeded_float() random_seed(42) let b = random_seeded_int(1, 100) let g = random_seeded_float() assert(a == b) assert(f == g) return a }";
    let (v, _) = run_value(src).expect("runs");
    assert!(v.render().parse::<i64>().unwrap() >= 1);
}

#[test]
fn b1_seeded_bounds() {
    let src = "fn main() -> i32 { random_seed(7) let i = 0 while i < 50 { let v = random_seeded_int(-5, 5) assert(v >= -5) assert(v <= 5) let f = random_seeded_float() assert(f >= 0.0) assert(f < 1.0) i = i + 1 } return 0 }";
    run_value(src).expect("stays in range");
    let err = run_err("fn main() -> i32 { random_seed(1) return random_seeded_int(9, 2) }");
    assert_eq!(err.code, "E-RANDOM-BOUNDS", "got: {}", err.to_json());
    assert!(err.message.contains('9') && err.message.contains('2'), "names bounds: {}", err.message);
}

// ---- string methods ----

#[test]
fn b1_pad_exact() {
    assert_eq!(run_str("fn main() -> str { return \"hi\".pad_start(5, \"ab\") }"), "abahi");
    assert_eq!(run_str("fn main() -> str { return \"hi\".pad_end(5, \"ab\") }"), "hiaba");
    assert_eq!(run_str("fn main() -> str { return \"hello\".pad_start(3, \" \") }"), "hello");
    assert_eq!(run_str("fn main() -> str { return \"é\".pad_end(3, \"-\") }"), "é--");
}

#[test]
fn b1_pad_errors() {
    for (src, want) in [
        ("fn main() -> str { return \"hi\".pad_start(5, \"\") }", "E-STR-INVALID"),
        ("fn main() -> str { return \"hi\".pad_end(-1, \" \") }", "E-STR-INVALID"),
        ("fn main() -> str { return \"hi\".repeat(-2) }", "E-STR-INVALID"),
        ("fn main() -> str { return \"x\".pad_start(2000000, \"y\") }", "E-STR-TOO-LARGE"),
        ("fn main() -> str { return \"ab\".repeat(600000) }", "E-STR-TOO-LARGE"),
    ] {
        let err = run_err(src);
        assert_eq!(err.code, want, "{src}: {}", err.to_json());
    }
}

#[test]
fn b1_repeat_and_lines() {
    assert_eq!(run_str("fn main() -> str { return \"ab\".repeat(3) }"), "ababab");
    assert_eq!(run_str("fn main() -> str { return \"ab\".repeat(0) }"), "");
    assert_eq!(
        run_str("fn main() -> str { return \"a\\r\\nb\\nc\".lines().join(\"|\") }"),
        "a|b|c"
    );
}

#[test]
fn b1_method_static_checks() {
    check_err_code("fn main() -> str { return \"hi\".pad_start(5) }", "E-ARITY");
    check_err_code("fn main() -> str { return \"hi\".lines(1) }", "E-ARITY");
    // Dynamic-type misuse is a runtime E-TYPE, never a panic.
    let err = run_err(
        "fn main() -> str { let m = {\"k\": \"x\"} return \"hi\".pad_start(m[\"k\"], \" \") }",
    );
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
}

// ---- hostile inputs never panic ----

#[test]
fn b1_hostile_no_panic() {
    // Empty pattern matches at every boundary; huge-but-legal shapes work.
    assert_eq!(
        run_str("fn main() -> str { return str(len(regex_find_all(\"\", \"abc\"))) }"),
        "4"
    );
    assert_eq!(run_str("fn main() -> str { return sprintf(\"\", []) }"), "");
    assert_eq!(run_str("fn main() -> str { return \"\".repeat(5) }"), "");
    assert_eq!(run_str("fn main() -> str { return str(len(\"\".lines())) }"), "1");
    let got = run_str("fn main() -> str { return path_absolute(\"a/../../b\") }");
    assert!(got.ends_with("/b") || got == "/b", "{got}");
}

// ---- backend boundaries: JIT and v2 reject loudly ----

#[test]
fn b1_jit_rejects_new_builtins() {
    for src in [
        "fn main() -> i32 { print(regex_find_all(\"a\", \"a\").join(\",\")) return 0 }",
        "fn main() -> str { return sprintf(\"hi {}\", [\"x\"]) }",
        "fn main() -> str { return uuid_v4() }",
    ] {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok(), "{src}");
        let mir = klang::mir::lower(&prog);
        let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject");
        assert!(err.contains("unsupported"), "loud JIT-FAIL, got: {err}");
    }
}

#[test]
fn b1_v2_rejects_new_builtins() {
    let src = "fn main() -> i32 {\n    print(regex_find_all(\"a\", \"a\"))\n    return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let err = klang::runtime::v2::run_v2_program(&prog, "main").expect_err("v2 must reject");
    assert_eq!(err.code, "E-UNDEFINED", "got: {}", err.to_json());
}

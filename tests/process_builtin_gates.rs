//! S6 process builtins: `spawn(cmd, args)` + `run(cmd, args)`.
//!
//! `spawn` captures (status/stdout/stderr map, no shell); `run` inherits
//! stdio and returns only the exit code. Missing binaries are
//! `E-PROCESS-NOT-FOUND` (catchable via try/catch), never a crash.

use std::collections::HashMap;

use klang::parser::Parser;

fn test_dir(tag: &str) -> std::path::PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "klang-s6-process-{tag}-{}-{n}",
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

fn map_get<'a>(v: &'a klang::runtime::Value, key: &str) -> &'a klang::runtime::Value {
    match v {
        klang::runtime::Value::Map(m) => m
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, val)| val)
            .unwrap_or_else(|| panic!("map missing key `{key}`: {v:?}")),
        other => panic!("want map, got {other:?}"),
    }
}

fn str_of(v: &klang::runtime::Value) -> String {
    match v {
        klang::runtime::Value::Str(s) => s.clone(),
        other => panic!("want str, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn s6_spawn_echo_verbatim_spaces_and_quotes() {
    let _dir = test_dir("echo");
    // Spaces must not split, shell metachars must not expand, quotes
    // arrive literally (argv array, no shell).
    let (v, _) = run_ok(
        "fn main() -> i32 { let r = spawn(\"echo\", [\"hello world\", \"it's\"]) return r[\"status\"] }",
    );
    assert_eq!(int_of(&v), 0);
    let (v2, _) = run_ok(
        "fn main() -> str { let r = spawn(\"echo\", [\"hello world\"]) return r[\"stdout\"] }",
    );
    assert_eq!(str_of(&v2), "hello world\n");
    // Quotes verbatim: single quote inside a double-quoted Klang string.
    let (vq, _) = run_ok(
        "fn main() -> str { let r = spawn(\"echo\", [\"it's\"]) return r[\"stdout\"] }",
    );
    assert_eq!(str_of(&vq), "it's\n");
    // Full verbatim check via stdout content.
    let src = "fn main() -> str { let r = spawn(\"printf\", [\"<%s>\", \"hello world\", \"a;b|c$d\"]) return r[\"stdout\"] }";
    let (v3, _) = run_ok(src);
    assert_eq!(str_of(&v3), "<hello world><a;b|c$d>");
}

#[cfg(unix)]
#[test]
fn s6_spawn_nonzero_status_preserved() {
    let _dir = test_dir("nonzero");
    let (v, _) = run_ok("fn main() -> i32 { let r = spawn(\"false\", []) return r[\"status\"] }");
    assert_eq!(int_of(&v), 1);
    let (v2, _) = run_ok(
        "fn main() -> i32 { let r = spawn(\"sh\", [\"-c\", \"exit 3\"]) return r[\"status\"] }",
    );
    assert_eq!(int_of(&v2), 3);
}

#[test]
fn s6_spawn_missing_caught_by_try_catch() {
    let _dir = test_dir("missing");
    let src = "fn main() -> i32 { let code = \"\" try { let r = spawn(\"klang-s6-missing-xyz-12345\", []) code = \"no-error\" } catch e { code = e[\"code\"] } print(code) if code == \"E-PROCESS-NOT-FOUND\" { return 42 } return 1 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (r, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("catches, so Ok");
    assert_eq!(r, 42, "must catch E-PROCESS-NOT-FOUND, prints: {out:?}");
    assert!(out.iter().any(|l| l.contains("E-PROCESS-NOT-FOUND")));
}

#[cfg(unix)]
#[test]
fn s6_spawn_large_output_no_deadlock() {
    let _dir = test_dir("large");
    // >1 MB on stdout with stderr also piped: sequential reads would
    // deadlock on a full pipe; Command::output reads both on threads.
    let (v, _) = run_ok(
        "fn main() -> i32 { let r = spawn(\"seq\", [\"1\", \"300000\"]) return r[\"status\"] }",
    );
    assert_eq!(int_of(&v), 0);
    let (v2, _) = run_ok(
        "fn main() -> str { let r = spawn(\"seq\", [\"1\", \"300000\"]) return r[\"stdout\"] }",
    );
    let s = str_of(&v2);
    assert!(
        s.len() > 1_000_000,
        "stdout must exceed 1 MB, got {} bytes",
        s.len()
    );
    assert!(s.starts_with("1\n2\n3\n"));
    assert!(s.ends_with("300000\n"));
}

#[cfg(unix)]
#[test]
fn s6_spawn_stderr_captured() {
    let _dir = test_dir("stderr");
    let (v, _) = run_ok(
        "fn main() -> i32 { let r = spawn(\"ls\", [\"/nonexistent_klang_xyz_12345\"]) return r[\"status\"] }",
    );
    assert_ne!(int_of(&v), 0);
    let (v2, _) = run_ok(
        "fn main() -> str { let r = spawn(\"ls\", [\"/nonexistent_klang_xyz_12345\"]) return r[\"stderr\"] }",
    );
    assert!(!str_of(&v2).is_empty(), "stderr must be captured");
    // stdout/stderr are distinct pipes.
    let (v3, _) = run_ok(
        "fn main() -> str { let r = spawn(\"sh\", [\"-c\", \"echo out; echo err >&2\"]) return r[\"stdout\"] }",
    );
    assert_eq!(str_of(&v3), "out\n");
    let (v4, _) = run_ok(
        "fn main() -> str { let r = spawn(\"sh\", [\"-c\", \"echo out; echo err >&2\"]) return r[\"stderr\"] }",
    );
    assert_eq!(str_of(&v4), "err\n");
}

#[cfg(unix)]
#[test]
fn s6_run_exit_status_inherited() {
    let _dir = test_dir("run");
    let (v, _) = run_ok("fn main() -> i32 { return run(\"true\", []) }");
    assert_eq!(int_of(&v), 0);
    let (v2, _) = run_ok("fn main() -> i32 { return run(\"false\", []) }");
    assert_eq!(int_of(&v2), 1);
    let (v3, _) = run_ok("fn main() -> i32 { return run(\"sh\", [\"-c\", \"exit 3\"]) }");
    assert_eq!(int_of(&v3), 3);
}

#[test]
fn s6_run_missing_caught_by_try_catch() {
    let _dir = test_dir("run-missing");
    let src = "fn main() -> i32 { let code = \"\" try { let s = run(\"klang-s6-missing-xyz-12345\", []) code = \"no-error\" } catch e { code = e[\"code\"] } if code == \"E-PROCESS-NOT-FOUND\" { return 42 } return 1 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (r, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("catches, so Ok");
    assert_eq!(r, 42);
}

#[test]
fn s6_spawn_run_arity_and_types_checked() {
    let _dir = test_dir("checks");
    for (src, want) in [
        ("fn main() -> i32 { return spawn(\"echo\") }", "E-ARITY"),
        ("fn main() -> i32 { return run(\"true\") }", "E-ARITY"),
        ("fn main() -> i32 { return spawn(42, [\"x\"]) }", "E-TYPE"),
        ("fn main() -> i32 { return spawn(\"echo\", \"notarray\") }", "E-TYPE"),
        ("fn main() -> i32 { return run(42, []) }", "E-TYPE"),
        ("fn main() -> i32 { return run(\"true\", \"notarray\") }", "E-TYPE"),
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
fn s6_spawn_result_shape_is_status_stdout_stderr() {
    let _dir = test_dir("shape");
    // Exact shape: map with int `status` + str `stdout`/`stderr`.
    let src = "fn main() -> i32 { let r = spawn(\"echo\", [\"hi\"]) if r[\"status\"] != 0 { return 1 } if r[\"stdout\"] != \"hi\\n\" { return 2 } if r[\"stderr\"] != \"\" { return 3 } return 42 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (r, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(r, 42);
}

#[test]
fn s6_run_shadows_as_builtin_w_shadow() {
    let _dir = test_dir("wshadow");
    // `run` is a plain Ident builtin, so a user `fn run` must be W-SHADOW.
    let mut p = Parser::new("fn run() -> i32 { return 1 } fn main() -> i32 { return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(
        err.iter().any(|d| d.code == "W-SHADOW"),
        "want W-SHADOW for `run`, got {:?}",
        err.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    // `spawn` is keyword-reserved (stronger than W-SHADOW): `fn spawn`
    // cannot even parse, so builtins win by construction.
    let mut p2 = Parser::new("fn spawn() -> i32 { return 1 } fn main() -> i32 { return 0 }");
    assert!(
        p2.parse_program().is_err(),
        "`fn spawn` must not parse (keyword)"
    );
}

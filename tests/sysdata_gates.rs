//! SYS-DATA-1 — system + data builtins (Phase 2 Batch A).
//!
//! Covers `args`, `exit`, `cwd`, `set_env`, `list_dir`, `make_dir`,
//! `make_dirs`, `is_dir`, `is_file`, `rename_file`, `copy_file`,
//! `file_size`, `ord`, `chr`, `slice`, `sort`: happy paths, failure
//! codes (`E-EXIT` uncatchable, `E-ENV-INVALID`, `E-CHAR-INVALID`,
//! `E-IO-*`, `E-TYPE` on mixed sorts), and builtin arity/type checking.
//! File tests use real temp-dir paths (inside `temp_dir()` so the
//! unsafe-path guard does not fire); guard behavior itself is pinned by
//! `osio_file_gates`.

use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    run_argv(src, entry, &[])
}

fn run_argv(
    src: &str,
    entry: &str,
    argv: &[String],
) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    let (r, out) = klang::runtime::run_with_argv(&mir, entry, &[], argv, &HashMap::new());
    r.map(|v| (v.as_int() as i32, out))
}

fn tmp(name: &str) -> String {
    let dir = std::env::temp_dir().join("klang-sysdata-test");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().replace('\\', "/")
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

// ---- args / exit ----

#[test]
fn sysdata_args_empty_by_default() {
    let (v, out) = run_src("fn main() -> i32 { print(str(len(args()))) return len(args()) }", "main")
        .expect("runs");
    assert_eq!((v, out), (0, vec!["0".to_string()]));
}

#[test]
fn sysdata_args_flow_through() {
    let argv = ["a".to_string(), "--help".to_string(), "".to_string()];
    let (v, out) = run_argv(
        "fn main() -> i32 { let a = args() print(a.join(\",\")) print(str(len(a))) return len(a) }",
        "main",
        &argv,
    )
    .expect("runs");
    assert_eq!(v, 3);
    assert_eq!(out, vec!["a,--help,".to_string(), "3".to_string()]);
}

#[test]
fn sysdata_exit_code_and_message() {
    let mut p = Parser::new("fn main() -> i32 { print(\"before\") exit(3) return 0 }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (r, out) =
        klang::runtime::run_with_argv(&mir, "main", &[], &[], &HashMap::new());
    let err = r.expect_err("exit must unwind as E-EXIT");
    assert_eq!(err.code, "E-EXIT");
    assert_eq!(err.exit_code(), 3);
    assert!(err.is_exit());
    assert_eq!(out, vec!["before".to_string()]);
}

#[test]
fn sysdata_exit_is_uncatchable() {
    // Like E-CANCELLED, E-EXIT bypasses try/catch: catching exit would
    // let programs swallow termination.
    let mut p = Parser::new("fn main() -> i32 { try { exit(7) } catch e { return 99 } return 0 }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (r, _) = klang::runtime::run_with_argv(&mir, "main", &[], &[], &HashMap::new());
    let err = r.expect_err("exit must not be caught");
    assert_eq!(err.code, "E-EXIT");
    assert_eq!(err.exit_code(), 7);
}

// ---- cwd / set_env ----

#[test]
fn sysdata_cwd_matches_process() {
    let (v, out) = run_src("fn main() -> i32 { print(cwd()) return len(cwd()) }", "main")
        .expect("runs");
    let real = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(out, vec![real.clone()]);
    assert_eq!(v, real.len() as i32);
}

#[test]
fn sysdata_set_env_roundtrip() {
    let (v, out) = run_src(
        "fn main() -> i32 { set_env(\"KLANG_SYSDATA_PROBE\", \"v42\") print(env(\"KLANG_SYSDATA_PROBE\")) return 0 }",
        "main",
    )
    .expect("runs");
    assert_eq!(out, vec!["v42".to_string()]);
    assert_eq!(v, 0);
    std::env::remove_var("KLANG_SYSDATA_PROBE");
}

#[test]
fn sysdata_set_env_bad_name() {
    let err = run_src("fn main() -> i32 { set_env(\"a=b\", \"v\") return 0 }", "main")
        .expect_err("must fail");
    assert_eq!(err.code, "E-ENV-INVALID", "got: {}", err.to_json());
}

// ---- directory builtins ----

#[test]
fn sysdata_dir_roundtrip() {
    let base = tmp("roundtrip");
    let _ = std::fs::remove_dir_all(&base);
    let src = format!(
        "fn main() -> i32 {{ make_dirs(\"{base}/sub\") print(str(is_dir(\"{base}/sub\"))) print(str(is_file(\"{base}/sub\"))) make_dir(\"{base}/solo\") print(list_dir(\"{base}\").join(\",\")) write_file(\"{base}/a.txt\", \"hey\") print(str(file_size(\"{base}/a.txt\"))) print(str(is_file(\"{base}/a.txt\"))) copy_file(\"{base}/a.txt\", \"{base}/b.txt\") rename_file(\"{base}/b.txt\", \"{base}/c.txt\") print(read_file(\"{base}/c.txt\")) return 0 }}"
    );
    let (v, out) = run_src(&src, "main").expect("runs");
    assert_eq!(
        out,
        vec!["1", "0", "solo,sub", "3", "1", "hey"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>(),
        "dir assertions"
    );
    assert_eq!(v, 0);
    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn sysdata_dir_error_codes() {
    let missing = tmp("missing-dir-xyz-123");
    let _ = std::fs::remove_dir_all(&missing);
    // Missing dir: E-IO-NOT-FOUND.
    let err = run_src(
        &format!("fn main() -> i32 {{ print(list_dir(\"{missing}\")) return 0 }}"),
        "main",
    )
    .expect_err("must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
    // Missing file size: E-IO-NOT-FOUND.
    let err = run_src(
        &format!("fn main() -> i32 {{ print(str(file_size(\"{missing}/f\"))) return 0 }}"),
        "main",
    )
    .expect_err("must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND");
    // make_dir on an existing dir: E-IO-FAILED (use make_dirs instead).
    let err = run_src(
        "fn main() -> i32 { make_dir(\"/tmp\") return 0 }",
        "main",
    )
    .expect_err("must fail");
    assert_eq!(err.code, "E-IO-FAILED", "got: {}", err.to_json());
    // Unsafe paths stay E-RUNTIME (capability guard, not I/O).
    let err = run_src("fn main() -> i32 { print(list_dir(\"../evil\")) return 0 }", "main")
        .expect_err("must fail");
    assert_eq!(err.code, "E-RUNTIME");
}

// ---- ord / chr ----

#[test]
fn sysdata_ord_chr() {
    let (v, out) = run_src(
        "fn main() -> i32 { print(str(ord(\"A\"))) print(chr(65)) print(str(ord(chr(233)))) print(chr(8364)) return 0 }",
        "main",
    )
    .expect("runs");
    assert_eq!(out, vec!["65", "A", "233", "€"]);
    assert_eq!(v, 0);
}

#[test]
fn sysdata_ord_chr_errors() {
    let err = run_src("fn main() -> i32 { print(str(ord(\"\"))) return 0 }", "main")
        .expect_err("must fail");
    assert_eq!(err.code, "E-CHAR-INVALID", "got: {}", err.to_json());
    for bad in ["55296", "-1", "1114112"] {
        let err = run_src(
            &format!("fn main() -> i32 {{ print(chr({bad})) return 0 }}"),
            "main",
        )
        .expect_err("must fail");
        assert_eq!(err.code, "E-CHAR-INVALID", "{bad}: {}", err.to_json());
    }
}

// ---- slice / sort ----

#[test]
fn sysdata_slice_str_and_list() {
    let (v, out) = run_src(
        "fn main() -> i32 { print(slice(\"hello\", 1, 4)) print(slice([10, 20, 30, 40], 1, 3).join(\",\")) print(slice(\"abc\", 0, 0)) return 0 }",
        "main",
    )
    .expect("runs");
    assert_eq!(out, vec!["ell", "20,30", ""]);
    assert_eq!(v, 0);
}

#[test]
fn sysdata_slice_bounds_are_loud() {
    for src in [
        "fn main() -> i32 { print(slice(\"abc\", 0, 9)) return 0 }",
        "fn main() -> i32 { print(slice(\"abc\", 2, 1)) return 0 }",
        "fn main() -> i32 { print(slice(\"abc\", -1, 2)) return 0 }",
        "fn main() -> i32 { print(slice([1], 0, 5).join(\",\")) return 0 }",
    ] {
        let err = run_src(src, "main").expect_err("must fail");
        assert_eq!(err.code, "E-RUNTIME", "{src}: {}", err.to_json());
    }
}

#[test]
fn sysdata_sort_homogeneous() {
    let (v, out) = run_src(
        "fn main() -> i32 { print(sort([3, 1, 2]).join(\",\")) print(sort([\"b\", \"a\"]).join(\",\")) print(sort([2.5, 1.5]).join(\",\")) print(str(len(sort([1])))) print(str(len(sort(range(0, 0))))) return 0 }",
        "main",
    )
    .expect("runs");
    assert_eq!(out, vec!["1,2,3", "a,b", "1.5,2.5", "1", "0"]);
    assert_eq!(v, 0);
}

#[test]
fn sysdata_sort_mixed_is_type_error() {
    // Homogeneous only: ints+floats and strings+numbers are E-TYPE.
    for src in [
        "fn main() -> i32 { print(sort([1, \"a\"]).join(\",\")) return 0 }",
        "fn main() -> i32 { print(sort([1, 2.5]).join(\",\")) return 0 }",
        "fn main() -> i32 { print(sort([\"a\", 1]).join(\",\")) return 0 }",
    ] {
        let err = run_src(src, "main").expect_err("must fail");
        assert_eq!(err.code, "E-TYPE", "{src}: {}", err.to_json());
    }
}

// ---- arity / static types ----

#[test]
fn sysdata_builtin_arity() {
    for (src, _) in [
        ("fn main() -> i32 { print(str(len(args(1)))) return 0 }", "args"),
        ("fn main() -> i32 { exit() return 0 }", "exit"),
        ("fn main() -> i32 { print(slice(\"ab\", 1)) return 0 }", "slice"),
        ("fn main() -> i32 { print(sort([1], [2])) return 0 }", "sort"),
        ("fn main() -> i32 { print(cwd(1)) return 0 }", "cwd"),
        ("fn main() -> i32 { print(str(ord())) return 0 }", "ord"),
    ] {
        check_err_code(src, "E-ARITY");
    }
}

#[test]
fn sysdata_builtin_types() {
    check_err_code("fn main() -> i32 { exit(\"x\") return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(chr(\"a\")) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(list_dir(1)) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(sort(1)) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(slice(42, 0, 1)) return 0 }", "E-TYPE");
}

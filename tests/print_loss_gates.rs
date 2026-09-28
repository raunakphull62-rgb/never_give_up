//! HEAVY-TEST-1 — prints before a runtime failure must survive.
//!
//! A program that prints and then fails used to show only the failure
//! diagnostic: `run_with_output_value` dropped `ctx.output` on `Err`
//! and the CLI/MCP error arms printed just the diagnostic. The partial
//! runners now return the accumulated output alongside the outcome, and
//! every display site flushes it before the diagnostic. These tests pin
//! both the mechanism (partial runners) and the user-visible behavior
//! (real `klang run` stdout: prints first, `run: FAIL` after).

use std::collections::HashMap;

use klang::parser::Parser;

fn run_partial(
    src: &str,
) -> (
    Result<klang::runtime::Value, klang::diagnostics::Diagnostic>,
    Vec<String>,
) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output_value_partial(&mir, "main", &[], &HashMap::new())
}

fn missing_path(name: &str) -> String {
    let path = std::env::temp_dir().join(format!("klang-print-loss-{name}"));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn print_before_failure_survives_with_diagnostic() {
    // The heavy_print_loss.klang repro shape: one print, then a real
    // failing read_file. Both the output and the diagnostic must come
    // back — not one or the other.
    let path = missing_path("single.txt");
    let src = format!(
        "fn main() -> i32 {{ print(\"before failure\") print(read_file(\"{path}\")) return 0 }}"
    );
    let (r, out) = run_partial(&src);
    assert_eq!(out, vec!["before failure".to_string()]);
    let err = r.expect_err("read of a missing file must still fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn multiple_prints_before_failure_all_survive_in_order() {
    // At least two prints before the failure: all must appear, in order,
    // not just the first or the last one.
    let path = missing_path("multi.txt");
    let src = format!(
        "fn main() -> i32 {{ print(\"first\") print(40 + 2) print(\"third\") print(read_file(\"{path}\")) return 0 }}"
    );
    let (r, out) = run_partial(&src);
    assert_eq!(
        out,
        vec![
            "first".to_string(),
            "42".to_string(),
            "third".to_string()
        ]
    );
    let err = r.expect_err("must still fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn success_path_output_unchanged() {
    // The delegate keeps its contract: success still returns value and
    // output together through the original function.
    let mut p = Parser::new("fn main() -> i32 { print(\"hi\") return 42 }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(out, vec!["hi".to_string()]);
    assert_eq!(v.render(), "42");
}

#[test]
fn arity_failure_carries_empty_output() {
    // Nothing can have printed before an entry-arity rejection, so the
    // partial failure correctly carries empty output (never garbage).
    let mut p = Parser::new("fn main(a: i32) -> i32 { print(a) return a }");
    let prog = p.parse_program().expect("parses");
    let mir = klang::mir::lower(&prog);
    let (r, out) =
        klang::runtime::run_with_output_value_partial(&mir, "main", &[], &HashMap::new());
    let err = r.expect_err("arity must fail");
    assert_eq!(err.code, "E-ARITY");
    assert!(out.is_empty());
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn run_cli_split(path: &str, entry: &str) -> (String, String, i32) {
    let output = std::process::Command::new(klang_bin())
        .arg("run")
        .arg(path)
        .arg(entry)
        .output()
        .expect("klang binary runs");
    let code = output.status.code().unwrap_or(-1);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        code,
    )
}

fn run_cli(path: &str, entry: &str) -> (String, i32) {
    let (out, err, code) = run_cli_split(path, entry);
    let mut combined = out;
    combined.push_str(&err);
    (combined, code)
}

#[test]
fn cli_repro_prints_before_fail() {
    // The checked-in repro, through the real binary: the user must see
    // the print AND the failure, in that order — the exact sequence the
    // bug report requires. RUN-DEFAULT-1: raw print on stdout, diagnostic
    // on stderr.
    let repro = concat!(env!("CARGO_MANIFEST_DIR"), "/heavy_print_loss.klang");
    assert!(
        std::path::Path::new(repro).exists(),
        "repro file must exist: {repro}"
    );
    let (out, err, code) = run_cli_split(repro, "main");
    assert_eq!(code, 1, "failing run exits 1, stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("before failure"), "print on stdout:\n{out}");
    assert!(!out.contains("run: FAIL"), "FAIL on stderr, not stdout:\n{out}");
    assert!(err.contains("run: FAIL"), "FAIL on stderr:\n{err}");
    assert!(err.contains("E-IO-NOT-FOUND"), "diagnostic on stderr:\n{err}");
}

#[test]
fn cli_multiple_prints_before_fail_in_order() {
    let dir = std::env::temp_dir().join("klang-print-loss-cli");
    std::fs::create_dir_all(&dir).unwrap();
    let prog_path = dir.join("multi.klang");
    let missing = missing_path("cli-multi.txt");
    std::fs::write(
        &prog_path,
        format!(
            "fn main() -> i32 {{ print(\"alpha\") print(\"beta\") print(read_file(\"{missing}\")) return 0 }}"
        ),
    )
    .unwrap();
    let (out, err, code) = run_cli_split(&prog_path.to_string_lossy(), "main");
    assert_eq!(code, 1, "stdout:\n{out}\nstderr:\n{err}");
    let a = out.find("alpha").expect("first print on stdout:\n{out}");
    let b = out.find("beta").expect("second print on stdout:\n{out}");
    assert!(a < b, "prints in order:\n{out}");
    assert!(err.contains("run: FAIL"), "FAIL on stderr:\n{err}");
    assert!(err.contains("E-IO-NOT-FOUND"), "diagnostic on stderr:\n{err}");
}

#[test]
fn v2_print_before_failure_survives() {
    // v2 mirror of the same bug: a print before a failing tune must
    // survive in the partial runner.
    let src = r#"
schema Profile "1" { name: str, age: i32 }
fn main() -> i32 {
    print("before failure")
    let bad = tune<Profile>({name: "bob", age: "not a number"})
    print(bad)
    return 0
}
"#;
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let (r, out) = klang::runtime::v2::run_v2_program_partial(&prog, "main");
    assert_eq!(out, vec!["before failure".to_string()]);
    let err = r.expect_err("bad tune must still fail");
    assert_eq!(err.code, "E-SCHEMA-INVALID", "got: {}", err.to_json());
}

#[test]
fn v2_cli_prints_before_fail() {
    let dir = std::env::temp_dir().join("klang-print-loss-cli");
    std::fs::create_dir_all(&dir).unwrap();
    let prog_path = dir.join("pf_fail.v2");
    std::fs::write(
        &prog_path,
        "schema Profile \"1\" { name: str, age: i32 }\nfn main() -> i32 {\n    print(\"v2 before\")\n    let bad = tune<Profile>({name: \"bob\", age: \"not a number\"})\n    print(bad)\n    return 0\n}\n",
    )
    .unwrap();
    let (out, err, code) = run_cli_split(&prog_path.to_string_lossy(), "main");
    assert_eq!(code, 1, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("v2 before"), "print on stdout:\n{out}");
    assert!(err.contains("run: FAIL"), "FAIL on stderr:\n{err}");
    assert!(err.contains("E-SCHEMA-INVALID"), "diagnostic on stderr:\n{err}");
}

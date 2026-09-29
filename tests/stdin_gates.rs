//! STDLIB-IO-1 — stdin input builtins.
//!
//! `read_line()` / `parse_int()` / `parse_float()` through the real
//! binary with piped stdin (library calls cannot feed process stdin),
//! plus library-level parsing semantics: strictness, `i32` range,
//! redaction, negatives, and the pre-existing `int()` equivalence.
//! The MCP stdin reservation is pinned without touching the server loop.

use std::collections::HashMap;
use std::io::Write;
use std::process::Stdio;

use klang::parser::Parser;

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

/// Run the CLI with `stdin_data` fed to process stdin. Stdin is closed
/// after the write so reads past the data see EOF.
fn run_cli_with_stdin(args: &[&str], stdin_data: &str) -> (String, String, i32) {
    let mut child = std::process::Command::new(klang_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("klang binary runs");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin_data.as_bytes())
        .expect("stdin write");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn cli_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("klang-stdin-gates");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_prog(name: &str, src: &str) -> String {
    let path = cli_dir().join(name);
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().into_owned()
}

fn run_src_value(src: &str) -> Result<(klang::runtime::Value, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    Ok(klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new()).expect("runs"))
}

fn run_src_err(src: &str) -> klang::diagnostics::Diagnostic {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail")
}

// ---- parse_int semantics (no stdin needed) ----

#[test]
fn parse_int_ok_and_negatives() {
    let (v, _) = run_src_value("fn main() -> i32 { return parse_int(\"12\") }").unwrap();
    assert_eq!(v.render(), "12");
    let (v, _) = run_src_value("fn main() -> i32 { return parse_int(\"-5\") }").unwrap();
    assert_eq!(v.render(), "-5");
    let (v, _) = run_src_value("fn main() -> i32 { return parse_int(\"0\") }").unwrap();
    assert_eq!(v.render(), "0");
    let (v, _) = run_src_value("fn main() -> i32 { return parse_int(\"-2147483648\") }").unwrap();
    assert_eq!(v.render(), "-2147483648");
}

#[test]
fn parse_int_rejects_non_numeric() {
    for bad in ["abc", "", "12.5", "+5", " 12", "12 ", "0x10", "--5", "-"] {
        let d = run_src_err(&format!("fn main() -> i32 {{ return parse_int(\"{bad}\") }}"));
        assert_eq!(d.code, "E-PARSE-INT", "input {bad:?}: {}", d.to_json());
        assert!(d.message.contains(bad) || bad.is_empty(), "names {bad:?}: {}", d.message);
    }
}

#[test]
fn parse_int_rejects_out_of_i32_range() {
    let d = run_src_err("fn main() -> i32 { return parse_int(\"9999999999\") }");
    assert_eq!(d.code, "E-PARSE-INT", "{}", d.to_json());
    assert!(d.message.contains("9999999999"), "names text: {}", d.message);
}

#[test]
fn parse_int_redacts_overlong_text() {
    // 50 chars of offending input must not land verbatim in the message.
    let long = "x".repeat(50);
    let d = run_src_err(&format!("fn main() -> i32 {{ return parse_int(\"{long}\") }}"));
    assert_eq!(d.code, "E-PARSE-INT");
    assert!(!d.message.contains(&long), "full text leaked: {}", d.message);
    assert!(d.message.contains("redacted"), "marks redaction: {}", d.message);
    // At the boundary (40 chars) the text is still shown.
    let edge = "y".repeat(40);
    let d = run_src_err(&format!("fn main() -> i32 {{ return parse_int(\"{edge}\") }}"));
    assert!(d.message.contains(&edge), "boundary shown: {}", d.message);
}

#[test]
fn parse_float_ok_and_errors() {
    let (v, _) = run_src_value("fn main() -> f64 { return parse_float(\"3.5\") }").unwrap();
    assert_eq!(v.render(), "3.5");
    let (v, _) = run_src_value("fn main() -> f64 { return parse_float(\"-0.5\") }").unwrap();
    assert_eq!(v.render(), "-0.5");
    for bad in ["abc", "", "inf", "NaN", "12 "] {
        let d = run_src_err(&format!("fn main() -> f64 {{ return parse_float(\"{bad}\") }}"));
        assert_eq!(d.code, "E-PARSE-FLOAT", "input {bad:?}: {}", d.to_json());
    }
}

#[test]
fn legacy_int_builtin_still_converts() {
    // `int()` predates `parse_int()`: it converts ints, floats (truncating)
    // and strings with generic `E-RUNTIME`. Pinned so the new strict
    // builtin cannot silently change the old one.
    let (v, _) = run_src_value("fn main() -> i32 { return int(\"42\") }").unwrap();
    assert_eq!(v.render(), "42");
    let (v, _) = run_src_value("fn main() -> i32 { return int(4.9) }").unwrap();
    assert_eq!(v.render(), "4");
    let mut p = Parser::new("fn main() -> i32 { return int(\"abc\") }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let d = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    assert_eq!(d.code, "E-RUNTIME", "{}", d.to_json());
}

// ---- read_line through the real binary with piped stdin ----

#[test]
fn read_line_piped_input() {
    let prog = write_prog("echo.klang", "fn main() -> i32 {\n    print(read_line())\n    return 0\n}\n");
    let (out, err, code) = run_cli_with_stdin(&["run", &prog], "hello\n");
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "hello\n", "got:\n{out}");
    assert!(err.is_empty());
}

#[test]
fn read_line_eof_is_empty() {
    // Closed stdin: read sees EOF and yields "" (prints an empty line).
    let prog = write_prog("eof.klang", "fn main() -> i32 {\n    print(read_line())\n    return 0\n}\n");
    let (out, err, code) = run_cli_with_stdin(&["run", &prog], "");
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "\n", "empty print, got:\n{out}");
}

#[test]
fn read_line_empty_line_matches_eof() {
    // A bare newline is indistinguishable from EOF: both yield "".
    let prog = write_prog("empty.klang", "fn main() -> i32 {\n    print(read_line())\n    return 0\n}\n");
    let (out, _, code) = run_cli_with_stdin(&["run", &prog], "\n");
    assert_eq!(code, 0);
    assert_eq!(out, "\n", "got:\n{out}");
}

#[test]
fn read_line_strips_crlf() {
    let prog = write_prog("crlf.klang", "fn main() -> i32 {\n    print(read_line())\n    return 0\n}\n");
    let (out, _, code) = run_cli_with_stdin(&["run", &prog], "a\r\n");
    assert_eq!(code, 0);
    assert_eq!(out, "a\n", "CRLF stripped, got:\n{out:?}");
}

#[test]
fn read_line_negative_number_roundtrip() {
    let prog = write_prog(
        "neg.klang",
        "fn main() -> i32 {\n    print(parse_int(read_line()))\n    return 0\n}\n",
    );
    let (out, _, code) = run_cli_with_stdin(&["run", &prog], "-7\n");
    assert_eq!(code, 0);
    assert_eq!(out, "-7\n", "got:\n{out}");
}

#[test]
fn piped_non_numeric_is_clean_error_exit_1() {
    // Prior prints stay on stdout; the diagnostic goes to stderr.
    let prog = write_prog(
        "pnerr.klang",
        "fn main() -> i32 {\n    print(\"go\")\n    print(parse_int(read_line()))\n    return 0\n}\n",
    );
    let (out, err, code) = run_cli_with_stdin(&["run", &prog], "abc\n");
    assert_eq!(code, 1, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "go\n", "prior print survives on stdout, got:\n{out}");
    assert!(err.contains("run: FAIL"), "stderr:\n{err}");
    assert!(err.contains("E-PARSE-INT"), "stderr:\n{err}");
    assert!(err.contains("abc"), "names text:\n{err}");
}

#[test]
fn calculator_two_ints_default_plus() {
    // examples/calculator.klang: two integers, empty operator line means +.
    let calc = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/calculator.klang");
    assert!(std::path::Path::new(calc).exists(), "example must exist");
    let (out, err, code) = run_cli_with_stdin(&["run", calc], "12\n5\n");
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "17\n", "got:\n{out}");
}

#[test]
fn calculator_explicit_operator() {
    let calc = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/calculator.klang");
    let (out, _, code) = run_cli_with_stdin(&["run", calc], "10\n3\n*\n");
    assert_eq!(code, 0);
    assert_eq!(out, "30\n", "got:\n{out}");
}

// ---- MCP stdin protection + backend boundaries ----

#[test]
fn mcp_reserved_stdin_reads_eof_without_blocking() {
    // With stdin reserved for the MCP loop, read_line returns ""
    // immediately instead of consuming protocol bytes.
    klang::stdlib::io::reserve_stdin_for_mcp();
    assert!(klang::stdlib::io::stdin_reserved_for_mcp());
    assert_eq!(klang::stdlib::io::read_line(), "");
}

#[test]
fn jit_rejects_stdin_builtins() {
    // int-only JIT has no stdin: loud rejection, never silent code.
    let mut p = Parser::new("fn main() -> i32 {\n    print(read_line())\n    return 0\n}\n");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported"), "{err}");
}

#[test]
fn v2_has_no_stdin_builtins() {
    // run-v2 is unaffected: read_line is simply undefined there.
    let src = "fn main() -> i32 {\n  print(read_line())\n  return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let (r, _) = klang::runtime::v2::run_v2_program_partial(&prog, "main");
    let err = r.expect_err("must fail");
    assert_eq!(err.code, "E-UNDEFINED", "{}", err.to_json());
}

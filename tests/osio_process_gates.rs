//! STDLIB-OSIO Phase 2 — Process execution gates.
//!
//! Covers `run_process(cmd, args)` per the PRD: a succeeding command
//! with captured stdout, a non-zero exit as a normal result (populating
//! `exit_code`, not raising), a nonexistent command (`E-PROCESS-NOT-FOUND`),
//! stderr capture, argv-array fidelity for spaces/special characters
//! (proves no shell string-join), plus `E-PROCESS-*` code specificity
//! and builtin arity/type checking.

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
fn osio_process_run_success_captures_stdout() {
    let src = "fn main() -> i32 { let r = run_process(\"echo\", [\"hello\"]) print(r[\"stdout\"]) return r[\"exit_code\"] }";
    let (v, out) = run_src(src, "main").expect("runs");
    assert_eq!(v, 0, "echo exits 0");
    assert_eq!(out, vec!["hello\n".to_string()]);
}

#[test]
fn osio_process_nonzero_exit_is_result_not_error() {
    // Design decision (PRD §4): non-zero exits populate `exit_code`;
    // they do NOT raise. This test proves it: `false` exits 1 yet the
    // call itself is `Ok`.
    let src = "fn main() -> i32 { let r = run_process(\"false\", []) return r[\"exit_code\"] }";
    let (v, _) = run_src(src, "main").expect("non-zero exit must still be Ok");
    assert_eq!(v, 1, "false exits 1");
}

#[test]
fn osio_process_stderr_captured_with_nonzero_exit() {
    let src = "fn main() -> i32 { let r = run_process(\"ls\", [\"/nonexistent_klang_xyz_12345\"]) print(r[\"stderr\"]) return r[\"exit_code\"] }";
    let (v, out) = run_src(src, "main").expect("runs");
    assert_ne!(v, 0, "ls on a missing path exits non-zero, got {v}");
    assert_eq!(out.len(), 1);
    assert!(
        !out[0].is_empty(),
        "stderr must be captured, got empty"
    );
}

#[test]
fn osio_process_missing_command_is_not_found() {
    let src = "fn main() -> i32 { let r = run_process(\"klang-osio-nonexistent-xyz-12345\", []) return r[\"exit_code\"] }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-PROCESS-NOT-FOUND", "got: {}", err.to_json());
    assert!(
        err.to_json().contains("No such file")
            || err.to_json().contains("not found")
            || err.to_json().contains("os error 2"),
        "real OS cause, not generic: {}",
        err.to_json()
    );
}

#[test]
fn osio_process_argv_array_not_shell() {
    // If the implementation secretly joined args into a shell string,
    // the space in "hello world" would split and `;`, `|`, `$` would
    // act as shell metacharacters. `printf '<%s>'` reveals argv
    // boundaries exactly: each arg renders inside its own `<>`.
    let src = "fn main() -> i32 { let r = run_process(\"printf\", [\"<%s>\", \"hello world\", \"a;b|c$d\"]) print(r[\"stdout\"]) return r[\"exit_code\"] }";
    let (v, out) = run_src(src, "main").expect("runs");
    assert_eq!(v, 0);
    assert_eq!(out, vec!["<hello world><a;b|c$d>".to_string()]);
}

#[test]
fn osio_process_error_codes_are_specific() {
    // Rust-level mapping: NotFound/PermissionDenied -> E-PROCESS-NOT-FOUND
    // (missing or not executable), other -> E-PROCESS-FAILED with the
    // real OS message.
    let nf = std::io::Error::new(std::io::ErrorKind::NotFound, "entity not found xyz");
    let d = klang::stdlib::process::map_spawn_error("mycmd", &nf);
    assert_eq!(d.code, "E-PROCESS-NOT-FOUND");
    let pd = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied exec abc");
    let d = klang::stdlib::process::map_spawn_error("mycmd", &pd);
    assert_eq!(d.code, "E-PROCESS-NOT-FOUND");
    assert!(
        d.to_json().contains("denied exec abc"),
        "real cause, not generic: {}",
        d.to_json()
    );
    let other = std::io::Error::new(std::io::ErrorKind::StorageFull, "no space proc987");
    let d = klang::stdlib::process::map_spawn_error("mycmd", &other);
    assert_eq!(d.code, "E-PROCESS-FAILED");
    assert!(
        d.to_json().contains("no space proc987"),
        "real cause, not generic: {}",
        d.to_json()
    );
    // Distinct causes per code (never one shared generic string).
    let a = klang::stdlib::process::map_spawn_error(
        "mycmd",
        &std::io::Error::new(std::io::ErrorKind::NotFound, "nf"),
    );
    let b = klang::stdlib::process::map_spawn_error(
        "mycmd",
        &std::io::Error::new(std::io::ErrorKind::StorageFull, "sf"),
    );
    assert_ne!(a.code, b.code);
    assert_ne!(a.cause, b.cause);
}

#[test]
fn osio_process_arg_types_checked() {
    let mut p = Parser::new("fn main() -> i32 { return run_process(42, [\"x\"]) }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { return run_process(\"echo\", \"notarray\") }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { return run_process(\"echo\") }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
}

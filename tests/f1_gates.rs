//! F1 — plain-mode file access + `set_env` validation.
//!
//! Plain mode (`klang run` without `--sandbox`, library callers) allows
//! ALL relative and absolute paths; the OS decides (`E-IO-NOT-FOUND`,
//! `E-IO-PERMISSION`, `E-IO-FAILED`). Only `klang run --sandbox` keeps
//! the strict temp-dir-only guard (`E-SANDBOX-FILE`).
//!
//! `set_env("", _)` and names with `=`/NUL are `E-ENV-INVALID` naming the
//! bad name, never a panic; `get_env` keeps working afterwards.

use std::collections::HashMap;
use std::io::Write;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use klang::parser::Parser;

static CTR: AtomicU64 = AtomicU64::new(0);
static F1_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn next_tag(tag: &str) -> std::path::PathBuf {
    let c = CTR.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("klang-f1-{tag}-{}-{c}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn parse(src: &str) -> klang::ast::Program {
    Parser::new(src).parse_program().expect("parses")
}

fn check_ok(src: &str) -> klang::ast::Program {
    let prog = parse(src);
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    prog
}

/// Run `src` in-process with sandbox flag set as given. Serializes on the
/// global mutex and restores `false` afterwards.
fn run_lib(
    src: &str,
) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new())
}

fn run_lib_sandboxed(
    src: &str,
    sandbox: bool,
) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let _guard = F1_LOCK.lock().unwrap();
    klang::sandbox::set_sandbox(sandbox);
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    klang::sandbox::set_sandbox(false);
    r
}

fn run_cli(args: &[&str], cwd: &std::path::Path) -> (String, String, i32) {
    let mut child = std::process::Command::new(klang_bin())
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("klang binary runs");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(b"")
        .expect("stdin write");
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn write_prog(dir: &std::path::Path, name: &str, src: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, src).unwrap();
    p.to_string_lossy().replace('\\', "/")
}

// ---- plain mode: absolute paths ----

#[test]
fn f1_plain_absolute_write_read_roundtrip() {
    let base = next_tag("abs-rw");
    let target = base.join("data.txt");
    let ps = target.to_string_lossy().replace('\\', "/");
    let src = format!(
        "fn main() -> i32 {{ write_file(\"{ps}\", \"f1-hello\") print(read_file(\"{ps}\")) return 0 }}"
    );
    let (v, out) = run_lib(&src).expect("plain absolute rw runs");
    assert_eq!(v, 0);
    assert_eq!(out, vec!["f1-hello".to_string()]);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "f1-hello");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn f1_plain_missing_absolute_is_io_not_found() {
    let base = next_tag("abs-missing");
    let target = base.join("nope-xyz-12345.txt");
    let _ = std::fs::remove_file(&target);
    let ps = target.to_string_lossy().replace('\\', "/");
    let src = format!("fn main() -> i32 {{ print(read_file(\"{ps}\")) return 0 }}");
    let err = run_lib(&src).expect_err("missing must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn f1_plain_absolute_outside_tmp_missing_is_io_not_found() {
    // Outside the temp dir entirely: plain mode still reaches the OS.
    let src =
        "fn main() -> i32 { print(read_file(\"/no/such/klang-file-xyz-outside-tmp\")) return 0 }";
    let err = run_lib(src).expect_err("must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn f1_plain_absolute_cli_roundtrip() {
    let base = next_tag("abs-cli");
    let target = base.join("cli.txt");
    let ps = target.to_string_lossy().replace('\\', "/");
    let prog_src = format!(
        "fn main() -> i32 {{\n    write_file(\"{ps}\", \"cli-ok\")\n    print(read_file(\"{ps}\"))\n    return 0\n}}\n"
    );
    let prog = write_prog(&base, "abs.klang", &prog_src);
    let (out, err, code) = run_cli(&["run", &prog], &base);
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "cli-ok\n", "got: {out:?}");
    let _ = std::fs::remove_dir_all(&base);
}

// ---- plain mode: relative `..` ----

#[test]
fn f1_plain_dotdot_cli_works() {
    let base = next_tag("dotdot-cli");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(base.join("file.txt"), "dotdot-ok").unwrap();
    let prog_src = "fn main() -> i32 {\n    print(read_file(\"../file.txt\"))\n    return 0\n}\n";
    let prog = write_prog(&work, "rel.klang", prog_src);
    let (out, err, code) = run_cli(&["run", &prog], &work);
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "dotdot-ok\n", "got: {out:?}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn f1_plain_dotdot_write_cli_works() {
    let base = next_tag("dotdot-write");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let prog_src = "fn main() -> i32 {\n    write_file(\"../up.txt\", \"up-ok\")\n    print(read_file(\"../up.txt\"))\n    return 0\n}\n";
    let prog = write_prog(&work, "w.klang", prog_src);
    let (out, err, code) = run_cli(&["run", &prog], &work);
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "up-ok\n", "got: {out:?}");
    assert_eq!(
        std::fs::read_to_string(base.join("up.txt")).unwrap(),
        "up-ok"
    );
    let _ = std::fs::remove_dir_all(&base);
}

// ---- sandbox mode: still strict ----

#[test]
fn f1_sandbox_rejects_absolute_with_sandbox_code() {
    let base = next_tag("sbx-abs");
    let target = base.join("data.txt");
    std::fs::write(&target, "secret").unwrap();
    let ps = target.to_string_lossy().replace('\\', "/");
    // Inside the temp dir it is allowed even in sandbox; use a path
    // outside tmp to prove denial.
    let src =
        "fn main() -> i32 { print(read_file(\"/no/such/klang-file-xyz-outside-tmp\")) return 0 }";
    let err = run_lib_sandboxed(src, true).expect_err("sandbox must deny abs");
    assert_eq!(err.code, "E-SANDBOX-FILE", "got: {}", err.to_json());
    let _ = std::fs::remove_dir_all(&base);
    let _ = ps;
}

#[test]
fn f1_sandbox_rejects_missing_absolute_with_sandbox_code() {
    // Even a missing file outside tmp is a sandbox denial, not NOT-FOUND.
    let src =
        "fn main() -> i32 { print(read_file(\"/no/such/klang-f1-missing-xyz\")) return 0 }";
    let err = run_lib_sandboxed(src, true).expect_err("sandbox must deny");
    assert_eq!(err.code, "E-SANDBOX-FILE", "got: {}", err.to_json());
}

#[test]
fn f1_sandbox_rejects_dotdot_with_sandbox_code() {
    let src = "fn main() -> i32 { print(read_file(\"../evil\")) return 0 }";
    let err = run_lib_sandboxed(src, true).expect_err("sandbox must deny ..");
    assert_eq!(err.code, "E-SANDBOX-FILE", "got: {}", err.to_json());
}

#[test]
fn f1_sandbox_cli_rejects_dotdot_and_absolute() {
    let base = next_tag("sbx-cli");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(base.join("file.txt"), "rel-ok").unwrap();
    let prog_dot = write_prog(
        &work,
        "rel.klang",
        "fn main() -> i32 {\n    print(read_file(\"../file.txt\"))\n    return 0\n}\n",
    );
    let (_out, err, code) = run_cli(&["run", "--sandbox", &prog_dot], &work);
    assert_eq!(code, 1, "sandbox must fail");
    assert!(err.contains("E-SANDBOX-FILE"), "stderr:\n{err}");
    let prog_abs = write_prog(
        &base,
        "abs.klang",
        "fn main() -> i32 {\n    print(read_file(\"/no/such/klang-file-xyz-outside-tmp\"))\n    return 0\n}\n",
    );
    let (_out, err, code) = run_cli(&["run", "--sandbox", &prog_abs], &base);
    assert_eq!(code, 1, "sandbox must fail");
    assert!(err.contains("E-SANDBOX-FILE"), "stderr:\n{err}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn f1_sandbox_tmp_absolute_still_allowed() {
    // Temp-dir files work in both modes (sandbox happy path).
    let _guard = F1_LOCK.lock().unwrap();
    klang::sandbox::set_sandbox(true);
    let base = next_tag("sbx-tmp-ok");
    let target = base.join("ok.txt");
    std::fs::write(&target, "sbx-tmp-ok").unwrap();
    let ps = target.to_string_lossy().replace('\\', "/");
    let src = format!("fn main() -> i32 {{ print(read_file(\"{ps}\")) return 0 }}");
    let prog = check_ok(&src);
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("tmp allowed");
    assert_eq!(out, vec!["sbx-tmp-ok".to_string()]);
    assert_eq!(v, 0);
    klang::sandbox::set_sandbox(false);
    let _ = std::fs::remove_dir_all(&base);
}

// ---- set_env validation ----

#[test]
fn f1_set_env_empty_name_is_env_invalid() {
    let err = run_lib("fn main() -> i32 { set_env(\"\", \"x\") return 0 }")
        .expect_err("empty name must fail");
    assert_eq!(err.code, "E-ENV-INVALID", "got: {}", err.to_json());
    assert!(
        err.message.contains("\"\""),
        "message must name the bad (empty) name: {}",
        err.to_json()
    );
}

#[test]
fn f1_set_env_equals_in_name_is_env_invalid() {
    let err = run_lib("fn main() -> i32 { set_env(\"a=b\", \"x\") return 0 }")
        .expect_err("must fail");
    assert_eq!(err.code, "E-ENV-INVALID", "got: {}", err.to_json());
    assert!(
        err.message.contains("a=b"),
        "message must name the bad name: {}",
        err.to_json()
    );
}

#[test]
fn f1_set_env_nul_in_name_is_env_invalid() {
    // `\u0000` smuggles a real NUL into the Klang string.
    let err =
        run_lib("fn main() -> i32 { set_env(\"a\\u0000b\", \"x\") return 0 }")
            .expect_err("NUL name must fail");
    assert_eq!(err.code, "E-ENV-INVALID", "got: {}", err.to_json());
}

#[test]
fn f1_set_env_nul_in_value_is_env_invalid() {
    let err =
        run_lib("fn main() -> i32 { set_env(\"KLANG_F1_PROBE\", \"a\\u0000b\") return 0 }")
            .expect_err("NUL value must fail");
    assert_eq!(err.code, "E-ENV-INVALID", "got: {}", err.to_json());
}

#[test]
fn f1_get_env_still_works_after_bad_set_env() {
    // Bad `set_env` calls never panic and leave the env intact.
    let var = format!("KLANG_F1_AFTER_{}", std::process::id());
    let src = format!(
        "fn main() -> i32 {{ try {{ set_env(\"\", \"x\") }} catch e {{ print(e[\"code\"]) }} try {{ set_env(\"a=b\", \"x\") }} catch e {{ print(e[\"code\"]) }} set_env(\"{var}\", \"v42\") print(get_env(\"{var}\")) print(env(\"{var}\")) return 0 }}"
    );
    let (v, out) = run_lib(&src).expect("runs");
    assert_eq!(v, 0);
    assert_eq!(
        out,
        vec![
            "E-ENV-INVALID".to_string(),
            "E-ENV-INVALID".to_string(),
            "v42".to_string(),
            "v42".to_string(),
        ],
        "{out:?}"
    );
    std::env::remove_var(&var);
}

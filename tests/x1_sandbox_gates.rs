//! X1 Step 2 — `klang run --sandbox` gates (both modes).
//!
//! Sandbox mode (CLI `--sandbox` or `klang::sandbox::set_sandbox(true)`
//! + strict file guard) limits files to the temp dir (`E-SANDBOX-FILE`)
//! and denies process spawn/run (`E-SANDBOX-SPAWN`) and network
//! (`E-SANDBOX-NETWORK`). Plain `klang run` keeps normal behavior
//! (relative `..` allowed, spawn/network attempted). Threads and FFI
//! have no runtime surface in this tree, so there is nothing to deny
//! for those categories (reserved codes documented in `src/sandbox.rs`).
//!
//! Library tests serialize on a mutex because the sandbox/file-guard
//! flags are process-global; CLI tests spawn the real binary and are
//! isolated by the OS.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use klang::parser::Parser;

static CTR: AtomicU64 = AtomicU64::new(0);
static SANDBOX_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn tmpdir(tag: &str) -> PathBuf {
    let c = CTR.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("klang-x1-sbx-{tag}-{}-{c}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &PathBuf, rel: &str, content: &str) -> String {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, content).unwrap();
    p.to_string_lossy().replace('\\', "/")
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn run_cli(args: &[&str], cwd: &PathBuf) -> (String, String, i32) {
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

/// Run `src` with the sandbox/file-guard flags set as given. Serializes
/// on the global mutex and restores defaults (`false`/`false`) after.
fn run_with_flags(src: &str, sandbox: bool, relaxed: bool) -> Result<(), String> {
    let _guard = SANDBOX_LOCK.lock().unwrap();
    klang::sandbox::set_sandbox(sandbox);
    klang::runtime::set_file_guard_relaxed(relaxed);
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    let code = match &r {
        Ok(_) => String::new(),
        Err(d) => d.code.clone(),
    };
    let msg = match &r {
        Ok(_) => String::new(),
        Err(d) => d.to_json(),
    };
    klang::sandbox::set_sandbox(false);
    klang::runtime::set_file_guard_relaxed(false);
    match r {
        Ok(_) => Ok(()),
        Err(_) => Err(format!("{code} {msg}")),
    }
}

// ---- library: file guard in both modes ----

#[test]
fn x1_sandbox_file_parent_rejected_with_sandbox_code() {
    let src = "fn main() -> i32 { print(read_file(\"../evil\")) return 0 }";
    let err = run_with_flags(src, true, false).expect_err("sandbox must deny ..");
    assert!(err.starts_with("E-SANDBOX-FILE"), "got: {err}");
    assert!(err.contains("..") || err.contains("parent"), "got: {err}");
}

#[test]
fn x1_sandbox_file_absolute_outside_tmp_rejected_with_sandbox_code() {
    let src =
        "fn main() -> i32 { print(read_file(\"/no/such/klang-file-xyz-outside-tmp\")) return 0 }";
    let err = run_with_flags(src, true, false).expect_err("sandbox must deny abs");
    assert!(err.starts_with("E-SANDBOX-FILE"), "got: {err}");
}

#[test]
fn x1_nonsandbox_strict_file_keeps_runtime_code() {
    // Plain library strict (no sandbox) preserves the pinned E-RUNTIME.
    let src = "fn main() -> i32 { print(read_file(\"../evil\")) return 0 }";
    let err = run_with_flags(src, false, false).expect_err("strict must deny ..");
    assert!(err.starts_with("E-RUNTIME"), "got: {err}");
    assert!(!err.starts_with("E-SANDBOX"), "must not be sandbox code: {err}");
}

#[test]
fn x1_sandbox_tmp_file_allowed() {
    let _guard = SANDBOX_LOCK.lock().unwrap();
    klang::sandbox::set_sandbox(true);
    klang::runtime::set_file_guard_relaxed(false);
    let dir = std::env::temp_dir().join(format!("klang-x1-sbx-tmpok-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ok.txt");
    std::fs::write(&path, "sandbox-tmp-ok").unwrap();
    let ps = path.to_string_lossy().replace('\\', "/");
    let src = format!("fn main() -> i32 {{ print(read_file(\"{ps}\")) return 0 }}");
    let prog = check_ok(&src);
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("tmp allowed");
    assert_eq!(out, vec!["sandbox-tmp-ok".to_string()]);
    assert_eq!(v, 0);
    klang::sandbox::set_sandbox(false);
    klang::runtime::set_file_guard_relaxed(false);
    let _ = std::fs::remove_file(&path);
}

// ---- library: spawn/run denied in sandbox, attempted outside ----

#[test]
fn x1_sandbox_spawn_denied() {
    let src = "fn main() -> i32 { let r = spawn(\"echo\", [\"hi\"]) return 0 }";
    let err = run_with_flags(src, true, false).expect_err("sandbox must deny spawn");
    assert!(err.starts_with("E-SANDBOX-SPAWN"), "got: {err}");
}

#[test]
fn x1_sandbox_run_denied() {
    let src = "fn main() -> i32 { let c = run(\"echo\", [\"hi\"]) return 0 }";
    let err = run_with_flags(src, true, false).expect_err("sandbox must deny run");
    assert!(err.starts_with("E-SANDBOX-SPAWN"), "got: {err}");
}

#[test]
fn x1_sandbox_run_process_denied() {
    let src = "fn main() -> i32 { let r = run_process(\"echo\", [\"hi\"]) return 0 }";
    let err = run_with_flags(src, true, false).expect_err("sandbox must deny run_process");
    assert!(err.starts_with("E-SANDBOX-SPAWN"), "got: {err}");
}

#[test]
fn x1_nonsandbox_spawn_attempts_not_sandbox_denied() {
    // Outside sandbox, spawn tries the OS (echo exists; missing binary is
    // E-PROCESS-*, never E-SANDBOX-*).
    let _guard = SANDBOX_LOCK.lock().unwrap();
    klang::sandbox::set_sandbox(false);
    klang::runtime::set_file_guard_relaxed(false);
    let src = "fn main() -> i32 { let r = spawn(\"echo\", [\"hi\"]) return r[\"status\"] }";
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("echo runs");
    assert_eq!(v, 0);
    klang::sandbox::set_sandbox(false);
    klang::runtime::set_file_guard_relaxed(false);
}

// ---- library: network denied in sandbox ----

#[test]
fn x1_sandbox_http_get_denied() {
    let src = "fn main() -> i32 { let r = http_get(\"http://127.0.0.1:1/\") return 0 }";
    let err = run_with_flags(src, true, false).expect_err("sandbox must deny http");
    assert!(err.starts_with("E-SANDBOX-NETWORK"), "got: {err}");
}

#[test]
fn x1_sandbox_http_post_denied() {
    let src =
        "fn main() -> i32 { let r = http_post(\"http://127.0.0.1:1/\", \"b\", []) return 0 }";
    let err = run_with_flags(src, true, false).expect_err("sandbox must deny http post");
    assert!(err.starts_with("E-SANDBOX-NETWORK"), "got: {err}");
}

#[test]
fn x1_nonsandbox_http_attempts_not_sandbox_denied() {
    // Outside sandbox, a bad target is E-NET-*, never E-SANDBOX-* (no real
    // network needed: unroutable port fails fast on loopback).
    let _guard = SANDBOX_LOCK.lock().unwrap();
    klang::sandbox::set_sandbox(false);
    klang::runtime::set_file_guard_relaxed(false);
    let src = "fn main() -> i32 { let r = http_get(\"http://127.0.0.1:1/\") return 0 }";
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    let err =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    assert!(err.code.starts_with("E-NET-"), "got: {}", err.to_json());
    assert!(!err.code.starts_with("E-SANDBOX"), "got: {}", err.to_json());
    klang::sandbox::set_sandbox(false);
    klang::runtime::set_file_guard_relaxed(false);
}

#[test]
fn x1_sandbox_task_group_still_runs() {
    // Language task_group/spawn is structured concurrency, not OS threads:
    // sandbox leaves it alone.
    let src = "fn fetch(n: i32) -> i32 { return n * 2 }\nfn main() -> i32 async {\n    let total = 0\n    task_group {\n        let a = spawn fetch(20)\n        let b = spawn fetch(1)\n        total = await a + await b\n    }\n    return total\n}\n";
    let _guard = SANDBOX_LOCK.lock().unwrap();
    klang::sandbox::set_sandbox(true);
    klang::runtime::set_file_guard_relaxed(false);
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("tasks run");
    assert_eq!(v, 42);
    klang::sandbox::set_sandbox(false);
    klang::runtime::set_file_guard_relaxed(false);
}

// ---- CLI: both modes end to end ----

#[test]
fn x1_cli_sandbox_file_denied_but_plain_run_allows_dotdot() {
    let base = tmpdir("cli-file");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(base.join("file.txt"), "rel-ok").unwrap();
    let prog_src =
        "fn main() -> i32 {\n    print(read_file(\"../file.txt\"))\n    return 0\n}\n";
    let prog = write(&work, "rel.klang", prog_src);
    // Plain run: relaxed, succeeds.
    let (out, _err, code) = run_cli(&["run", &prog], &work);
    assert_eq!(code, 0, "plain run must allow ..: out={out}");
    assert_eq!(out, "rel-ok\n", "got: {out:?}");
    // Sandbox run: denied with E-SANDBOX-FILE.
    let (_out, err, code) = run_cli(&["run", "--sandbox", &prog], &work);
    assert_eq!(code, 1, "sandbox must fail: {err}");
    assert!(err.contains("E-SANDBOX-FILE"), "stderr:\n{err}");
}

#[test]
fn x1_cli_sandbox_spawn_denied_but_plain_run_spawns() {
    let dir = tmpdir("cli-spawn");
    let prog = write(
        &dir,
        "sp.klang",
        "fn main() -> i32 {\n    let r = spawn(\"echo\", [\"hi\"])\n    print(r[\"status\"])\n    return 0\n}\n",
    );
    let (out, _err, code) = run_cli(&["run", &prog], &dir);
    assert_eq!(code, 0, "plain run spawns: {out}");
    assert!(out.contains('0'), "got: {out:?}");
    let (_out, err, code) = run_cli(&["run", "--sandbox", &prog], &dir);
    assert_eq!(code, 1, "sandbox must deny: {err}");
    assert!(err.contains("E-SANDBOX-SPAWN"), "stderr:\n{err}");
}

#[test]
fn x1_cli_sandbox_network_denied() {
    let dir = tmpdir("cli-net");
    let prog = write(
        &dir,
        "net.klang",
        "fn main() -> i32 {\n    let r = http_get(\"http://127.0.0.1:1/\")\n    return 0\n}\n",
    );
    let (_out, err, code) = run_cli(&["run", "--sandbox", &prog], &dir);
    assert_eq!(code, 1, "sandbox must deny network: {err}");
    assert!(err.contains("E-SANDBOX-NETWORK"), "stderr:\n{err}");
}

#[test]
fn x1_cli_sandbox_tmp_file_allowed() {
    let dir = tmpdir("cli-tmp");
    let target = std::env::temp_dir().join(format!("klang-x1-cli-tmp-{}", std::process::id()));
    std::fs::create_dir_all(&target).unwrap();
    let f = target.join("ok.txt");
    std::fs::write(&f, "cli-tmp-ok").unwrap();
    let fs = f.to_string_lossy().replace('\\', "/");
    let prog = write(
        &dir,
        "t.klang",
        &format!("fn main() -> i32 {{\n    print(read_file(\"{fs}\"))\n    return 0\n}}\n"),
    );
    let (out, _err, code) = run_cli(&["run", "--sandbox", &prog], &dir);
    assert_eq!(code, 0, "tmp allowed in sandbox: {out}");
    assert_eq!(out, "cli-tmp-ok\n", "got: {out:?}");
}

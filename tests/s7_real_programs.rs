//! S7 dogfood — three real programs in `examples/real/`.
//!
//! Each program runs under the interpreter (`run` / `run_with_argv` or the
//! real `klang` binary with piped stdin). Fixtures live in
//! `examples/real/fixtures/`: `sample.log` (logsum), `shell_demo.txt`
//! (minishell stdin script), `organize_sample/` (organize template dir).
//! Organize/logsum file tests use real temp-dir copies so the capability
//! guard (`reject_unsafe_path`: absolute only inside `temp_dir()`) does
//! not fire and the repo fixtures stay untouched.

use std::collections::HashMap;
use std::io::Write;
use std::process::Stdio;

use klang::parser::Parser;

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn prog_path(name: &str) -> String {
    format!("{}/examples/real/{}", env!("CARGO_MANIFEST_DIR"), name)
}

fn fixture_path(rel: &str) -> String {
    format!(
        "{}/examples/real/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        rel
    )
}

fn tmp_base(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "klang-s7-{}-{}",
        tag,
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run the real binary with piped stdin (minishell reads `read_line()`).
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

/// Load a program file, check clean, lower to MIR.
fn load_mir(path: &str) -> klang::mir::MirModule {
    let src = std::fs::read_to_string(path).expect("program file exists");
    let mut p = Parser::new(&src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {path}"
    );
    klang::mir::lower(&prog)
}

fn run_prog_argv(
    prog: &str,
    argv: &[String],
) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let mir = load_mir(prog);
    let (r, out) = klang::runtime::run_with_argv(&mir, "main", &[], argv, &HashMap::new());
    r.map(|v| (v.as_int() as i32, out))
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        let s = src.join(&name);
        let d = dst.join(&name);
        if s.is_dir() {
            copy_dir_recursive(&s, &d);
        } else {
            std::fs::copy(&s, &d).unwrap();
        }
    }
}

fn fmt_clean(path: &str) {
    let src = std::fs::read_to_string(path).expect("exists");
    let out = klang::fmt::fmt_source(&src, path).expect("formats");
    assert_eq!(src, out, "fmt --check must pass for {path}");
}

// ---- minishell (CLI, piped stdin) ----

#[test]
fn s7_minishell_echo_and_failed_exit() {
    let prog = prog_path("minishell.klang");
    let (out, err, code) = run_cli_with_stdin(&["run", &prog], "echo hi\nfalse\nexit\n");
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("hi"), "echo output: {out:?}");
    assert!(
        out.contains("exit: 1"),
        "failed exit status printed: {out:?}"
    );
}

#[test]
fn s7_minishell_env_assign_and_read() {
    let prog = prog_path("minishell.klang");
    let (out, err, code) = run_cli_with_stdin(
        &["run", &prog],
        "S7_PROBE_VAR_XYZ=s7hello123\nenv S7_PROBE_VAR_XYZ\nexit\n",
    );
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert!(
        out.contains("s7hello123"),
        "NAME=value then env NAME: {out:?}"
    );
}

#[test]
fn s7_minishell_cd_pwd() {
    let prog = prog_path("minishell.klang");
    let (out, err, code) =
        run_cli_with_stdin(&["run", &prog], "pwd\ncd /tmp\npwd\nexit\n");
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("/tmp"), "cd /tmp then pwd shows /tmp: {out:?}");
}

#[test]
fn s7_minishell_quoting_missing_and_fixture() {
    let prog = prog_path("minishell.klang");
    let fixture = std::fs::read_to_string(fixture_path("shell_demo.txt")).unwrap();
    let (out, err, code) = run_cli_with_stdin(&["run", &prog], &fixture);
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("hello shell"), "fixture echo: {out:?}");
    assert!(
        out.contains("quoted arg test"),
        "quotes keep spaces: {out:?}"
    );
    assert!(
        out.contains("command not found: no_such_cmd_xyz_12345"),
        "missing binary: {out:?}"
    );
}

// ---- organize (library, temp-dir copies) ----

#[test]
fn s7_organize_dry_run_leaves_files() {
    let prog = prog_path("organize.klang");
    let base = tmp_base("org-dry");
    let work = base.join("work");
    let _ = std::fs::remove_dir_all(&work);
    copy_dir_recursive(
        &std::path::PathBuf::from(fixture_path("organize_sample")),
        &work,
    );
    let dir = work.to_string_lossy().replace('\\', "/");
    let argv = vec![dir.clone(), "--dry-run".to_string()];
    let (code, out) = run_prog_argv(&prog, &argv).expect("runs");
    assert_eq!(code, 0, "out: {out:?}");
    let joined = out.join("\n");
    assert!(joined.contains("would move"), "dry-run prints: {joined:?}");
    assert!(joined.contains("would move 5 files"), "{joined:?}");
    // Nothing moved: no subdirs, originals still present.
    assert!(work.join("a.txt").exists(), "dry-run leaves files");
    assert!(!work.join("txt").exists(), "no dirs created");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn s7_organize_real_run_moves_and_summary() {
    let prog = prog_path("organize.klang");
    let base = tmp_base("org-real");
    let work = base.join("work");
    let _ = std::fs::remove_dir_all(&work);
    copy_dir_recursive(
        &std::path::PathBuf::from(fixture_path("organize_sample")),
        &work,
    );
    let dir = work.to_string_lossy().replace('\\', "/");
    let argv = vec![dir.clone()];
    let (code, out) = run_prog_argv(&prog, &argv).expect("runs");
    assert_eq!(code, 0, "out: {out:?}");
    let joined = out.join("\n");
    assert!(joined.contains("moved 5 files"), "{joined:?}");
    assert!(joined.contains("txt: 2"), "{joined:?}");
    assert!(joined.contains("jpg: 1"), "JPG lowercased: {joined:?}");
    assert!(joined.contains("noext: 1"), "README -> noext: {joined:?}");
    assert!(work.join("txt").join("a.txt").exists());
    assert!(work.join("txt").join("c.txt").exists());
    assert!(work.join("md").join("b.md").exists());
    assert!(work.join("jpg").join("photo.JPG").exists());
    assert!(work.join("noext").join("README").exists());
    assert!(!work.join("a.txt").exists(), "original gone");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn s7_organize_missing_dir_reports_error() {
    let prog = prog_path("organize.klang");
    let missing = tmp_base("org-missing").join("nope-xyz-123");
    let _ = std::fs::remove_dir_all(&missing);
    let dir = missing.to_string_lossy().replace('\\', "/");
    let argv = vec![dir];
    let (code, out) = run_prog_argv(&prog, &argv).expect("runs (error is value, not diagnostic)");
    assert_eq!(code, 1, "out: {out:?}");
    assert!(
        out.join("\n").contains("not a directory"),
        "friendly error: {out:?}"
    );
}

#[test]
fn s7_organize_usage_no_args() {
    let prog = prog_path("organize.klang");
    let (code, out) = run_prog_argv(&prog, &[]).expect("runs");
    assert_eq!(code, 1);
    assert!(out.join("\n").contains("usage: organize"), "{out:?}");
}

// ---- logsum (library, temp copies) ----

fn write_temp_log(tag: &str, content: &str) -> String {
    let base = tmp_base(tag);
    let p = base.join("app.log");
    std::fs::write(&p, content).unwrap();
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn s7_logsum_counts() {
    let prog = prog_path("logsum.klang");
    let fixture = std::fs::read_to_string(fixture_path("sample.log")).unwrap();
    let path = write_temp_log("log-counts", &fixture);
    let (code, out) = run_prog_argv(&prog, &[path]).expect("runs");
    assert_eq!(code, 0, "out: {out:?}");
    assert!(out.contains(&"INFO: 4".to_string()), "{out:?}");
    assert!(out.contains(&"WARN: 1".to_string()), "{out:?}");
    assert!(out.contains(&"ERROR: 2".to_string()), "{out:?}");
}

#[test]
fn s7_logsum_top_and_timestamps() {
    let prog = prog_path("logsum.klang");
    let fixture = std::fs::read_to_string(fixture_path("sample.log")).unwrap();
    let path = write_temp_log("log-top", &fixture);
    let (code, out) = run_prog_argv(&prog, &[path]).expect("runs");
    assert_eq!(code, 0, "out: {out:?}");
    let joined = out.join("\n");
    assert!(joined.contains("first: 2026-10-11T10:00:00"), "{joined:?}");
    assert!(joined.contains("last: 2026-10-11T10:06:00"), "{joined:?}");
    assert!(joined.contains("request ok: 3"), "{joined:?}");
    assert!(joined.contains("disk full: 2"), "{joined:?}");
}

#[test]
fn s7_logsum_empty_file() {
    let prog = prog_path("logsum.klang");
    let path = write_temp_log("log-empty", "");
    let (code, out) = run_prog_argv(&prog, &[path]).expect("runs");
    assert_eq!(code, 0, "out: {out:?}");
    assert!(out.contains(&"INFO: 0".to_string()), "{out:?}");
    assert!(out.contains(&"WARN: 0".to_string()), "{out:?}");
    assert!(out.contains(&"ERROR: 0".to_string()), "{out:?}");
}

#[test]
fn s7_logsum_missing_file() {
    let prog = prog_path("logsum.klang");
    let missing = tmp_base("log-missing").join("nope-xyz-123.log");
    let _ = std::fs::remove_file(&missing);
    let path = missing.to_string_lossy().replace('\\', "/");
    let (code, out) = run_prog_argv(&prog, &[path]).expect("runs (error is value)");
    assert_eq!(code, 1, "out: {out:?}");
    let joined = out.join("\n");
    assert!(joined.contains("cannot read"), "{joined:?}");
    assert!(joined.contains("E-IO-NOT-FOUND"), "{joined:?}");
}

// ---- fmt must pass on all three ----

#[test]
fn s7_fmt_minishell_clean() {
    fmt_clean(&prog_path("minishell.klang"));
}

#[test]
fn s7_fmt_organize_clean() {
    fmt_clean(&prog_path("organize.klang"));
}

#[test]
fn s7_fmt_logsum_clean() {
    fmt_clean(&prog_path("logsum.klang"));
}

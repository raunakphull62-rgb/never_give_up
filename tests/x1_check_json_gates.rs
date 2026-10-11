//! X1 Step 1 — `klang check --json` CLI equivalent of `klang_check`.
//!
//! Covers the parity the MCP removal requires: `--json` prints one
//! `{"diagnostics":[...]}` object built from the same
//! `Diagnostic::to_json()` objects, follows imports (S2), respects
//! `--strict`/manifest, and accepts `--json` before or after the file.
//! Each test uses its own temp dir (pid + atomic counter).

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

static CTR: AtomicU64 = AtomicU64::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    let c = CTR.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("klang-x1-json-{tag}-{}-{c}", std::process::id()));
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

#[test]
fn x1_json_clean_is_empty_array_exit_0() {
    let dir = tmpdir("clean");
    let f = write(&dir, "ok.klang", "fn main() -> i32 { return 42 }\n");
    let (out, _err, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 0, "stdout:\n{out}");
    assert_eq!(out.trim(), "{\"diagnostics\":[]}", "got: {out:?}");
}

#[test]
fn x1_json_error_carries_code_exit_1() {
    let dir = tmpdir("err");
    let f = write(
        &dir,
        "bad.klang",
        "fn main() -> i32 { return nope_x1 + 1 }\n",
    );
    let (out, _err, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 1, "stdout:\n{out}");
    let t = out.trim();
    assert!(t.starts_with("{\"diagnostics\":["), "got: {t:?}");
    assert!(t.contains("\"code\""), "got: {t:?}");
    assert!(t.contains("E-UNDEFINED"), "got: {t:?}");
    assert!(t.ends_with("]}"), "got: {t:?}");
}

#[test]
fn x1_json_flag_order_before_and_after_file() {
    let dir = tmpdir("order");
    let f = write(&dir, "ok.klang", "fn main() -> i32 { return 1 }\n");
    let (a, _, ca) = run_cli(&["check", "--json", &f], &dir);
    let (b, _, cb) = run_cli(&["check", &f, "--json"], &dir);
    assert_eq!((ca, a.trim()), (0, "{\"diagnostics\":[]}"));
    assert_eq!((cb, b.trim()), (0, "{\"diagnostics\":[]}"));
}

#[test]
fn x1_json_follows_local_import_clean() {
    let dir = tmpdir("import-clean");
    write(&dir, "lib.klang", "fn libdouble(n: i32) -> i32 { return n * 2 }\n");
    let entry_src = "import \"lib.klang\"\nfn main() -> i32 { return libdouble(21) }\n";
    let f = write(&dir, "main.klang", entry_src);
    let (out, _err, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 0, "stdout:\n{out}");
    assert_eq!(out.trim(), "{\"diagnostics\":[]}", "got: {out:?}");
}

#[test]
fn x1_json_error_inside_imported_file_names_that_file() {
    let dir = tmpdir("import-err");
    let lib = write(
        &dir,
        "lib.klang",
        "fn libbadx1() -> i32 { return \"hi\" - 1 }\n",
    );
    let entry_src = "import \"lib.klang\"\nfn main() -> i32 { return libbadx1() }\n";
    let f = write(&dir, "main.klang", entry_src);
    let (out, _err, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 1, "stdout:\n{out}");
    assert!(out.contains("E-TYPE"), "got: {out:?}");
    assert!(out.contains(&lib), "diagnostic must name imported file: {out:?}");
}

#[test]
fn x1_json_unresolved_import_is_single_diagnostic() {
    let dir = tmpdir("unresolved");
    let entry_src = "import \"nosuchx1.klang\"\nfn main() -> i32 { return 0 }\n";
    let f = write(&dir, "main.klang", entry_src);
    let (out, _err, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 1, "stdout:\n{out}");
    assert!(out.contains("nosuchx1.klang"), "got: {out:?}");
    assert!(out.contains("\"diagnostics\""), "got: {out:?}");
}

#[test]
fn x1_json_strict_flag_rejects_int_condition() {
    let dir = tmpdir("strict-flag");
    let src = "fn main() -> i32 { if 1 { return 1 } else { return 0 } }\n";
    let f = write(&dir, "int.klang", src);
    // Default: clean.
    let (out, _, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 0, "default must be clean: {out}");
    // Strict flag: E-TYPE via JSON.
    let (out, _, code) = run_cli(&["check", "--json", "--strict", &f], &dir);
    assert_eq!(code, 1, "strict must fail: {out}");
    assert!(out.contains("E-TYPE"), "got: {out:?}");
}

#[test]
fn x1_json_strict_manifest_rejects_int_condition() {
    let dir = tmpdir("strict-manifest");
    write(
        &dir,
        "klang.toml",
        "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[project]\nstrict = true\n",
    );
    let src = "fn main() -> i32 { if 1 { return 1 } else { return 0 } }\n";
    let f = write(&dir, "int.klang", src);
    let (out, _, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 1, "manifest strict must fail: {out}");
    assert!(out.contains("E-TYPE"), "got: {out:?}");
}

#[test]
fn x1_json_parse_error_has_e_parse() {
    let dir = tmpdir("parse");
    let f = write(&dir, "broken.klang", "fn main() -> i32 { return \n");
    let (out, _err, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 1, "stdout:\n{out}");
    assert!(out.contains("E-PARSE"), "got: {out:?}");
}

#[test]
fn x1_json_output_is_single_line_object() {
    let dir = tmpdir("shape");
    let f = write(&dir, "ok.klang", "fn main() -> i32 { return 0 }\n");
    let (out, _err, code) = run_cli(&["check", "--json", &f], &dir);
    assert_eq!(code, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 1, "one JSON object, got: {out:?}");
    assert!(lines[0].starts_with("{\"diagnostics\":"), "got: {out:?}");
}

#[test]
fn x1_human_check_still_works_without_json() {
    let dir = tmpdir("human");
    let f = write(&dir, "ok.klang", "fn main() -> i32 { return 0 }\n");
    let (out, _err, code) = run_cli(&["check", &f], &dir);
    assert_eq!(code, 0, "stdout:\n{out}");
    assert!(out.contains("check: OK"), "got: {out:?}");
}

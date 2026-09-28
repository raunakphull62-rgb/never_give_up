//! QUIET-1 — `klang run --quiet` / `-q` shows only program output.
//!
//! Option (a) from the PRD: an additive flag, not a flipped default.
//! `scripts/verify_docs.py` parses the verbose `run` shape (`check: OK`,
//! `print:` lines), so quiet-by-default would have broken the docs
//! harness; the default output is byte-identical to before. Quiet
//! contract pinned here: raw `print` lines (no `print: ` prefix, no
//! `file:`/`parse:`/`check:`/`mir:` preamble) plus the minimal
//! `run {entry}() = {v}` result line. Failures still show the full
//! diagnostic in both modes.

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn run_cli(args: &[&str]) -> (String, i32) {
    let output = std::process::Command::new(klang_bin())
        .args(args)
        .output()
        .expect("klang binary runs");
    let code = output.status.code().unwrap_or(-1);
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (combined, code)
}

fn hello_prog(dir: &std::path::Path) -> String {
    let path = dir.join("hello.klang");
    std::fs::write(&path, "fn main() -> i32 {\n    print(\"hello\")\n    return 0\n}\n").unwrap();
    path.to_string_lossy().into_owned()
}

fn cli_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("klang-quiet-gates");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn quiet_run_prints_only_program_output() {
    // The PRD's exact case: flag after the file, default entry.
    let dir = cli_dir();
    let prog = hello_prog(&dir);
    let (out, code) = run_cli(&["run", &prog, "main", "--quiet"]);
    assert_eq!(code, 0, "got:\n{out}");
    assert_eq!(out, "hello\nrun main() = 0\n", "exact quiet bytes, got:\n{out}");
}

#[test]
fn quiet_flag_before_file_and_short_form() {
    // Flag position must not matter; `-q` is the short form.
    let dir = cli_dir();
    let prog = hello_prog(&dir);
    let (out, code) = run_cli(&["run", "--quiet", &prog]);
    assert_eq!(code, 0, "got:\n{out}");
    assert_eq!(out, "hello\nrun main() = 0\n", "got:\n{out}");
    let (out, code) = run_cli(&["run", &prog, "-q"]);
    assert_eq!(code, 0, "got:\n{out}");
    assert_eq!(out, "hello\nrun main() = 0\n", "got:\n{out}");
}

#[test]
fn default_run_keeps_full_dump() {
    // Anyone not using the flag sees exactly what they saw before.
    let dir = cli_dir();
    let prog = hello_prog(&dir);
    let (out, code) = run_cli(&["run", &prog, "main"]);
    assert_eq!(code, 0, "got:\n{out}");
    for marker in [
        "file:",
        "parse: OK",
        "check: OK",
        "mir:",
        "print: hello",
        "run main() = 0",
    ] {
        assert!(out.contains(marker), "missing `{marker}` in:\n{out}");
    }
}

#[test]
fn quiet_failure_still_shows_diagnostic() {
    // Quiet suppresses chatter, never errors: prior prints (raw) plus
    // the full failure diagnostic must both appear.
    let dir = cli_dir();
    let prog = dir.join("qfail.klang");
    let missing = std::env::temp_dir().join("klang-quiet-missing-xyz.txt");
    let _ = std::fs::remove_file(&missing);
    let missing = missing.to_string_lossy().replace('\\', "/");
    std::fs::write(
        &prog,
        format!(
            "fn main() -> i32 {{ print(\"almost\") print(read_file(\"{missing}\")) return 0 }}"
        ),
    )
    .unwrap();
    let (out, code) = run_cli(&["run", "--quiet", &prog.to_string_lossy(), "main"]);
    assert_eq!(code, 1, "got:\n{out}");
    let p = out.find("almost\n").expect("raw print shown:\n{out}");
    let f = out.find("run: FAIL").expect("FAIL shown:\n{out}");
    assert!(p < f, "print before FAIL:\n{out}");
    assert!(out.contains("E-IO-NOT-FOUND"), "diagnostic shown:\n{out}");
    assert!(!out.contains("parse: OK"), "no preamble in quiet:\n{out}");
    assert!(!out.contains("print: almost"), "no prefix in quiet:\n{out}");
}

#[test]
fn quiet_custom_entry_names_result_line() {
    let dir = cli_dir();
    let prog = dir.join("qentry.klang");
    std::fs::write(
        &prog,
        "fn other() -> i32 {\n    print(\"from other\")\n    return 7\n}\nfn main() -> i32 {\n    return 0\n}\n",
    )
    .unwrap();
    let (out, code) = run_cli(&["run", "--quiet", &prog.to_string_lossy(), "other"]);
    assert_eq!(code, 0, "got:\n{out}");
    assert_eq!(out, "from other\nrun other() = 7\n", "got:\n{out}");
}

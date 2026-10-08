//! RUN-DEFAULT-1 — `klang run` prints only program output by default.
//!
//! Default: raw `print` lines on stdout, nothing else; the entry return
//! value becomes the process exit code (never printed). `--verbose`/`-v`
//! restores the old full dump (`file:`/`parse:`/`check:`/`mir:`,
//! `print:`-prefixed lines, `run e() = v`). `--quiet`/`-q` is a no-op
//! alias. Failures print program output on stdout and the structured
//! diagnostic on stderr, exiting 1.

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn run_cli(args: &[&str]) -> (String, String, i32) {
    let output = std::process::Command::new(klang_bin())
        .args(args)
        .output()
        .expect("klang binary runs");
    let code = output.status.code().unwrap_or(-1);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        code,
    )
}

fn hello_prog(dir: &std::path::Path) -> String {
    let path = dir.join("hello.klang");
    std::fs::write(&path, "fn main() -> i32 {\n    print(\"hello\")\n    return 0\n}\n").unwrap();
    path.to_string_lossy().into_owned()
}

fn cli_dir() -> std::path::PathBuf {
    // Unique dir per call: tests in this file run in parallel, so the old
    // shared dir + shared `hello.klang` filename raced (one test truncated
    // the file while another ran it -> exit 1). The pid isolates parallel
    // test binaries; the counter isolates threads within one binary.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "klang-quiet-gates-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn default_run_prints_only_program_output() {
    // PRD exact case: stdout is exactly `hello\n`, nothing else.
    let dir = cli_dir();
    let prog = hello_prog(&dir);
    let (out, err, code) = run_cli(&["run", &prog, "main"]);
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "hello\n", "exact default bytes, got:\n{out}");
    assert!(err.is_empty(), "no stderr on success:\n{err}");
}

#[test]
fn quiet_flag_is_noop_alias() {
    // `--quiet`/`-q` accepted, changes nothing.
    let dir = cli_dir();
    let prog = hello_prog(&dir);
    let (out, err, code) = run_cli(&["run", &prog, "main", "--quiet"]);
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "hello\n", "got:\n{out}");
    let (out, _, code) = run_cli(&["run", "--quiet", &prog]);
    assert_eq!(code, 0);
    assert_eq!(out, "hello\n", "got:\n{out}");
    let (out, _, code) = run_cli(&["run", &prog, "-q"]);
    assert_eq!(code, 0);
    assert_eq!(out, "hello\n", "got:\n{out}");
}

#[test]
fn verbose_restores_full_dump() {
    // Old output, byte-shape unchanged, in both flag positions + short form.
    let dir = cli_dir();
    let prog = hello_prog(&dir);
    for args in [
        vec!["run", "--verbose", prog.as_str()],
        vec!["run", prog.as_str(), "--verbose"],
        vec!["run", "-v", prog.as_str()],
    ] {
        let (out, _, code) = run_cli(&args);
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
}

#[test]
fn exit_code_is_return_value() {
    // return 42 exits 42; return 0 exits 0. Value never printed.
    let dir = cli_dir();
    let prog = dir.join("ret42.klang");
    std::fs::write(
        &prog,
        "fn main() -> i32 {\n    print(\"hi\")\n    return 42\n}\n",
    )
    .unwrap();
    let (out, err, code) = run_cli(&["run", &prog.to_string_lossy(), "main"]);
    assert_eq!(code, 42, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "hi\n", "only program output, got:\n{out}");
    assert!(err.is_empty());
    let prog0 = dir.join("ret0.klang");
    std::fs::write(&prog0, "fn main() -> i32 {\n    return 0\n}\n").unwrap();
    let (out, _, code) = run_cli(&["run", &prog0.to_string_lossy()]);
    assert_eq!(code, 0);
    assert_eq!(out, "", "no output and no result line, got:\n{out}");
}

#[test]
fn exit_code_wraps_at_256_and_negatives() {
    // Linux low-8-bits wrapping: 256 -> 0, -1 -> 255. Never clamped.
    let dir = cli_dir();
    let prog = dir.join("ret256.klang");
    std::fs::write(&prog, "fn main() -> i32 {\n    return 256\n}\n").unwrap();
    let (_, _, code) = run_cli(&["run", &prog.to_string_lossy()]);
    assert_eq!(code, 0, "256 wraps to 0");
    let prog = dir.join("retneg1.klang");
    std::fs::write(&prog, "fn main() -> i32 {\n    return 0 - 1\n}\n").unwrap();
    let (_, _, code) = run_cli(&["run", &prog.to_string_lossy()]);
    assert_eq!(code, 255, "-1 wraps to 255");
}

#[test]
fn failure_prints_diagnostic_to_stderr_exit_1() {
    // Program output on stdout, structured diagnostic on stderr.
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
    let (out, err, code) = run_cli(&["run", &prog.to_string_lossy(), "main"]);
    assert_eq!(code, 1, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "almost\n", "raw print on stdout, got:\n{out}");
    assert!(err.contains("run: FAIL"), "FAIL on stderr:\n{err}");
    assert!(err.contains("E-IO-NOT-FOUND"), "diagnostic on stderr:\n{err}");
    assert!(!out.contains("E-IO-NOT-FOUND"), "no diagnostic on stdout:\n{out}");
}

#[test]
fn stdout_empty_when_failing_with_no_output() {
    // `klang run bad.klang 2>/dev/null` prints nothing.
    let dir = cli_dir();
    let prog = dir.join("badempty.klang");
    let missing = std::env::temp_dir().join("klang-quiet-missing-xyz2.txt");
    let _ = std::fs::remove_file(&missing);
    let missing = missing.to_string_lossy().replace('\\', "/");
    std::fs::write(
        &prog,
        format!("fn main() -> i32 {{ print(read_file(\"{missing}\")) return 0 }}"),
    )
    .unwrap();
    let (out, err, code) = run_cli(&["run", &prog.to_string_lossy()]);
    assert_eq!(code, 1);
    assert_eq!(out, "", "stdout empty, got:\n{out}");
    assert!(err.contains("E-IO-NOT-FOUND"), "stderr:\n{err}");
}

#[test]
fn verbose_custom_entry_names_result_line() {
    // Result line only exists in verbose mode now.
    let dir = cli_dir();
    let prog = dir.join("qentry.klang");
    std::fs::write(
        &prog,
        "fn other() -> i32 {\n    print(\"from other\")\n    return 7\n}\nfn main() -> i32 {\n    return 0\n}\n",
    )
    .unwrap();
    let (out, _, code) = run_cli(&["run", "--verbose", &prog.to_string_lossy(), "other"]);
    assert_eq!(code, 7, "exit is return value even in verbose");
    assert!(out.contains("from other\n") || out.contains("print: from other\n"), "got:\n{out}");
    assert!(out.contains("run other() = 7"), "got:\n{out}");
    let (out, _, code) = run_cli(&["run", &prog.to_string_lossy(), "other"]);
    assert_eq!(code, 7);
    assert_eq!(out, "from other\n", "default has no result line, got:\n{out}");
}

#[test]
fn run_v2_default_and_verbose() {
    // Same contract on the v2 path.
    let dir = cli_dir();
    let prog = dir.join("hi.v2");
    std::fs::write(
        &prog,
        "fn main() -> i32 {\n  print(\"v2hi\")\n  return 5\n}\n",
    )
    .unwrap();
    let (out, err, code) = run_cli(&["run-v2", &prog.to_string_lossy()]);
    assert_eq!(code, 5, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "v2hi\n", "got:\n{out}");
    let (out, _, code) = run_cli(&["run-v2", "--verbose", &prog.to_string_lossy()]);
    assert_eq!(code, 5);
    assert!(out.contains("print: v2hi"), "got:\n{out}");
    assert!(out.contains("run main() = 5"), "got:\n{out}");
}

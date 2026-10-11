//! X1 Step 3 — `klang mcp` was removed.
//!
//! `klang mcp` (any args) must print the removal pointer and exit 2:
//! `E-CLI-REMOVED: \`klang mcp\` was removed; use \`klang check --json\`,
//! \`klang run --sandbox\`, or \`klang fmt\``.

use std::io::Write;
use std::process::Stdio;

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn run_mcp(args: &[&str]) -> (String, String, i32) {
    let mut full = vec!["mcp"];
    full.extend(args);
    let mut child = std::process::Command::new(klang_bin())
        .args(&full)
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

const REMOVED: &str =
    "E-CLI-REMOVED: `klang mcp` was removed; use `klang check --json`, `klang run --sandbox`, or `klang fmt`";

#[test]
fn x1_mcp_bare_prints_removed_and_exits_2() {
    let (out, err, code) = run_mcp(&[]);
    assert_eq!(code, 2, "stdout:\n{out}\nstderr:\n{err}");
    assert!(err.contains(REMOVED), "stderr:\n{err}");
}

#[test]
fn x1_mcp_with_args_still_prints_removed_and_exits_2() {
    let (out, err, code) = run_mcp(&["--help"]);
    assert_eq!(code, 2, "stdout:\n{out}\nstderr:\n{err}");
    assert!(err.contains(REMOVED), "stderr:\n{err}");
}

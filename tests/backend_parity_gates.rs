//! D6 differential-testing harness (corpus runner), Phase 3a Part 1.
//!
//! Runs a corpus of Klang programs on every backend — the v1 interpreter
//! (`run`, the reference), the JIT (`run --backend-jit`), and the v2
//! tree-walker (`run-v2`) — and compares exit outcome, stdout, and the
//! normalized stderr diagnostic code.
//!
//! Corpus = `tests/parity/*.klang` (targeted divergence probes) +
//! runnable `examples/*.klang` + the 26 `stdlib-packages/*/*_test.klang`
//! self-tests (minus the network-dependent `http_live_test.klang`).
//! The in-tree inline diff suites (`tests/jit_int.rs`,
//! `tests/shortcircuit_gates.rs`) pin the same agreement in-process and run
//! in this same `cargo test` gate.
//!
//! Known, accepted divergences live in `tests/parity/allowlist.toml` with
//! the expected behavior per backend and a `reason` saying why. The gate
//! fails on any divergence NOT on the list, on any allowlisted entry whose
//! actual behavior changed shape, AND when an allowlisted divergence
//! disappears (actual behavior now equals the v1 reference) — so the list
//! stays honest and the D6 fallible-JIT work can flip entries to agreement
//! in its own PR.
//!
//! No backend behavior is changed here; every subprocess runs with a
//! timeout and in a temp CWD (a JIT trap must never take down the harness,
//! and a core dump must never land in the repo).

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

// ---------------------------------------------------------------------------
// Allowlist file (minimal TOML subset)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Expect {
    programs: Vec<String>,
    backend: String,
    outcome: String,
    exit: Option<i32>,
    stdout: String,
    stderr_contains: String,
    stdin: String,
    reason: String,
    line: usize,
}

fn unescape(s: &str, line: usize) -> Result<String, String> {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                return Err(format!("line {line}: bad escape `\\{other}`"));
            }
            None => return Err(format!("line {line}: trailing backslash")),
        }
    }
    Ok(out)
}

fn parse_quoted(v: &str, line: usize) -> Result<String, String> {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        unescape(&v[1..v.len() - 1], line)
    } else {
        Err(format!("line {line}: expected `\"string\"`, got `{v}`"))
    }
}

fn parse_allowlist(text: &str) -> Result<Vec<Expect>, String> {
    let mut entries: Vec<Expect> = Vec::new();
    let mut cur: Option<Expect> = None;
    for (idx, raw) in text.lines().enumerate() {
        let line = idx + 1;
        let s = raw.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        if s == "[[entry]]" {
            if let Some(e) = cur.take() {
                entries.push(e);
            }
            cur = Some(Expect {
                programs: Vec::new(),
                backend: String::new(),
                outcome: String::new(),
                exit: None,
                stdout: String::new(),
                stderr_contains: String::new(),
                stdin: String::new(),
                reason: String::new(),
                line,
            });
            continue;
        }
        let e = cur
            .as_mut()
            .ok_or_else(|| format!("line {line}: key=value outside `[[entry]]`"))?;
        let (k, v) = s
            .split_once('=')
            .ok_or_else(|| format!("line {line}: expected `key = value`"))?;
        let (k, v) = (k.trim(), v.trim());
        match k {
            "programs" => {
                if !v.starts_with('[') || !v.ends_with(']') {
                    return Err(format!("line {line}: `programs` must be `[\"a\", ...]`"));
                }
                let inner = &v[1..v.len() - 1];
                let mut ps = Vec::new();
                for part in inner.split(',') {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    ps.push(parse_quoted(part, line)?);
                }
                if ps.is_empty() {
                    return Err(format!("line {line}: `programs` is empty"));
                }
                e.programs = ps;
            }
            "backend" => e.backend = parse_quoted(v, line)?,
            "outcome" => e.outcome = parse_quoted(v, line)?,
            "exit" => {
                e.exit = Some(
                    v.parse::<i32>()
                        .map_err(|_| format!("line {line}: `exit` must be an integer"))?,
                )
            }
            "stdout" => e.stdout = parse_quoted(v, line)?,
            "stderr_contains" => e.stderr_contains = parse_quoted(v, line)?,
            "stdin" => e.stdin = parse_quoted(v, line)?,
            "reason" => e.reason = parse_quoted(v, line)?,
            _ => return Err(format!("line {line}: unknown key `{k}`")),
        }
    }
    if let Some(e) = cur.take() {
        entries.push(e);
    }
    // Validation: complete, known-valued, non-duplicate, on-disk.
    let root = repo_root();
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    let mut stdin_for: HashMap<String, (String, usize)> = HashMap::new();
    for e in &entries {
        if e.programs.is_empty() {
            return Err(format!("line {}: missing `programs`", e.line));
        }
        if e.backend != "jit" && e.backend != "v2" {
            return Err(format!(
                "line {}: `backend` must be \"jit\" or \"v2\", got {:?}",
                e.line, e.backend
            ));
        }
        if e.outcome != "crash" && e.outcome != "exit" {
            return Err(format!(
                "line {}: `outcome` must be \"crash\" or \"exit\", got {:?}",
                e.line, e.outcome
            ));
        }
        if e.outcome == "exit" && e.exit.is_none() {
            return Err(format!("line {}: `outcome = \"exit\"` needs `exit = <n>`", e.line));
        }
        if e.outcome == "crash" && e.exit.is_some() {
            return Err(format!("line {}: `outcome = \"crash\"` takes no `exit`", e.line));
        }
        if e.reason.is_empty() {
            return Err(format!("line {}: missing `reason` (every entry says why)", e.line));
        }
        for p in &e.programs {
            if !root.join(p).is_file() {
                return Err(format!("line {}: program `{p}` does not exist", e.line));
            }
            if let Some(prev) = seen.insert((p.clone(), e.backend.clone()), e.line) {
                return Err(format!(
                    "line {}: duplicate entry for ({p}, {}), first at line {prev}",
                    e.line, e.backend
                ));
            }
            if let Some((prev_stdin, prev_line)) = stdin_for.get(p) {
                if *prev_stdin != e.stdin {
                    return Err(format!(
                        "line {}: conflicting `stdin` for `{p}` (also at line {prev_line})",
                        e.line
                    ));
                }
            } else {
                stdin_for.insert(p.clone(), (e.stdin.clone(), e.line));
            }
        }
    }
    Ok(entries)
}

fn load_allowlist() -> Vec<Expect> {
    let text = std::fs::read_to_string(repo_root().join("tests/parity/allowlist.toml"))
        .expect("tests/parity/allowlist.toml readable");
    parse_allowlist(&text).expect("allowlist parses and validates")
}

// ---------------------------------------------------------------------------
// Subprocess runner (crash-safe: timeout + temp CWD, never in-process)
// ---------------------------------------------------------------------------

const RUN_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Crash { stdout: Vec<u8> },
    Exit { code: i32, stdout: Vec<u8>, stderr: Vec<u8> },
    Timeout,
}

fn run_backend(program: &str, backend: &str, stdin: &str, workdir: &std::path::Path) -> Outcome {
    let path = repo_root().join(program);
    let mut cmd = std::process::Command::new(klang_bin());
    match backend {
        "v1" => {
            cmd.arg("run").arg(&path);
        }
        "jit" => {
            cmd.arg("run").arg("--backend-jit").arg(&path);
        }
        "v2" => {
            cmd.arg("run-v2").arg(&path);
        }
        _ => panic!("unknown backend {backend}"),
    }
    cmd.current_dir(workdir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().expect("klang spawns");
    if !stdin.is_empty() {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(stdin.as_bytes())
            .expect("stdin writes");
        // `take()` drops the pipe handle here, delivering EOF.
    } else {
        // No input: close the pipe immediately so `read_line()` sees EOF
        // instead of blocking forever.
        drop(child.stdin.take());
    }
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait works") {
            Some(status) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                use std::io::Read;
                if let Some(mut o) = child.stdout.take() {
                    o.read_to_end(&mut stdout).ok();
                }
                if let Some(mut e) = child.stderr.take() {
                    e.read_to_end(&mut stderr).ok();
                }
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if status.signal().is_some() {
                        return Outcome::Crash { stdout };
                    }
                }
                match status.code() {
                    Some(code) => return Outcome::Exit { code, stdout, stderr },
                    None => return Outcome::Crash { stdout },
                }
            }
            None => {
                if start.elapsed() > RUN_TIMEOUT {
                    child.kill().ok();
                    child.wait().ok();
                    return Outcome::Timeout;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Normalized stderr class for cross-backend comparison. Full diagnostic
/// JSON differs legitimately (`file` fields: "runtime" vs "input.v2"),
/// so agreement compares the code, not the bytes.
fn err_code(stderr: &[u8], exit: i32) -> String {
    let s = String::from_utf8_lossy(stderr);
    if let Some(pos) = s.find("\"code\": \"") {
        let rest = &s[pos + 9..];
        if let Some(end) = rest.find('"') {
            return rest[..end].to_string();
        }
    }
    if s.contains("JIT-FAIL") {
        return "JIT-FAIL".to_string();
    }
    if exit == 0 {
        return "OK".to_string();
    }
    "OTHER-FAIL".to_string()
}

fn same_outcome(a: &Outcome, b: &Outcome) -> bool {
    match (a, b) {
        (Outcome::Crash { stdout: x }, Outcome::Crash { stdout: y }) => x == y,
        (
            Outcome::Exit { code: x, stdout: xs, stderr: xe },
            Outcome::Exit { code: y, stdout: ys, stderr: ye },
        ) => x == y && xs == ys && err_code(xe, *x) == err_code(ye, *y),
        _ => false,
    }
}

fn describe(o: &Outcome) -> String {
    match o {
        Outcome::Crash { stdout } => {
            format!("crash (signal), stdout={:?}", String::from_utf8_lossy(stdout))
        }
        Outcome::Exit { code, stdout, stderr } => format!(
            "exit {code}, stdout={:?}, errcode={}",
            String::from_utf8_lossy(stdout),
            err_code(stderr, *code)
        ),
        Outcome::Timeout => "TIMEOUT after 60s".to_string(),
    }
}

fn entry_matches(e: &Expect, actual: &Outcome) -> bool {
    match actual {
        Outcome::Timeout => false,
        Outcome::Crash { stdout } => e.outcome == "crash" && stdout.is_empty(),
        Outcome::Exit { code, stdout, stderr } => {
            if e.outcome != "exit" || Some(*code) != e.exit {
                return false;
            }
            if stdout != e.stdout.as_bytes() {
                return false;
            }
            e.stderr_contains.is_empty()
                || String::from_utf8_lossy(stderr).contains(&e.stderr_contains)
        }
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "klang-parity-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run one program on all three backends; enforce agreement-or-allowlist.
/// Pushes human-readable failures into `problems`, prints pins otherwise.
fn check_program(
    program: &str,
    entries: &[Expect],
    workdir: &std::path::Path,
    problems: &mut Vec<String>,
) {
    let stdin = entries
        .iter()
        .find(|e| e.programs.iter().any(|p| p == program))
        .map(|e| e.stdin.as_str())
        .unwrap_or("");
    let ref_out = run_backend(program, "v1", stdin, workdir);
    if ref_out == Outcome::Timeout {
        problems.push(format!("{program}: v1 reference run TIMED OUT (60s)"));
        return;
    }
    for backend in ["jit", "v2"] {
        let entry = entries.iter().find(|e| {
            e.backend == backend && e.programs.iter().any(|p| p == program)
        });
        let actual = run_backend(program, backend, stdin, workdir);
        match entry {
            None => {
                if !same_outcome(&ref_out, &actual) {
                    problems.push(format!(
                        "{program} on {backend}: DIVERGENCE NOT ALLOWLISTED\n  v1 ref: {}\n  {backend}:   {}\n  (if accepted, add it to tests/parity/allowlist.toml with a reason; never \"fix\" this by editing the program)",
                        describe(&ref_out),
                        describe(&actual)
                    ));
                } else {
                    println!("agree  {program} [{backend}] ({})", describe(&actual));
                }
            }
            Some(e) => {
                if same_outcome(&ref_out, &actual) {
                    problems.push(format!(
                        "{program} on {backend}: ALLOWLISTED DIVERGENCE DISAPPEARED (now agrees with v1: {}) — remove the tests/parity/allowlist.toml entry (line {}) instead of letting the list lie",
                        describe(&actual),
                        e.line
                    ));
                } else if !entry_matches(e, &actual) {
                    problems.push(format!(
                        "{program} on {backend}: ALLOWLISTED DIVERGENCE CHANGED SHAPE (line {})\n  expected: outcome={} exit={:?} stdout={:?} stderr_contains={:?}\n  v1 ref:   {}\n  {backend}:     {}\n  reason was: {}",
                        e.line,
                        e.outcome,
                        e.exit,
                        e.stdout,
                        e.stderr_contains,
                        describe(&ref_out),
                        describe(&actual),
                        e.reason
                    ));
                } else {
                    println!("pinned {program} [{backend}] ({})", describe(&actual));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scope
// ---------------------------------------------------------------------------

/// Runnable examples. Deliberately NOT auto-discovered: lib.klang is a
/// library file with no main entry point, and time_http_integration.klang
/// needs outbound internet (covered by scripts/live-tests.sh instead).
fn example_programs() -> Vec<String> {
    for skip in ["examples/lib.klang", "examples/time_http_integration.klang"] {
        assert!(
            repo_root().join(skip).is_file(),
            "{skip} still exists as a documented skip"
        );
    }
    [
        "examples/app.klang",
        "examples/simple.klang",
        "examples/full.klang",
        "examples/eval.klang",
        "examples/format.klang",
        "examples/stdlib_demo.klang",
        "examples/osio_integration.klang",
        "examples/try_catch.klang",
        "examples/calculator.klang",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn corpus_programs() -> Vec<String> {
    let mut out = Vec::new();
    let dir = repo_root().join("tests/parity");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("tests/parity readable")
        .map(|e| e.expect("dir entry").file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".klang"))
        .collect();
    names.sort();
    assert!(
        !names.is_empty(),
        "tests/parity holds the targeted corpus (*.klang)"
    );
    for n in names {
        out.push(format!("tests/parity/{n}"));
    }
    out
}

fn stdlib_programs() -> Vec<(String, String)> {
    // (program, package) for every self-test except the network-dependent
    // http_live_test.klang.
    let mut out = Vec::new();
    let mut pkgs: Vec<String> = std::fs::read_dir(repo_root().join("stdlib-packages"))
        .expect("stdlib-packages readable")
        .map(|e| e.expect("dir entry").file_name().to_string_lossy().to_string())
        .collect();
    pkgs.sort();
    for pkg in pkgs {
        let dir = repo_root().join("stdlib-packages").join(&pkg);
        if !dir.is_dir() || pkg == "BUILTINS.md" {
            continue;
        }
        let test = format!("stdlib-packages/{pkg}/{pkg}_test.klang");
        if repo_root().join(&test).is_file() {
            out.push((test, pkg));
        }
    }
    assert_eq!(out.len(), 26, "all 26 package self-tests in scope");
    assert!(
        repo_root()
            .join("stdlib-packages/http/http_live_test.klang")
            .is_file(),
        "http_live_test.klang still exists as a documented network skip"
    );
    out
}

// ---------------------------------------------------------------------------
// Gates
// ---------------------------------------------------------------------------

#[test]
fn parity_allowlist_is_wellformed() {
    let entries = load_allowlist();
    assert!(!entries.is_empty(), "allowlist is non-empty");
    println!("allowlist OK: {} entries", entries.len());
}

#[test]
fn parity_corpus_agrees_or_allowlisted() {
    let entries = load_allowlist();
    let workdir = scratch_dir("corpus");
    let mut problems = Vec::new();
    for prog in corpus_programs() {
        check_program(&prog, &entries, &workdir, &mut problems);
    }
    std::fs::remove_dir_all(&workdir).ok();
    assert!(
        problems.is_empty(),
        "\n{} parity corpus problem(s):\n{}\n",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn parity_examples_agree_or_allowlisted() {
    let entries = load_allowlist();
    let workdir = scratch_dir("examples");
    let mut problems = Vec::new();
    for prog in example_programs() {
        check_program(&prog, &entries, &workdir, &mut problems);
    }
    std::fs::remove_dir_all(&workdir).ok();
    assert!(
        problems.is_empty(),
        "\n{} example parity problem(s):\n{}\n",
        problems.len(),
        problems.join("\n")
    );
}

#[test]
fn parity_stdlib_selftests_pass_and_diverge_loudly() {
    let entries = load_allowlist();
    let workdir = scratch_dir("stdlib");
    let mut problems = Vec::new();
    for (prog, pkg) in stdlib_programs() {
        // v1 reference bar: the self-test prints "<pkg>: OK" and exits 0.
        let stdin = entries
            .iter()
            .find(|e| e.programs.iter().any(|p| p == &prog))
            .map(|e| e.stdin.as_str())
            .unwrap_or("");
        let ref_out = run_backend(&prog, "v1", stdin, &workdir);
        match &ref_out {
            Outcome::Exit { code: 0, stdout, .. }
                if String::from_utf8_lossy(stdout).contains(&format!("{pkg}: OK")) =>
            {
                println!("stdlib {pkg}: OK on v1")
            }
            other => {
                problems.push(format!(
                    "{prog} on v1: SELF-TEST DID NOT PASS ({}) — expected exit 0 with `{pkg}: OK`",
                    describe(other)
                ));
                continue;
            }
        }
        // JIT/v2 must diverge LOUDLY exactly as allowlisted (no agreement
        // expected: every self-test uses strings/lists/maps/IO/imports).
        for backend in ["jit", "v2"] {
            let entry = entries.iter().find(|e| {
                e.backend == backend && e.programs.iter().any(|p| p == &prog)
            });
            let actual = run_backend(&prog, backend, stdin, &workdir);
            match entry {
                None => problems.push(format!(
                    "{prog} on {backend}: DIVERGENCE NOT ALLOWLISTED (v1 passes; {backend}: {})",
                    describe(&actual)
                )),
                Some(e) => {
                    if same_outcome(&ref_out, &actual) {
                        problems.push(format!(
                            "{prog} on {backend}: ALLOWLISTED DIVERGENCE DISAPPEARED (now passes like v1!) — remove the allowlist.toml entry (line {})",
                            e.line
                        ));
                    } else if !entry_matches(e, &actual) {
                        problems.push(format!(
                            "{prog} on {backend}: ALLOWLISTED DIVERGENCE CHANGED SHAPE (line {}): expected outcome={} exit={:?} stderr_contains={:?}, got {}",
                            e.line,
                            e.outcome,
                            e.exit,
                            e.stderr_contains,
                            describe(&actual)
                        ));
                    } else {
                        println!("pinned {prog} [{backend}] ({})", describe(&actual));
                    }
                }
            }
        }
    }
    std::fs::remove_dir_all(&workdir).ok();
    assert!(
        problems.is_empty(),
        "\n{} stdlib parity problem(s):\n{}\n",
        problems.len(),
        problems.join("\n")
    );
}

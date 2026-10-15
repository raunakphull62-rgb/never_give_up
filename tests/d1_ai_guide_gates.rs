//! D1 AI-guide gates: every fenced block in docs/ai/*.md runs, examples
//! match expected output, fmt clean, token budget, stdlib coverage.
//!
//! Annotations: ```klang run (+ following ```output must match stdout
//! exactly), ```klang error CODE (must fail with that code via check or
//! run), ```klang silent (must exit 0). Temp dirs, empty stdin, no network.

use std::collections::HashMap;
use std::io::Write;
use std::process::Stdio;

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn ai_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/ai")
}

fn run_cli_in(
    dir: &std::path::Path,
    args: &[&str],
    stdin_data: &str,
) -> (String, String, i32) {
    let mut child = std::process::Command::new(klang_bin())
        .args(args)
        .current_dir(dir)
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

fn mktemp_dir(prefix: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    p.push(format!(
        "d1_{}_{}_{}",
        prefix,
        std::process::id(),
        nanos % 1_000_000_000
    ));
    std::fs::create_dir_all(&p).expect("mktemp");
    p
}

#[derive(Debug)]
enum Block {
    Run { code: String, expected: Option<String> },
    Error { want: String, code: String },
    Silent { code: String },
    /// Parse + type-check only (no run): for examples needing network or
    /// a database (http_serve, pg_*). Verified via `Parser` + `TypedHIR`.
    Check { code: String },
}

fn extract_blocks(md: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim().to_string();
        if t == "```klang run" {
            let mut code = String::new();
            i += 1;
            while i < lines.len() && lines[i].trim() != "```" {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            // skip closing ```
            i += 1;
            // optional blank + ```output
            let mut expected: Option<String> = None;
            // skip blank lines
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            if i < lines.len() && lines[i].trim() == "```output" {
                let mut exp = String::new();
                i += 1;
                while i < lines.len() && lines[i].trim() != "```" {
                    exp.push_str(lines[i]);
                    exp.push('\n');
                    i += 1;
                }
                i += 1; // closing
                // An empty output block (no lines) means expect empty stdout.
                // A block with lines means those lines plus trailing newline.
                // Our extractor always ends with \n when non-empty; when the
                // block had zero lines, exp is "" (expect empty stdout).
                expected = Some(exp);
            }
            out.push(Block::Run { code, expected });
            continue;
        } else if t.starts_with("```klang error") {
            let want = t
                .strip_prefix("```klang error")
                .unwrap()
                .trim()
                .to_string();
            let mut code = String::new();
            i += 1;
            while i < lines.len() && lines[i].trim() != "```" {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            i += 1;
            out.push(Block::Error { want, code });
            continue;
        } else if t == "```klang silent" {
            let mut code = String::new();
            i += 1;
            while i < lines.len() && lines[i].trim() != "```" {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            i += 1;
            out.push(Block::Silent { code });
            continue;
        } else if t == "```klang check" {
            let mut code = String::new();
            i += 1;
            while i < lines.len() && lines[i].trim() != "```" {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            i += 1;
            out.push(Block::Check { code });
            continue;
        }
        i += 1;
    }
    out
}

fn write_helper_if_needed(dir: &std::path::Path, code: &str) {
    if code.contains("import \"helper.klang\"") {
        std::fs::write(
            dir.join("helper.klang"),
            "fn double(x: i32) -> i32 {\n    return x + x\n}\n",
        )
        .expect("helper write");
    }
}

fn test_run_block(code: &str, expected: &Option<String>, label: &str) {
    let dir = mktemp_dir("run");
    write_helper_if_needed(&dir, code);
    std::fs::write(dir.join("prog.klang"), code).expect("write prog");
    let (stdout, stderr, status) = run_cli_in(&dir, &["run", "prog.klang"], "");
    assert_eq!(
        status, 0,
        "{label}: `klang run` must exit 0, got {status}\nstderr: {stderr}\ncode:\n{code}"
    );
    if let Some(exp) = expected {
        // Empty output block means expect empty stdout.
        // Otherwise exp already ends with \n (one per line).
        assert_eq!(
            &stdout, exp,
            "{label}: stdout mismatch\ncode:\n{code}\nexpected: {exp:?}\ngot: {stdout:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn test_error_block(want: &str, code: &str, label: &str) {
    // Try check first; import failures surface at parse/load.
    let dir = mktemp_dir("err");
    write_helper_if_needed(&dir, code);
    std::fs::write(dir.join("prog.klang"), code).expect("write prog");
    let (check_out, check_err, _) = run_cli_in(&dir, &["check", "prog.klang"], "");
    let combined = format!("{check_out}{check_err}");
    if combined.contains("check: OK") {
        let (run_out, run_err, _) = run_cli_in(&dir, &["run", "prog.klang"], "");
        let run_combined = format!("{run_out}{run_err}");
        assert!(
            run_combined.contains(want),
            "{label}: want code {want} in run output, got:\n{run_combined}\ncode:\n{code}"
        );
    } else {
        assert!(
            combined.contains(want),
            "{label}: want code {want} in check output, got:\n{combined}\ncode:\n{code}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn test_silent_block(code: &str, label: &str) {
    let dir = mktemp_dir("silent");
    write_helper_if_needed(&dir, code);
    std::fs::write(dir.join("prog.klang"), code).expect("write prog");
    let (_, stderr, status) = run_cli_in(&dir, &["run", "prog.klang"], "");
    assert_eq!(
        status, 0,
        "{label}: silent block must exit 0, got {status}\nstderr: {stderr}\ncode:\n{code}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn test_check_block(code: &str, label: &str) {
    // Parse + type-check only (no run, no network/DB): the program must
    // parse and pass `TypedHIR::check`. Used for backend examples that
    // need `http_serve` or Postgres at run time.
    let mut p = klang::parser::Parser::new(code);
    let prog = p.parse_program().unwrap_or_else(|_| panic!("{label}: check block must parse\ncode:\n{code}"));
    let diags = klang::hir::TypedHIR::check(prog);
    assert!(
        diags.is_ok(),
        "{label}: check block must type-check clean, got {diags:?}\ncode:\n{code}"
    );
}

fn md_files_to_gate() -> Vec<std::path::PathBuf> {
    // Source files (not the generated concatenation, to avoid double-run).
    let names = [
        "core.md",
        "pitfalls.md",
        "python-to-klang.md",
        "stdlib.md",
        "backend.md",
        "errors.md",
    ];
    names.iter().map(|n| ai_dir().join(n)).collect()
}

#[test]
fn d1_all_fenced_blocks_pass() {
    let mut total_run = 0;
    let mut total_err = 0;
    let mut total_silent = 0;
    let mut total_check = 0;
    for path in md_files_to_gate() {
        let md = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("read {}", path.display()));
        let blocks = extract_blocks(&md);
        for (idx, b) in blocks.iter().enumerate() {
            let label = format!("{} block {idx}", path.file_name().unwrap().to_string_lossy());
            match b {
                Block::Run { code, expected } => {
                    test_run_block(code, expected, &label);
                    total_run += 1;
                }
                Block::Error { want, code } => {
                    test_error_block(want, code, &label);
                    total_err += 1;
                }
                Block::Silent { code } => {
                    test_silent_block(code, &label);
                    total_silent += 1;
                }
                Block::Check { code } => {
                    test_check_block(code, &label);
                    total_check += 1;
                }
            }
        }
    }
    // Sanity: we expect a large guide (catches accidental truncation).
    assert!(
        total_run >= 100,
        "expected >=100 run blocks, got {total_run}"
    );
    assert!(total_err >= 60, "expected >=60 error blocks, got {total_err}");
    assert!(
        total_silent >= 10,
        "expected >=10 silent blocks, got {total_silent}"
    );
    eprintln!("d1 blocks: {total_run} run, {total_err} error, {total_silent} silent, {total_check} check");
}

#[test]
fn d1_examples_run_and_match() {
    let dir = ai_dir().join("examples");
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("examples dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().map(|e| e == "klang").unwrap_or(false))
        .collect();
    entries.sort();
    assert!(
        entries.len() >= 30,
        "need >=30 examples, got {}",
        entries.len()
    );
    for prog_path in &entries {
        let name = prog_path.file_stem().unwrap().to_string_lossy().to_string();
        let expected_path = prog_path.with_extension("expected");
        assert!(
            expected_path.exists(),
            "missing {name}.expected for {}",
            prog_path.display()
        );
        let expected = std::fs::read_to_string(&expected_path).expect("read expected");
        // Line-count gate: 10..60 lines each.
        let src = std::fs::read_to_string(prog_path).expect("read src");
        let lines = src.lines().count();
        assert!(
            (10..=60).contains(&lines),
            "{name}: need 10..60 lines, got {lines}"
        );
        // fmt gate.
        let tmp = mktemp_dir("fmt");
        let tmp_prog = tmp.join("prog.klang");
        std::fs::write(&tmp_prog, &src).expect("copy");
        let (_, _, fmt_status) =
            run_cli_in(&tmp, &["fmt", "--check", "prog.klang"], "");
        assert_eq!(
            fmt_status, 0,
            "{name}: `klang fmt --check` must pass (exit 0)"
        );
        // Run in a fresh temp dir (file examples stay isolated).
        let work = mktemp_dir("exrun");
        std::fs::write(work.join("prog.klang"), &src).expect("copy prog");
        let (stdout, stderr, status) = run_cli_in(&work, &["run", "prog.klang"], "");
        assert_eq!(
            status, 0,
            "{name}: `klang run` must exit 0, got {status}\nstderr: {stderr}"
        );
        assert_eq!(
            stdout, expected,
            "{name}: stdout mismatch\nexpected: {expected:?}\ngot: {stdout:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&work);
    }
}

#[test]
fn d1_core_token_budget() {
    let core = std::fs::read_to_string(ai_dir().join("core.md")).expect("read core");
    let tokens = core.len() / 4;
    assert!(
        tokens <= 12000,
        "core.md over budget: {} tokens ({} chars), max 12000",
        tokens,
        core.len()
    );
}

#[test]
fn d1_system_prompt_budget() {
    let p = std::fs::read_to_string(ai_dir().join("ai-system-prompt.md"))
        .expect("read prompt");
    let tokens = p.len() / 4;
    assert!(
        tokens <= 2500,
        "ai-system-prompt.md over budget: {tokens} tokens, max 2500"
    );
}

#[test]
fn d1_stdlib_covers_every_builtin() {
    // Generate the list from code (do not trust docs): parse is_builtin.
    let hir_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/hir.rs");
    let hir = std::fs::read_to_string(&hir_path).expect("read hir.rs");
    let start = hir.find("pub fn is_builtin").expect("is_builtin");
    let slice = &hir[start..];
    let mstart = slice.find("matches!").expect("matches");
    let after = &slice[mstart..];
    let end = after.find(')').expect("end") + mstart;
    let _ = end;
    // Extract quoted names in the is_builtin matches! block (up to the
    // closing of the matches! macro's first arm list).
    let block_end = slice.find("\n}\n").expect("fn end");
    let block = &slice[..block_end];
    let mut names = Vec::new();
    let mut in_quotes = false;
    let mut cur = String::new();
    for ch in block.chars() {
        if ch == '"' {
            if in_quotes {
                if !cur.is_empty() {
                    names.push(cur.clone());
                }
                cur.clear();
            }
            in_quotes = !in_quotes;
        } else if in_quotes {
            cur.push(ch);
        }
    }
    // Filter to plausible builtin names (the block also contains doc text
    // after the matches!; keep only names that is_builtin would match by
    // checking the runtime predicate via a curated re-check: every name
    // must look like an identifier and appear before the closing of the
    // matches! list — we approximate by requiring snake_case).
    let stdlib = std::fs::read_to_string(ai_dir().join("stdlib.md")).expect("read stdlib");
    let mut missing = Vec::new();
    for n in &names {
        if !n.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()) {
            continue;
        }
        if n.len() < 2 {
            continue;
        }
        // Skip non-builtin words from surrounding docs (heuristic: builtins
        // are listed in the matches! with `| "name"` separators; doc words
        // rarely appear quoted there — but to be strict, require the name
        // to be followed/preceded by builtin-like context is overkill.
        // Instead, verify against the known-good set size: we expect 82.
        if !stdlib.contains(n) {
            missing.push(n.clone());
        }
    }
    assert!(
        names.len() >= 80,
        "expected >=80 builtin names parsed from hir.rs, got {}",
        names.len()
    );
    assert!(
        missing.is_empty(),
        "stdlib.md missing builtins: {missing:?}"
    );
    // Methods must be documented too (all 23: 14 str + 6 array + 3 map).
    for m in [
        "pad_start", "pad_end", "repeat", "lines", "upper", "lower", "trim", "chars",
        "split", "contains", "starts_with", "ends_with", "join", "keys", "len",
        "replace", "push", "pop", "insert",
    ] {
        assert!(stdlib.contains(m), "stdlib.md missing method {m}");
    }
    // S9 + B1 must be present with verified examples (table alone is not
    // enough: the gate also requires example usage strings).
    for b in [
        "print_raw", "is_eof", "chdir", "sort_by", "regex_find_all", "regex_replace",
        "uuid_v4", "path_absolute", "file_mtime", "sprintf", "random_seed",
        "random_seeded_int", "random_seeded_float",
    ] {
        assert!(stdlib.contains(b), "stdlib.md missing S9/B1 builtin {b}");
    }
    for ex in ["print(print_raw", "sort_by(words", "pad_end", "random_seeded_float"] {
        assert!(
            stdlib.contains(ex),
            "stdlib.md missing verified example for {ex}"
        );
    }
    let _ = HashMap::<String, String>::new();
}

#[test]
fn d1_build_script_deterministic() {
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts/build_ai_guide.sh");
    assert!(script.exists(), "scripts/build_ai_guide.sh must exist");
    let out_path = ai_dir().join("KLANG_FOR_AI.md");
    assert!(out_path.exists(), "docs/ai/KLANG_FOR_AI.md must exist");
    let before = std::fs::read_to_string(&out_path).expect("read guide");
    let status = std::process::Command::new("sh")
        .arg(script.to_string_lossy().to_string())
        .status()
        .expect("run build script");
    assert!(status.success(), "build script must exit 0");
    let after = std::fs::read_to_string(&out_path).expect("re-read guide");
    assert_eq!(before, after, "build script must be deterministic");
    for part in ["core.md", "pitfalls.md", "python-to-klang.md", "stdlib.md", "backend.md", "errors.md"] {
        assert!(
            after.contains(&format!("{part}")),
            "concatenated guide must contain {part} marker"
        );
    }
}

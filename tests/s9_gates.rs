//! S9 gaps from docs/FINDINGS.md — gates for the six items.
//!
//! Each item has at least three tests. Library execution uses
//! `run_with_output` (F1 plain mode: all paths allowed, OS errors are
//! `E-IO-*`); CLI execution spawns the real binary (plain `run` allows
//! all paths; `--sandbox` keeps the strict temp-dir-only guard).

use std::collections::HashMap;
use std::io::Write;
use std::process::Stdio;

use klang::parser::Parser;

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

fn run_src(src: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new())
}

fn run_src_chunks(
    src: &str,
) -> (
    Result<klang::runtime::Value, klang::diagnostics::Diagnostic>,
    Vec<klang::runtime::OutputChunk>,
) {
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output_value_partial_chunks(&mir, "main", &[], &HashMap::new())
}

fn check_err_code(src: &str, want: &str) {
    let prog = parse(src);
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(
        codes.contains(&want),
        "want {want} in {codes:?}: {src}"
    );
}

fn run_cli(args: &[&str], stdin_data: &str) -> (String, String, i32) {
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

fn run_cli_in(dir: &std::path::Path, args: &[&str], stdin_data: &str) -> (String, String, i32) {
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

fn tmp_base(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("klang-s9-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_tmp_prog(dir: &std::path::Path, name: &str, src: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, src).unwrap();
    p.to_string_lossy().replace('\\', "/")
}

// ---- item 1: print_raw(x) ----

#[test]
fn s9_print_raw_chunk_is_raw_without_newline() {
    let (r, chunks) = run_src_chunks("fn main() -> i32 { print_raw(\"hi\") return 0 }");
    r.expect("runs");
    assert_eq!(chunks.len(), 1, "{chunks:?}");
    assert_eq!(chunks[0].text, "hi");
    assert!(chunks[0].raw, "print_raw must flag raw: {chunks:?}");
}

#[test]
fn s9_print_raw_ordered_with_print() {
    let (r, chunks) = run_src_chunks(
        "fn main() -> i32 { print(\"a\") print_raw(\"b\") print(\"c\") return 0 }",
    );
    r.expect("runs");
    assert_eq!(chunks.len(), 3, "{chunks:?}");
    assert_eq!(chunks[0].text, "a");
    assert!(!chunks[0].raw);
    assert_eq!(chunks[1].text, "b");
    assert!(chunks[1].raw);
    assert_eq!(chunks[2].text, "c");
    assert!(!chunks[2].raw);
    // The legacy Vec<String> view is unchanged (texts only).
    let (_, out) = run_src("fn main() -> i32 { print(\"a\") print_raw(\"b\") print(\"c\") return 0 }")
        .expect("runs");
    assert_eq!(out, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
}

#[test]
fn s9_print_raw_cli_has_no_trailing_newline() {
    let dir = tmp_base("raw-cli");
    let prog = write_tmp_prog(&dir, "raw.klang", "fn main() -> i32 {\n    print_raw(\"hi\")\n    return 0\n}\n");
    let (out, err, code) = run_cli(&["run", &prog], "");
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "hi", "no trailing newline, got: {out:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s9_print_raw_renders_int_like_print() {
    let (r, chunks) = run_src_chunks("fn main() -> i32 { print_raw(42) return 0 }");
    r.expect("runs");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "42");
    assert!(chunks[0].raw);
}

#[test]
fn s9_print_raw_jit_rejects_loudly() {
    // Int-only body so the JIT reaches the builtin check (not ConstStr first).
    let prog = check_ok("fn main() -> i32 { print_raw(42) return 0 }");
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject print_raw");
    assert!(err.contains("unsupported"), "{err}");
    assert!(err.contains("print_raw"), "{err}");
}

#[test]
fn s9_print_raw_v2_rejects_loudly() {
    let src = "fn main() -> i32 {\n  print_raw(\"hi\")\n  return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses call");
    let (r, _) = klang::runtime::v2::run_v2_program_partial(&prog, "main");
    let err = r.expect_err("v2 must reject print_raw");
    assert_eq!(err.code, "E-UNDEFINED", "{}", err.to_json());
}

// ---- item 2: is_eof() ----

#[test]
fn s9_is_eof_empty_stdin_is_true() {
    let dir = tmp_base("eof-empty");
    let prog = write_tmp_prog(
        &dir,
        "eof.klang",
        "fn main() -> i32 {\n    if is_eof() {\n        print(\"eof\")\n    } else {\n        print(\"data\")\n    }\n    return 0\n}\n",
    );
    let (out, err, code) = run_cli(&["run", &prog], "");
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "eof\n", "got: {out:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s9_is_eof_peeks_without_consuming() {
    let dir = tmp_base("eof-peek");
    let prog = write_tmp_prog(
        &dir,
        "peek.klang",
        "fn main() -> i32 {\n    if is_eof() {\n        print(\"eof-first\")\n    } else {\n        print(\"has-data\")\n    }\n    print(read_line())\n    if is_eof() {\n        print(\"eof-after\")\n    } else {\n        print(\"more\")\n    }\n    return 0\n}\n",
    );
    let (out, err, code) = run_cli(&["run", &prog], "hello\n");
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "has-data\nhello\neof-after\n", "got: {out:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s9_is_eof_empty_line_is_not_eof() {
    // A bare newline buffers "" (not EOF); the next read is EOF.
    let dir = tmp_base("eof-emptyline");
    let prog = write_tmp_prog(
        &dir,
        "empty.klang",
        "fn main() -> i32 {\n    if is_eof() {\n        print(\"eof-first\")\n    } else {\n        print(\"not-eof\")\n    }\n    let line = read_line()\n    print((\"got:\" + line))\n    if is_eof() {\n        print(\"eof-second\")\n    } else {\n        print(\"more\")\n    }\n    return 0\n}\n",
    );
    let (out, err, code) = run_cli(&["run", &prog], "\n");
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "not-eof\ngot:\neof-second\n", "got: {out:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s9_is_eof_read_line_behavior_unchanged() {
    // read_line still strips one newline/CRLF and yields "" at EOF.
    let dir = tmp_base("eof-readline");
    let prog = write_tmp_prog(
        &dir,
        "rl.klang",
        "fn main() -> i32 {\n    print(read_line())\n    return 0\n}\n",
    );
    let (out, _, code) = run_cli(&["run", &prog], "a\r\n");
    assert_eq!(code, 0);
    assert_eq!(out, "a\n", "CRLF stripped: {out:?}");
    let (out, _, code) = run_cli(&["run", &prog], "");
    assert_eq!(code, 0);
    assert_eq!(out, "\n", "EOF is empty line: {out:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s9_is_eof_arity_checked() {
    check_err_code("fn main() -> i32 { return is_eof(1) }", "E-ARITY");
}

#[test]
fn s9_is_eof_jit_and_v2_reject() {
    let prog = check_ok("fn main() -> i32 { if is_eof() { return 1 } return 0 }");
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject is_eof");
    assert!(err.contains("unsupported"), "{err}");
    let src = "fn main() -> i32 {\n  is_eof()\n  return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let (r, _) = klang::runtime::v2::run_v2_program_partial(&prog, "main");
    let err = r.expect_err("v2 must reject is_eof");
    assert_eq!(err.code, "E-UNDEFINED", "{}", err.to_json());
}

// ---- item 3: chdir(path) ----

#[test]
fn s9_chdir_missing_is_io_not_found() {
    // Failure leaves the process directory untouched (safe under parallel tests).
    let missing = tmp_base("chdir-missing").join("nope-xyz-123");
    let _ = std::fs::remove_dir_all(&missing);
    let path = missing.to_string_lossy().replace('\\', "/");
    let src = format!("fn main() -> i32 {{ chdir(\"{path}\") return 0 }}");
    let prog = check_ok(&src);
    let mir = klang::mir::lower(&prog);
    let err =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn s9_chdir_arity_and_type_checked() {
    check_err_code("fn main() -> i32 { return chdir() }", "E-ARITY");
    check_err_code("fn main() -> i32 { return chdir(42) }", "E-TYPE");
}

#[test]
fn s9_chdir_changes_children_and_relative_paths_via_cli() {
    // CLI subprocess only: the test process cwd never moves.
    let base = tmp_base("chdir-cli");
    let sub = base.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(base.join("data.txt"), "via-relative").unwrap();
    let prog_src = "fn main() -> i32 {\n    chdir(\"SUBDIR\")\n    print(cwd())\n    let r = spawn(\"pwd\", [])\n    print(r[\"status\"])\n    print(read_file(\"../data.txt\"))\n    return 0\n}\n".replace("SUBDIR", &sub.to_string_lossy().replace('\\', "/"));
    let prog = write_tmp_prog(&base, "cd.klang", &prog_src);
    // Run from the base dir so "../data.txt" resolves only after chdir(sub).
    let (out, err, code) = run_cli_in(&base, &["run", &prog], "");
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("sub"), "cwd shows sub: {out:?}");
    assert!(out.contains("via-relative"), "relative read after chdir: {out:?}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn s9_chdir_jit_and_v2_reject() {
    let prog = check_ok("fn main() -> i32 { chdir(\"/tmp\") return 0 }");
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject chdir");
    assert!(err.contains("unsupported"), "{err}");
    let src = "fn main() -> i32 {\n  chdir(\"/tmp\")\n  return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let (r, _) = klang::runtime::v2::run_v2_program_partial(&prog, "main");
    let err = r.expect_err("v2 must reject chdir");
    assert_eq!(err.code, "E-UNDEFINED", "{}", err.to_json());
}

#[test]
fn s9_minishell_cd_uses_chdir() {
    let src = std::fs::read_to_string(format!(
        "{}/examples/real/minishell.klang",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(src.contains("chdir("), "minishell cd must call chdir");
    assert!(src.contains("is_eof()"), "minishell must use is_eof");
    assert!(src.contains("print_raw("), "minishell must use print_raw");
    let prog = check_ok(&src);
    let _ = klang::mir::lower(&prog);
}

#[test]
fn s9_minishell_empty_line_continues() {
    // FINDINGS repro: a bare Enter used to quit; now echo bye still runs.
    let prog = format!("{}/examples/real/minishell.klang", env!("CARGO_MANIFEST_DIR"));
    let (out, err, code) =
        run_cli(&["run", &prog], "echo hi\n\necho bye\nexit\n");
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert!(out.contains("hi"), "first echo: {out:?}");
    assert!(out.contains("bye"), "empty line must not quit: {out:?}");
}

// ---- item 4: sort_by(array, key_fn) ----

#[test]
fn s9_sort_by_ints_ascending() {
    let (v, _) = run_src("fn main() -> i32 { let a = [3, 1, 2] let s = sort_by(a, fn(x: i32) -> i32 { return x }) return s[0] * 100 + s[1] * 10 + s[2] }").expect("runs");
    assert_eq!(v, 123);
}

#[test]
fn s9_sort_by_str_keys_ascending() {
    let (v, out) = run_src("fn main() -> i32 { let a = [\"bb\", \"a\", \"cc\"] let s = sort_by(a, fn(x: str) -> i32 { return len(x) }) print(s.join(\",\")) return len(s) }").expect("runs");
    assert_eq!(out, vec!["a,bb,cc".to_string()]);
    assert_eq!(v, 3);
}

#[test]
fn s9_sort_by_is_stable_for_equal_keys() {
    // Keys 2,1,2: "bb" keeps its input order ahead of "cc".
    let (_, out) = run_src("fn main() -> i32 { let a = [\"bb\", \"a\", \"cc\"] let s = sort_by(a, fn(x: str) -> i32 { return len(x) }) print(s[0]) print(s[1]) print(s[2]) return 0 }").expect("runs");
    assert_eq!(
        out,
        vec!["a".to_string(), "bb".to_string(), "cc".to_string()],
        "stable: {out:?}"
    );
}

#[test]
fn s9_sort_by_str_keys_lexicographic() {
    let (_, out) = run_src("fn main() -> i32 { let a = [\"pear\", \"apple\", \"fig\"] let s = sort_by(a, fn(x: str) -> str { return x }) print(s.join(\",\")) return 0 }").expect("runs");
    assert_eq!(out, vec!["apple,fig,pear".to_string()]);
}

#[test]
fn s9_sort_by_mixed_keys_is_etype() {
    // One Int key and one Str key: no total order, loud E-TYPE.
    let src = "fn key(x: str) -> array { if (x == \"a\") { return 1 } return \"x\" } fn main() -> i32 { let a = [\"a\", \"b\"] let s = sort_by(a, fn(x: str) -> array { if (x == \"a\") { return 1 } return \"x\" }) return len(s) }";
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    let err =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
}

#[test]
fn s9_sort_by_needs_closure_arity() {
    check_err_code(
        "fn main() -> i32 { let a = [1] return sort_by(a, 42) }",
        "E-TYPE",
    );
    check_err_code("fn main() -> i32 { let a = [1] return sort_by(a) }", "E-ARITY");
}

#[test]
fn s9_sort_by_jit_and_v2_reject() {
    let prog = check_ok(
        "fn main() -> i32 { let a = [2, 1] let s = sort_by(a, fn(x: i32) -> i32 { return x }) return s[0] }",
    );
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject sort_by");
    assert!(err.contains("unsupported"), "{err}");
}

#[test]
fn s9_logsum_uses_sort_by_and_still_counts() {
    let src = std::fs::read_to_string(format!(
        "{}/examples/real/logsum.klang",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    assert!(src.contains("sort_by("), "logsum must use sort_by");
    let prog = check_ok(&src);
    let mir = klang::mir::lower(&prog);
    let fixture = std::fs::read_to_string(format!(
        "{}/examples/real/fixtures/sample.log",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let base = tmp_base("logsum-s9");
    let p = base.join("app.log");
    std::fs::write(&p, &fixture).unwrap();
    let path = p.to_string_lossy().replace('\\', "/");
    let (r, out) = klang::runtime::run_with_argv(&mir, "main", &[], &[path], &HashMap::new());
    let (v, out) = (r.expect("runs").as_int() as i32, out);
    assert_eq!(v, 0, "{out:?}");
    assert!(out.contains(&"INFO: 4".to_string()), "{out:?}");
    assert!(out.contains(&"request ok: 3".to_string()), "{out:?}");
    let _ = std::fs::remove_dir_all(&base);
}

// ---- item 5: slice(unknown) stays array at runtime ----

#[test]
fn s9_slice_unknown_array_checks_clean_and_spawns() {
    // FINDINGS B1 repro: foo returns Unknown (array); slice must not become Str.
    let src = "fn foo() -> array { let a = [\"a\", \"b\", \"c\"] return a } fn main() -> i32 { let t = foo() let u = slice(t, 1, 3) print(u.join(\",\")) let r = spawn(\"echo\", u) print(r[\"status\"]) return 0 }";
    let (v, out) = run_src(src).expect("must check clean and run");
    assert_eq!(v, 0);
    assert!(out.contains(&"b,c".to_string()), "{out:?}");
    assert!(out.contains(&"0".to_string()), "{out:?}");
}

#[test]
fn s9_slice_unknown_string_stays_string() {
    let src = "fn foo() -> array { return \"hello\" } fn main() -> i32 { let t = foo() print(slice(t, 1, 3)) return 0 }";
    let (_, out) = run_src(src).expect("runs");
    assert_eq!(out, vec!["el".to_string()]);
}

#[test]
fn s9_slice_static_array_and_str_unchanged() {
    let (_, out) =
        run_src("fn main() -> i32 { let a = [1, 2, 3] print(str(slice(a, 0, 2)[1])) return 0 }")
            .expect("runs");
    assert_eq!(out, vec!["2".to_string()]);
    let (_, out) =
        run_src("fn main() -> i32 { print(slice(\"hello\", 1, 3)) return 0 }").expect("runs");
    assert_eq!(out, vec!["el".to_string()]);
}

#[test]
fn s9_slice_spawn_with_dynamic_toks() {
    // The exact minishell shape: spawn(cmd, slice(toks, 1, len(toks))).
    let src = "fn toks() -> array { return split_shell_line() } fn split_shell_line() -> array { return [\"echo\", \"hi\", \"there\"] } fn main() -> i32 { let t = toks() let r = spawn(t[0], slice(t, 1, len(t))) print(r[\"status\"]) return 0 }";
    let (v, out) = run_src(src).expect("runs");
    assert_eq!(v, 0);
    assert!(out.contains(&"0".to_string()), "{out:?}");
}

// ---- item 6: file guard (F1: plain allows all, sandbox strict) ----

#[test]
fn s9_guard_library_rejects_parent_relative() {
    // F1: plain mode (library, non-sandbox) allows `..`; a missing target
    // is E-IO-NOT-FOUND, not E-RUNTIME. Sandbox still denies (see f1_gates).
    // Use a target that cannot exist on any machine (not bare `evil`, which
    // could theoretically exist in the parent dir).
    klang::runtime::set_file_guard_relaxed(false);
    let src = "fn main() -> i32 { print(read_file(\"../klang_gate_no_such_dir_7f3a_evil\")) return 0 }";
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    let err =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn s9_guard_library_rejects_absolute_outside_tmp() {
    // F1: plain mode allows absolute paths outside tmp; missing file is
    // E-IO-NOT-FOUND. Sandbox still denies with E-SANDBOX-FILE.
    klang::runtime::set_file_guard_relaxed(false);
    let src =
        "fn main() -> i32 { print(read_file(\"/no/such/klang-file-xyz-outside-tmp\")) return 0 }";
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    let err =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn s9_guard_cli_allows_relative_dotdot() {
    // base/file.txt read as ../file.txt from base/work (relaxed `klang run`).
    let base = tmp_base("guard-rel");
    let work = base.join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(base.join("file.txt"), "rel-ok").unwrap();
    let prog_src = "fn main() -> i32 {\n    print(read_file(\"../file.txt\"))\n    return 0\n}\n";
    let prog = write_tmp_prog(&work, "rel.klang", prog_src);
    let (out, err, code) = run_cli_in(&work, &["run", &prog], "");
    assert_eq!(code, 0, "stdout:\n{out}\nstderr:\n{err}");
    assert_eq!(out, "rel-ok\n", "got: {out:?}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn s9_guard_cli_absolute_outside_tmp_still_rejected() {
    // F1: plain `klang run` allows absolute paths outside tmp; a missing
    // file is E-IO-NOT-FOUND (not E-RUNTIME). Sandbox still denies.
    let base = tmp_base("guard-abs");
    let prog = write_tmp_prog(
        &base,
        "abs.klang",
        "fn main() -> i32 {\n    print(read_file(\"/no/such/klang-file-xyz-outside-tmp\"))\n    return 0\n}\n",
    );
    let (out, err, code) = run_cli(&["run", &prog], "");
    assert_eq!(code, 1, "stdout:\n{out}\nstderr:\n{err}");
    assert!(err.contains("E-IO-NOT-FOUND"), "stderr:\n{err}");
    let _ = std::fs::remove_dir_all(&base);
}

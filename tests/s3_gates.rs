//! Wave1 S3 — formatter (`klang fmt <file|dir>`, `--check`) + LSP gates.
//!
//! S3a: one canonical style (see `docs/progress/S3-notes.md`); idempotent;
//! AST-equivalent on the whole corpus (`examples/`, `tests/parity/`, the
//! vendored `stdlib-packages/`); comments preserved (the lexer keeps them
//! as trivia, the formatter re-attaches them — never deleted); `--check`
//! exit codes; bad syntax leaves the file untouched.
//! S3b: spawn `klang lsp`, initialize, didOpen with a known error, assert
//! the published line, fix via didChange, assert clear, shutdown/exit.
//!
//! Every test uses its own unique temp dir (pid + atomic counter), never
//! a shared fixed path.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use klang::ast::*;
use klang::parser::Parser;

static CTR: AtomicU64 = AtomicU64::new(0);

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let id = CTR.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("klang-s3-{tag}-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn parse(src: &str) -> Program {
    let mut p = Parser::new(src);
    p.parse_program().expect("parses")
}

fn fmt(src: &str) -> String {
    klang::fmt::fmt_source(src, "test.klang").expect("formats")
}

// ---------------------------------------------------------------------------
// AST normalization: ignore spans/ids/omitted-arrow for equivalence
// ---------------------------------------------------------------------------

fn zero(id: &mut NodeId) {
    id.path.clear();
}

fn norm_expr(e: &mut Expr) {
    zero(e.id_mut());
    match e {
        Expr::Int { .. } | Expr::Float { .. } | Expr::Str { .. } | Expr::Bool { .. }
        | Expr::Var { .. } | Expr::Await { .. } => {}
        Expr::ArrayLit { elems, .. } => elems.iter_mut().for_each(norm_expr),
        Expr::MapLit { entries, .. } => entries.iter_mut().for_each(|(_, v)| norm_expr(v)),
        Expr::StructLit { fields, .. } => fields.iter_mut().for_each(|(_, v)| norm_expr(v)),
        Expr::EnumCtor { args, .. } => args.iter_mut().for_each(norm_expr),
        Expr::Match { scrutinee, arms, .. } => {
            norm_expr(scrutinee);
            for a in arms {
                zero(&mut a.id);
                a.stmts.iter_mut().for_each(norm_stmt);
                if let Some(g) = &mut a.guard {
                    norm_expr(g);
                }
                norm_expr(&mut a.body);
            }
        }
        Expr::Closure { body, .. } => norm_block(body),
        Expr::Index { base, index, .. } => {
            norm_expr(base);
            norm_expr(index);
        }
        Expr::Field { base, .. } => norm_expr(base),
        Expr::MethodCall { base, args, .. } => {
            norm_expr(base);
            args.iter_mut().for_each(norm_expr);
        }
        Expr::Spawn { call, .. } => norm_expr(call),
        Expr::Call { args, .. } => args.iter_mut().for_each(norm_expr),
        Expr::Add { left, right, .. }
        | Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. }
        | Expr::Eq { left, right, .. }
        | Expr::NotEq { left, right, .. }
        | Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. }
        | Expr::And { left, right, .. }
        | Expr::Or { left, right, .. } => {
            norm_expr(left);
            norm_expr(right);
        }
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => norm_expr(inner),
    }
}

fn norm_stmt(s: &mut Stmt) {
    match s {
        Stmt::Let(l) => {
            zero(&mut l.id);
            l.name_span = (0, 0);
            l.span = (0, 0);
            norm_expr(&mut l.value);
        }
        Stmt::Assign(a) => {
            zero(&mut a.id);
            match &mut a.target {
                AssignTarget::Var { .. } => {}
                AssignTarget::Index { base, index } => {
                    norm_expr(base);
                    norm_expr(index);
                }
                AssignTarget::Field { base, .. } => norm_expr(base),
            }
            norm_expr(&mut a.value);
        }
        Stmt::Return(r) => {
            zero(&mut r.id);
            r.span = (0, 0);
            norm_expr(&mut r.value);
        }
        Stmt::TaskGroup(g) => {
            zero(&mut g.id);
            norm_block(&mut g.body);
        }
        Stmt::If(i) => {
            zero(&mut i.id);
            norm_expr(&mut i.cond);
            norm_block(&mut i.then_block);
            if let Some(e) = &mut i.else_block {
                norm_block(e);
            }
        }
        Stmt::Print(p) => {
            zero(&mut p.id);
            norm_expr(&mut p.value);
        }
        Stmt::While(w) => {
            zero(&mut w.id);
            norm_expr(&mut w.cond);
            norm_block(&mut w.body);
        }
        Stmt::ForRange(fr) => {
            zero(&mut fr.id);
            norm_expr(&mut fr.start);
            norm_expr(&mut fr.end);
            norm_block(&mut fr.body);
        }
        Stmt::ForIn(fi) => {
            zero(&mut fi.id);
            norm_expr(&mut fi.iter);
            norm_block(&mut fi.body);
        }
        Stmt::Break(b) => zero(&mut b.id),
        Stmt::Continue(c) => zero(&mut c.id),
        Stmt::TryCatch(t) => {
            zero(&mut t.id);
            norm_block(&mut t.body);
            norm_block(&mut t.handler);
        }
        Stmt::Expr(e) => norm_expr(e),
    }
}

fn norm_block(b: &mut Block) {
    zero(&mut b.id);
    b.stmts.iter_mut().for_each(norm_stmt);
}

fn norm_func(f: &mut FunctionDecl) {
    zero(&mut f.id);
    f.name_span = (0, 0);
    f.return_ty_omitted = false;
    norm_block(&mut f.body);
}

fn norm_struct(s: &mut StructDecl) {
    zero(&mut s.id);
}

fn norm_enum(e: &mut EnumDecl) {
    zero(&mut e.id);
}

fn norm_mod(m: &mut ModDecl) {
    zero(&mut m.id);
    m.structs.iter_mut().for_each(norm_struct);
    m.enums.iter_mut().for_each(norm_enum);
    m.functions.iter_mut().for_each(norm_func);
}

fn norm_program(p: &mut Program) {
    p.mods.iter_mut().for_each(norm_mod);
    p.enums.iter_mut().for_each(norm_enum);
    p.structs.iter_mut().for_each(norm_struct);
    p.functions.iter_mut().for_each(norm_func);
}

fn assert_ast_eq(a_src: &str, b_src: &str, ctx: &str) {
    let mut a = parse(a_src);
    let mut b = parse(b_src);
    norm_program(&mut a);
    norm_program(&mut b);
    assert_eq!(a, b, "AST mismatch (ignoring spans) for {ctx}");
}

// ---------------------------------------------------------------------------
// S3a: idempotence + comment preservation (unit level)
// ---------------------------------------------------------------------------

#[test]
fn s3_fmt_idempotent_with_comments() {
    let cases = [
        "// header\n// second\nfn main() -> i32 {\n    return 1\n}\n",
        "fn main() -> i32 {\n    // inside\n    let x = 1 // trailing\n    return x\n}\n",
        "/* block */\nfn main() -> i32 {\n    return /* mid */ 1\n}\n",
        "/* multi\nline\nblock */\nfn add(a: i32, b: i32) -> i32 {\n    return a + b\n}\nfn main() -> i32 {\n    return add(20, 22)\n}\n",
        "// c1\nimport \"lib.klang\"\n// c2\nstruct P { x: i32 }\n// c3\nfn main() -> i32 {\n    // c4\n    return 0 // c5\n}\n// footer\n",
        "fn main() {\n    // omitted-arrow body\n    print(1)\n}\n",
        "fn f(a: i32) -> i32 { return a }\nfn main() -> i32 { return f(1) }\n",
    ];
    for src in cases {
        let once = fmt(src);
        let twice = fmt(&once);
        assert_eq!(once, twice, "fmt must be idempotent for:\n{src}\n got:\n{once}");
    }
}

#[test]
fn s3_fmt_preserves_all_comment_shapes() {
    let src = "// header marker alpha\n/* block marker beta */\nfn main() -> i32 {\n    // inner marker gamma\n    let x = 1 // trailing marker delta\n    return x /* tail marker epsilon */\n}\n// footer marker zeta\n/* multi marker eta\nsecond line */\n";
    let out = fmt(src);
    for marker in ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta"] {
        assert!(out.contains(marker), "comment {marker} lost:\n{out}");
    }
    // Idempotent with all shapes present.
    assert_eq!(out, fmt(&out), "not a fixpoint:\n{out}");
    // Meaning unchanged.
    assert_ast_eq(src, &out, "comment shapes");
}

#[test]
fn s3_fmt_comment_free_matches_fmt_program() {
    // Without comments the new entry point is byte-identical to the old one.
    let srcs = [
        "fn add(a: i32, b: i32) -> i32 {\n    return a + b\n}\nfn main() -> i32 {\n    return add(20, 22)\n}\n",
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(20, 22) }",
        "import \"lib.klang\"\nstruct P { x: i32 }\nenum Opt { Some(x: i32), None }\nmod m {\n    pub fn f() -> i32 {\n        return 1\n    }\n}\nfn main() -> i32 {\n    return m::f()\n}\n",
    ];
    for src in srcs {
        let a = klang::fmt::fmt_source(src, "t.klang").expect("formats");
        let b = klang::fmt::fmt_program(&parse(src));
        assert_eq!(a, b, "comment-free fmt_source must equal fmt_program");
    }
}

#[test]
fn s3_fmt_bad_syntax_errors_cleanly() {
    let err = klang::fmt::fmt_source("fn main() -> i32 {\n    return ", "bad.klang").expect_err("must fail");
    assert_eq!(err.code, "E-PARSE");
    let err = klang::fmt::fmt_source("/* unterminated", "bad.klang").expect_err("must fail");
    assert_eq!(err.code, "E-PARSE");
}

// ---------------------------------------------------------------------------
// S3a: corpus gates (AST equivalence + idempotence on every file)
// ---------------------------------------------------------------------------

fn corpus_files() -> Vec<std::path::PathBuf> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in ["examples", "tests/parity", "stdlib-packages"] {
        collect(&root.join(dir), &mut out);
    }
    out.sort();
    assert!(!out.is_empty(), "corpus must not be empty");
    out
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().map(|n| n == ".git" || n == "target").unwrap_or(false) {
                continue;
            }
            collect(&p, out);
        } else if p.extension().map(|x| x == "klang").unwrap_or(false) {
            out.push(p);
        }
    }
}

#[test]
fn s3_fmt_corpus_ast_equivalent_and_idempotent() {
    let files = corpus_files();
    let mut ok = 0usize;
    let mut skipped = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).expect("read corpus file");
        let formatted = match klang::fmt::fmt_source(&src, &f.to_string_lossy()) {
            Ok(s) => s,
            Err(d) => {
                skipped.push(format!("{}: fmt error {}", f.display(), d.code));
                continue;
            }
        };
        // Idempotence on every corpus file.
        let twice = klang::fmt::fmt_source(&formatted, &f.to_string_lossy()).expect("re-formats");
        assert_eq!(formatted, twice, "not idempotent: {}", f.display());
        // AST equivalence ignoring spans.
        let (mut a, mut b) = match (
            Parser::new(&src).parse_program(),
            Parser::new(&formatted).parse_program(),
        ) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e1), Err(e2)) => {
                assert_eq!(e1.code, e2.code, "both fail, same code: {}", f.display());
                ok += 1;
                continue;
            }
            (r1, r2) => panic!(
                "parse divergence on {}: orig={} fmt={}",
                f.display(),
                r1.is_ok(),
                r2.is_ok()
            ),
        };
        norm_program(&mut a);
        norm_program(&mut b);
        assert_eq!(a, b, "AST mismatch: {}", f.display());
        // Comments preserved: every comment's text survives.
        let (_, comments, _) = klang::parser::lex_with_comments(&src, "c");
        for c in &comments {
            let raw = c.text(&src).trim().to_string();
            assert!(
                formatted.contains(&raw),
                "comment lost in {}: {raw:?}",
                f.display()
            );
        }
        ok += 1;
    }
    assert!(skipped.is_empty(), "skipped files: {skipped:?}");
    assert!(ok >= 50, "corpus too small ({ok} files)");
}

fn has_marker(src: &str, marker: &str) -> bool {
    src.contains(marker)
}

fn run_outcome(src: &str) -> Option<(bool, String, Vec<String>)> {
    // Parse + check + lower, then run with a wall clock bound.
    // None = not runnable here (skipped, never failed).
    if src.contains("import") {
        return None;
    }
    if !(src.contains("fn main") || src.contains("fn main<")) {
        return None;
    }
    for m in [
        "http_get",
        "http_post",
        "write_file",
        "append_file",
        "remove_file",
        "read_line",
        "run_process",
        "time_sleep",
    ] {
        if has_marker(src, m) {
            return None;
        }
    }
    // The whole pipeline runs on a deep-stack worker like the CLI
    // (`with_deep_stack`): front end, checker, and interpreter recurse,
    // so deep-but-legal programs must produce diagnostics, never abort
    // the test process with a native stack overflow.
    let owned = src.to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("s3-run-outcome".to_string())
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let r: Option<(bool, String, Vec<String>)> = (|| {
                let prog = Parser::new(&owned).parse_program().ok()?;
                if klang::hir::TypedHIR::check(prog.clone()).is_err() {
                    return None;
                }
                let mir = klang::mir::lower(&prog);
                let (r, out) =
                    klang::runtime::run_with_output_value_partial(&mir, "main", &[], &HashMap::new());
                Some(match r {
                    Ok(v) => (true, v.render(), out),
                    Err(d) => (false, d.code.clone(), out),
                })
            })();
            let _ = tx.send(r);
        })
        .expect("spawn outcome worker");
    rx.recv_timeout(Duration::from_secs(20)).ok()?
}

#[test]
fn s3_fmt_corpus_run_equivalent_where_runnable() {
    let files = corpus_files();
    let mut ran = 0usize;
    let mut skipped = 0usize;
    for f in &files {
        let src = std::fs::read_to_string(f).expect("read corpus file");
        let formatted = klang::fmt::fmt_source(&src, &f.to_string_lossy()).expect("formats");
        match (run_outcome(&src), run_outcome(&formatted)) {
            (Some(a), Some(b)) => {
                assert_eq!(a, b, "run divergence: {}", f.display());
                ran += 1;
            }
            _ => skipped += 1,
        }
    }
    assert!(ran >= 5, "expected runnable corpus files, ran {ran}");
    eprintln!("s3 run-equivalence: ran={ran} skipped={skipped}");
}

// ---------------------------------------------------------------------------
// S3a: CLI gates (`fmt <file|dir>`, `--check`, bad syntax)
// ---------------------------------------------------------------------------

fn klang() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_klang"))
}

fn run_cli(args: &[&str], cwd: &std::path::Path) -> (i32, String, String) {
    let out = Command::new(klang())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn klang");
    let code = out.status.code().unwrap_or(-1);
    (
        code,
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

const MESSY: &str = "// messy file\nfn add(a: i32, b: i32) -> i32 { return a + b }\nfn main() -> i32 {\n    // inner note\n    return add(20, 22)\n}\n";

#[test]
fn s3_fmt_check_exit_codes_file() {
    let dir = tmpdir("check-file");
    let f = dir.join("m.klang");
    std::fs::write(&f, MESSY).unwrap();
    let fs = f.to_string_lossy().to_string();
    // Unformatted: --check exits nonzero and names the file.
    let (code, stdout, _) = run_cli(&["fmt", "--check", &fs], &dir);
    assert_ne!(code, 0, "--check must fail on unformatted file: {stdout}");
    assert!(stdout.contains("m.klang"), "must print which file: {stdout}");
    // File untouched by --check.
    assert_eq!(std::fs::read_to_string(&f).unwrap(), MESSY);
    // --write formats (and preserves the comments).
    let (code, stdout, _) = run_cli(&["fmt", "--write", &fs], &dir);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("m.klang"), "{stdout}");
    let after = std::fs::read_to_string(&f).unwrap();
    assert!(after.contains("messy file") && after.contains("inner note"), "{after}");
    // Now --check passes.
    let (code, stdout, _) = run_cli(&["fmt", "--check", &fs], &dir);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("clean"), "{stdout}");
}

#[test]
fn s3_fmt_dir_check_and_write() {
    let dir = tmpdir("check-dir");
    std::fs::write(dir.join("a.klang"), MESSY).unwrap();
    std::fs::write(dir.join("b.klang"), "fn main() -> i32 {\n    return 0\n}\n").unwrap();
    let ds = dir.to_string_lossy().to_string();
    let (code, stdout, _) = run_cli(&["fmt", "--check", &ds], &dir);
    assert_ne!(code, 0, "{stdout}");
    assert!(stdout.contains("a.klang"), "{stdout}");
    assert!(!stdout.contains("b.klang"), "clean file must not be listed: {stdout}");
    assert_eq!(std::fs::read_to_string(dir.join("a.klang")).unwrap(), MESSY);
    let (code, _, _) = run_cli(&["fmt", "--write", &ds], &dir);
    assert_eq!(code, 0);
    let (code, stdout, _) = run_cli(&["fmt", "--check", &ds], &dir);
    assert_eq!(code, 0, "{stdout}");
}

#[test]
fn s3_fmt_bad_syntax_file_untouched() {
    let dir = tmpdir("bad-syntax");
    let f = dir.join("bad.klang");
    let bad = "fn main() -> i32 {\n    return \n}\n";
    std::fs::write(&f, bad).unwrap();
    let fs = f.to_string_lossy().to_string();
    let (code, stdout, _) = run_cli(&["fmt", "--write", &fs], &dir);
    assert_ne!(code, 0, "bad syntax must fail");
    assert!(stdout.contains("parse: FAIL"), "{stdout}");
    assert!(stdout.contains("E-PARSE"), "{stdout}");
    assert_eq!(std::fs::read_to_string(&f).unwrap(), bad, "file must be untouched");
    // --check on bad syntax also fails cleanly.
    let (code, stdout, _) = run_cli(&["fmt", "--check", &fs], &dir);
    assert_ne!(code, 0, "{stdout}");
    assert!(stdout.contains("E-PARSE"), "{stdout}");
}

#[test]
fn s3_fmt_stdout_default_and_already_formatted() {
    let dir = tmpdir("stdout");
    let f = dir.join("p.klang");
    std::fs::write(&f, MESSY).unwrap();
    let fs = f.to_string_lossy().to_string();
    // Plain `fmt <file>` prints formatted source, file untouched.
    let (code, stdout, _) = run_cli(&["fmt", &fs], &dir);
    assert_eq!(code, 0);
    assert!(stdout.contains("fn add"), "{stdout}");
    assert!(stdout.contains("messy file"), "{stdout}");
    assert_eq!(std::fs::read_to_string(&f).unwrap(), MESSY);
    // Idempotent through the CLI: formatting twice is stable.
    let f2 = dir.join("p2.klang");
    std::fs::write(&f2, &stdout).unwrap();
    let f2s = f2.to_string_lossy().to_string();
    let (code, stdout2, _) = run_cli(&["fmt", &f2s], &dir);
    assert_eq!(code, 0);
    assert_eq!(stdout, stdout2, "CLI fmt must be idempotent");
}

// ---------------------------------------------------------------------------
// S3b: LSP end-to-end gate
// ---------------------------------------------------------------------------

struct Session {
    child: Child,
    stdin: ChildStdin,
    inbox: mpsc::Receiver<String>,
}

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn read_framed_blocking(reader: &mut BufReader<std::process::ChildStdout>) -> Option<String> {
    let mut content_len: Option<usize> = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {}
            Err(_) => return None,
        }
        let t = line.trim();
        if t.is_empty() {
            break;
        }
        if let Some(v) = t.strip_prefix("Content-Length:") {
            content_len = v.trim().parse().ok();
        }
    }
    let n = content_len?;
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

impl Session {
    fn start() -> Self {
        let mut child = Command::new(klang())
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("klang lsp spawns");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(body) = read_framed_blocking(&mut reader) {
                if tx.send(body).is_err() {
                    break;
                }
            }
        });
        Self { child, stdin, inbox: rx }
    }

    fn send(&mut self, body: &str) {
        self.stdin.write_all(&frame(body)).expect("write frame");
        self.stdin.flush().expect("flush");
    }

    fn recv(&mut self) -> String {
        self.inbox
            .recv_timeout(Duration::from_secs(15))
            .expect("lsp server answered within 15s")
    }

    /// Next `publishDiagnostics` body for `uri` (skips other messages).
    fn recv_publish(&mut self, uri: &str) -> String {
        for _ in 0..10 {
            let body = self.recv();
            if body.contains("publishDiagnostics") && body.contains(uri) {
                return body;
            }
        }
        panic!("no publishDiagnostics for {uri}");
    }
}

fn diag_count(body: &str) -> usize {
    body.match_indices("\"code\":\"E-").count()
}

fn publish_line(body: &str) -> Option<u32> {
    let marker = "\"method\":\"textDocument/publishDiagnostics\"";
    let at = body.find(marker)?;
    let after = &body[at..];
    let l = after.find("\"line\":")?;
    after[l + 7..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

#[test]
fn s3_lsp_diagnostics_flow() {
    // Unique dir + real file: the loader path checks the BUFFER text.
    let dir = tmpdir("lsp-flow");
    let path = dir.join("main.klang");
    std::fs::write(&path, "fn main() -> i32 {\n    return 1\n}\n").unwrap();
    let uri = format!("file://{}", path.to_string_lossy());
    let mut s = Session::start();
    s.send("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"capabilities\":{}}}");
    let resp = s.recv();
    assert!(resp.contains("\"id\":1"), "{resp}");
    assert!(resp.contains("textDocumentSync"), "{resp}");
    s.send("{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}");
    // Unknown requests get a JSON-RPC error, never a hang.
    s.send("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"bogus/method\",\"params\":{}}");
    let resp = s.recv();
    assert!(resp.contains("-32601"), "{resp}");
    // Open with a known type error on line 2 (0-based line 1).
    let bad = "fn main() -> i32 {\\n    return \\\"hi\\\"\\n}\\n";
    s.send(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\",\"languageId\":\"klang\",\"version\":1,\"text\":\"{bad}\"}}}}}}"
    ));
    let body = s.recv_publish(&uri);
    assert!(body.contains("E-TYPE"), "{body}");
    assert_eq!(publish_line(&body), Some(1), "error is on line 2 (0-based 1): {body}");
    // Fix via didChange (full sync): diagnostics clear.
    let good = "fn main() -> i32 {\\n    return 1\\n}\\n";
    s.send(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":{{\"textDocument\":{{\"uri\":\"{uri}\",\"version\":2}},\"contentChanges\":[{{\"text\":\"{good}\"}}]}}}}"
    ));
    let mut cleared = false;
    for _ in 0..10 {
        let body = s.recv();
        if body.contains("publishDiagnostics") && body.contains(&uri) && diag_count(&body) == 0 {
            cleared = true;
            break;
        }
    }
    assert!(cleared, "diagnostics must clear after the fix");
    // Shutdown + exit cleanly.
    s.send("{\"jsonrpc\":\"2.0\",\"id\":999,\"method\":\"shutdown\"}");
    let resp = s.recv();
    assert!(resp.contains("\"id\":999"), "{resp}");
    s.send("{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match s.child.try_wait().expect("try_wait") {
            Some(status) => {
                assert!(status.success(), "lsp must exit 0 after shutdown, got {status}");
                break;
            }
            None if Instant::now() >= deadline => {
                s.child.kill().ok();
                panic!("lsp did not exit after shutdown/exit");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

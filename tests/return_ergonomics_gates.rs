//! D4 return-type ergonomics (Phase 3a Part 2).
//!
//! Omitting `->` on a function means `-> void`, desugared at parse time in
//! both the v1 and v2 parsers. A function without a declared return type
//! that returns a value is a loud `E-TYPE` with a fix hint naming the
//! type to declare — value-returning functions never silently become void.
//! `-> ()` stays rejected (`void` is the only unit spelling).

use std::collections::HashMap;

use klang::parser::Parser;

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn run_cli(args: &[&str], stdin_bytes: &[u8]) -> (String, String, i32) {
    use std::io::Write;
    let mut child = std::process::Command::new(klang_bin())
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("klang binary runs");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin_bytes)
        .expect("stdin writes");
    let output = child.wait_with_output().expect("klang exits");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn cli_prog(tag: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "klang-d4-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("prog.klang");
    std::fs::write(&path, src).unwrap();
    path.to_string_lossy().into_owned()
}

fn check_src(src: &str) -> Result<klang::Program, Vec<klang::Diagnostic>> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    klang::hir::TypedHIR::check(prog.clone()).map(|_| prog)
}

fn run_src(src: &str, entry: &str) -> (i32, Vec<String>) {
    let prog = check_src(src).expect("checks clean");
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("runs")
}

// ---------------------------------------------------------------------------
// Parse: omission desugars to void (v1 fns, v1 closures, v2 fns)
// ---------------------------------------------------------------------------

#[test]
fn omitted_arrow_is_void_on_functions_and_closures() {
    for src in [
        "fn foo(a: i32) { print(a) }",
        "fn main() { print(1) }",
        "fn main() { }",
    ] {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses");
        assert_eq!(prog.functions[0].return_ty, "void", "{src}");
    }
    // Closure literal without an arrow: same desugar.
    let mut p = Parser::new("fn main() -> i32 { let f = fn(x: i32) { print(x) } return 0 }");
    let prog = p.parse_program().expect("closure parses");
    let mut lits = Vec::new();
    klang::closures::collect_closures(&prog.functions[0].body, &mut lits);
    assert_eq!(lits.len(), 1, "one closure literal");
    assert_eq!(lits[0].return_ty, "void", "closure desugared to void");
    println!("v1 void-desugar OK");
}

#[test]
fn v2_omitted_arrow_is_void() {
    let prog = klang::parser::v2::parse_v2_program("fn main() { print(7) }")
        .expect("v2 parses");
    assert_eq!(prog.functions.len(), 1);
    assert_eq!(prog.functions[0].name, "main");
    assert_eq!(prog.functions[0].return_ty.display(), "void");
    let (r, out) = klang::runtime::v2::run_v2_program_partial(&prog, "main");
    // Fall-off yields the last value on v2 exactly as on v1 (print(7)
    // leaves 7): the D4 desugar changes no runtime behavior.
    assert_eq!(r.expect("v2 runs"), 7);
    assert_eq!(out, vec!["7".to_string()]);
    println!("v2 void-desugar OK");
}

// ---------------------------------------------------------------------------
// Check: value-returns without a declared type are loud E-TYPE with a fix
// ---------------------------------------------------------------------------

#[test]
fn value_return_without_type_suggests_the_arrow() {
    for (src, want_fix) in [
        (
            "fn add(a: i32, b: i32) { return a + b } fn main() -> i32 { return 0 }",
            "add `-> i32`",
        ),
        (
            "fn greet() { return \"hi\" } fn main() -> i32 { return 0 }",
            "add `-> str`",
        ),
        (
            "fn half(x: i32) -> void { return x / 2 } fn main() -> i32 { return 0 }",
            "add `-> i32`",
        ),
    ] {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses (omission is legal)");
        let diags = klang::hir::TypedHIR::check(prog).expect_err("value return is E-TYPE");
        let d = diags.iter().find(|d| d.code == "E-TYPE").expect("E-TYPE present");
        assert!(
            d.message.contains("want void"),
            "names the void contract: {}",
            d.to_json()
        );
        assert!(
            d.fixes.iter().any(|f| f.label.contains(want_fix)),
            "suggests the fix ({want_fix}): {}",
            d.to_json()
        );
        println!("value-return hint OK: {want_fix}");
    }
}

#[test]
fn closure_value_return_without_type_suggests_the_arrow() {
    let mut p =
        Parser::new("fn main() -> i32 { let f = fn(x: i32) { return x } return f(1) }");
    let prog = p.parse_program().expect("parses (omission is legal)");
    let diags = klang::hir::TypedHIR::check(prog).expect_err("closure value return is E-TYPE");
    assert!(
        diags.iter().any(|d| d.code == "E-TYPE"
            && d.message.contains("want void")
            && d.fixes.iter().any(|f| f.label.contains("add `-> i32`"))),
        "closure gets the same hint: {:?}",
        diags.iter().map(|d| d.to_json()).collect::<Vec<_>>()
    );
    // And the fixed closure checks clean and runs.
    let (v, _) = run_src(
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x } return f(41) }",
        "main",
    );
    assert_eq!(v, 41);
    println!("closure hint OK");
}

#[test]
fn non_void_mismatch_keeps_generic_fixes() {
    // The D4 hint fires only against `void`; ordinary mismatches keep the
    // long-standing generic fixes (no misleading `add ->` suggestion).
    let mut p = Parser::new("fn main() -> i32 { return \"hi\" }");
    let prog = p.parse_program().expect("parses");
    let diags = klang::hir::TypedHIR::check(prog).expect_err("mismatch is E-TYPE");
    let d = diags.iter().find(|d| d.code == "E-TYPE").expect("E-TYPE present");
    assert!(
        d.fixes.iter().all(|f| !f.label.contains("add `->")),
        "no arrow hint for non-void mismatch: {}",
        d.to_json()
    );
    println!("generic fixes unchanged OK");
}

#[test]
fn unit_parens_still_rejected() {
    for src in [
        "fn main() -> () { print(1) }",
        "fn foo() -> () { print(1) } fn main() -> i32 { return 0 }",
    ] {
        let mut p = Parser::new(src);
        let d = p.parse_program().expect_err("`-> ()` fails");
        assert_eq!(d.code, "E-PARSE", "{}", d.to_json());
    }
    let d = klang::parser::v2::parse_v2_program("fn main() -> () { return 1 }")
        .expect_err("v2 `-> ()` fails");
    assert_eq!(d.code, "E-PARSE-V2", "{}", d.to_json());
    println!("unit-parens reject OK");
}

// ---------------------------------------------------------------------------
// Run: void functions run; main works in every form
// ---------------------------------------------------------------------------

#[test]
fn void_function_runs_and_returns_to_caller() {
    let (v, out) = run_src(
        "fn shout(m: str) -> void { print(m) } fn main() -> i32 { shout(\"a\") shout(\"b\") return 3 }",
        "main",
    );
    assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(v, 3);
    // Omitted-arrow twin behaves identically.
    let (v2, out2) = run_src(
        "fn shout(m: str) { print(m) } fn main() -> i32 { shout(\"a\") shout(\"b\") return 3 }",
        "main",
    );
    assert_eq!((v2, out2), (v, out));
    println!("void helper OK");
}

#[test]
fn main_in_all_forms_cli() {
    // Explicit `-> i32`: exit code is the return value.
    let prog = cli_prog("i32", "fn main() -> i32 { print(1) return 42 }");
    let (out, err, code) = run_cli(&["run", &prog], b"");
    assert_eq!((code, out.as_str()), (42, "1\n"), "stderr:\n{err}");
    // Omitted arrow: prints, exits 0 (void main contract).
    let prog = cli_prog("omitted", "fn main() { print(\"hi\") }");
    let (out, err, code) = run_cli(&["run", &prog], b"");
    assert_eq!(code, 0, "stderr:\n{err}");
    assert_eq!(out, "hi\n", "got:\n{out}");
    // Explicit `-> void` twin: identical bytes and code.
    let prog = cli_prog("void", "fn main() -> void { print(\"hi\") }");
    let (out2, err2, code2) = run_cli(&["run", &prog], b"");
    assert_eq!((code2, out2.as_str()), (code, out.as_str()), "stderr:\n{err2}");
    // Non-Int main (`-> str`): output preserved, exit 0.
    let prog = cli_prog("str", "fn main() -> str { print(\"hi\") return \"done\" }");
    let (out, err, code) = run_cli(&["run", &prog], b"");
    assert_eq!((code, out.as_str()), (0, "hi\n"), "stderr:\n{err}");
    println!("main forms OK");
}

#[test]
fn void_main_exit_code_is_always_zero_when_omitted() {
    // Phase 3b decision: a main with NO declared return type always exits
    // 0 — its last expression value is discarded for exit purposes.
    // Empty body exits 0.
    let prog = cli_prog("empty", "fn main() { }");
    let (_, _, code) = run_cli(&["run", &prog], b"");
    assert_eq!(code, 0);
    // A trailing Int value falls off the end but is discarded: exit 0,
    // not 42.
    let prog = cli_prog("falloff", "fn main() { 40 + 2 }");
    let (_, _, code) = run_cli(&["run", &prog], b"");
    assert_eq!(code, 0);
    // The required case: printing 7 still prints, but exits 0.
    let prog = cli_prog("print7", "fn main() { print(7) }");
    let (out, _, code) = run_cli(&["run", &prog], b"");
    assert_eq!(out, "7\n", "output preserved");
    assert_eq!(code, 0, "omitted arrow always exits 0");
    // Declared `-> void` is NOT omission: it keeps today's value mapping.
    let prog = cli_prog("explicit-void", "fn main() -> void { print(7) }");
    let (out, _, code) = run_cli(&["run", &prog], b"");
    assert_eq!(out, "7\n");
    assert_eq!(code, 7, "explicit `-> void` maps its fall-off value");
    println!("void-main exit-0 OK");
}

// ---------------------------------------------------------------------------
// Fmt: omitted arrow prints the canonical explicit form, idempotently
// ---------------------------------------------------------------------------

#[test]
fn fmt_canonicalizes_omitted_arrow_to_void() {
    for (omitted, explicit) in [
        (
            "fn main() { print(\"hi\") }",
            "fn main() -> void { print(\"hi\") }",
        ),
        (
            "fn shout(m: str) { print(m) }",
            "fn shout(m: str) -> void { print(m) }",
        ),
    ] {
        let mut p = Parser::new(omitted);
        let prog = p.parse_program().expect("parses");
        let mut q = Parser::new(explicit);
        let twin = q.parse_program().expect("twin parses");
        assert_eq!(
            klang::fmt::fmt_program(&prog),
            klang::fmt::fmt_program(&twin),
            "fmt(omitted) == fmt(explicit void)"
        );
        // Round-trip: formatted output re-parses to the same text.
        let once = klang::fmt::fmt_program(&prog);
        let mut r = Parser::new(&once);
        let re = r.parse_program().expect("formatted re-parses");
        assert_eq!(klang::fmt::fmt_program(&re), once, "fmt idempotent");
    }
    println!("fmt canonicalization OK");
}

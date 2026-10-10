//! Wave1 S4 (D3) — integer boundary gates.
//!
//! `Int` is 64-bit with checked arithmetic; the annotations `i32`, `u32`,
//! `i64`, `u64`, `u8` enforce their range at value boundaries —
//! parameter passing, `return`, and assignment to an annotated place
//! (parameter rebinding, struct construction, struct field assignment).
//! A value outside the range is a loud `E-RUNTIME` naming the type and
//! the value. Overflow past 64 bits stays `E-OVERFLOW` on the interpreter
//! and on the v2 tree-walker alike.
//!
//! Each test uses its own unique temp dir (pid + atomic counter) and
//! runs the real load → check → lower → run pipeline.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static CTR: AtomicU64 = AtomicU64::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    let c = CTR.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "klang-s4-int-{}-{}-{}",
        tag,
        std::process::id(),
        c
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn load(dir: &PathBuf, name: &str, src: &str) -> klang::ast::Program {
    let path = dir.join(name);
    std::fs::write(&path, src).unwrap();
    klang::imports::load_program(&path.to_string_lossy())
        .unwrap_or_else(|e| panic!("load failed for {name}: {}", e.to_json()))
        .program
}

fn check_clean(prog: &klang::ast::Program) {
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
}

/// Run `entry` through the value channel (no `i32`-channel truncation, so
/// wide `Int` values are observable).
fn run_value(
    prog: &klang::ast::Program,
    entry: &str,
) -> Result<(klang::runtime::Value, Vec<String>), klang::diagnostics::Diagnostic> {
    let mir = klang::mir::lower(prog);
    klang::runtime::run_with_output_value(&mir, entry, &[], &HashMap::new())
}

fn expect_ok(dir_tag: &str, src: &str, entry: &str, want_render: &str) {
    let dir = tmpdir(dir_tag);
    let prog = load(&dir, "prog.klang", src);
    check_clean(&prog);
    let (v, _) = run_value(&prog, entry).unwrap_or_else(|d| panic!("must run: {}", d.to_json()));
    assert_eq!(v.render(), want_render, "{src}");
}

fn expect_err(dir_tag: &str, src: &str, entry: &str, code: &str, contains: &[&str]) {
    let dir = tmpdir(dir_tag);
    let prog = load(&dir, "prog.klang", src);
    check_clean(&prog);
    let d = run_value(&prog, entry).expect_err("must fail at run time");
    assert_eq!(d.code, code, "{src}: {}", d.to_json());
    for needle in contains {
        assert!(
            d.message.contains(needle),
            "{src}: message missing {needle:?}: {}",
            d.to_json()
        );
    }
}

// ---------------------------------------------------------------------------
// i32: -2147483648..2147483647 at param / return / assignment boundaries.
// ---------------------------------------------------------------------------

#[test]
fn i32_boundary_param() {
    let mk = |arg: &str| {
        format!("fn id(x: i32) -> i32 {{ return x }} fn main() -> i32 {{ return id({arg}) }}")
    };
    expect_ok("i32p-in", &mk("2147483646"), "main", "2147483646");
    expect_ok("i32p-max", &mk("2147483647"), "main", "2147483647");
    expect_ok("i32p-min", &mk("-2147483648"), "main", "-2147483648");
    expect_ok("i32p-min1", &mk("-2147483647"), "main", "-2147483647");
    expect_err(
        "i32p-max1",
        &mk("2147483648"),
        "main",
        "E-RUNTIME",
        &["i32", "2147483648"],
    );
    expect_err(
        "i32p-min1b",
        &mk("-2147483649"),
        "main",
        "E-RUNTIME",
        &["i32", "-2147483649"],
    );
    println!("i32 param OK");
}

#[test]
fn i32_boundary_return() {
    let mk = |ret: &str| format!("fn f() -> i32 {{ return {ret} }} fn main() -> i32 {{ return f() }}");
    expect_ok("i32r-max", &mk("2147483647"), "main", "2147483647");
    expect_ok("i32r-min", &mk("-2147483648"), "main", "-2147483648");
    expect_err("i32r-max1", &mk("2147483648"), "main", "E-RUNTIME", &["i32", "2147483648"]);
    expect_err("i32r-min1", &mk("-2147483649"), "main", "E-RUNTIME", &["i32", "-2147483649"]);
    println!("i32 return OK");
}

#[test]
fn i32_boundary_assign() {
    let mk = |rhs: &str| {
        format!("fn f(x: i32) -> i32 {{ x = {rhs} return x }} fn main() -> i32 {{ return f(0) }}")
    };
    expect_ok("i32a-in", &mk("2147483647"), "main", "2147483647");
    expect_err("i32a-max1", &mk("2147483648"), "main", "E-RUNTIME", &["i32", "2147483648"]);
    expect_err("i32a-min1", &mk("-2147483649"), "main", "E-RUNTIME", &["i32", "-2147483649"]);
    println!("i32 assign OK");
}

// ---------------------------------------------------------------------------
// u32: 0..4294967295. Negatives always fail.
// ---------------------------------------------------------------------------

#[test]
fn u32_boundaries() {
    let param = |arg: &str| {
        format!("fn id(x: u32) -> u32 {{ return x }} fn main() -> u32 {{ return id({arg}) }}")
    };
    expect_ok("u32p-zero", &param("0"), "main", "0");
    expect_ok("u32p-one", &param("1"), "main", "1");
    expect_ok("u32p-max1", &param("4294967294"), "main", "4294967294");
    expect_ok("u32p-max", &param("4294967295"), "main", "4294967295");
    expect_err("u32p-max1b", &param("4294967296"), "main", "E-RUNTIME", &["u32", "4294967296"]);
    expect_err("u32p-neg", &param("0 - 1"), "main", "E-RUNTIME", &["u32", "-1"]);
    let ret = |r: &str| format!("fn f() -> u32 {{ return {r} }} fn main() -> u32 {{ return f() }}");
    expect_ok("u32r-max", &ret("4294967295"), "main", "4294967295");
    expect_err("u32r-max1", &ret("4294967296"), "main", "E-RUNTIME", &["u32", "4294967296"]);
    expect_err("u32r-neg", &ret("0 - 1"), "main", "E-RUNTIME", &["u32", "-1"]);
    let asn = |r: &str| {
        format!("fn f(x: u32) -> u32 {{ x = {r} return x }} fn main() -> u32 {{ return f(0) }}")
    };
    expect_ok("u32a-max", &asn("4294967295"), "main", "4294967295");
    expect_err("u32a-max1", &asn("4294967296"), "main", "E-RUNTIME", &["u32", "4294967296"]);
    expect_err("u32a-neg", &asn("0 - 1"), "main", "E-RUNTIME", &["u32", "-1"]);
    println!("u32 OK");
}

// ---------------------------------------------------------------------------
// i64: full range, no narrowing. u64: non-negative i64 values pass.
// ---------------------------------------------------------------------------

#[test]
fn i64_full_range_passes() {
    let src = "fn id(x: i64) -> i64 { return x } \
               fn main() -> i64 { print(id(9223372036854775807)) print(id(-9223372036854775808)) return id(5000000000) }";
    let dir = tmpdir("i64full");
    let prog = load(&dir, "prog.klang", src);
    check_clean(&prog);
    let (v, out) = run_value(&prog, "main").expect("runs");
    assert_eq!(v.render(), "5000000000");
    assert_eq!(out, vec!["9223372036854775807".to_string(), "-9223372036854775808".to_string()]);
    println!("i64 full OK");
}

#[test]
fn u64_boundaries() {
    let param = |arg: &str| {
        format!("fn id(x: u64) -> u64 {{ return x }} fn main() -> u64 {{ return id({arg}) }}")
    };
    expect_ok("u64p-zero", &param("0"), "main", "0");
    expect_ok("u64p-big", &param("9223372036854775807"), "main", "9223372036854775807");
    expect_err("u64p-neg", &param("0 - 1"), "main", "E-RUNTIME", &["u64", "-1"]);
    let ret = |r: &str| format!("fn f() -> u64 {{ return {r} }} fn main() -> u64 {{ return f() }}");
    expect_ok("u64r-big", &ret("9223372036854775807"), "main", "9223372036854775807");
    expect_err("u64r-neg", &ret("0 - 1"), "main", "E-RUNTIME", &["u64", "-1"]);
    let asn = |r: &str| {
        format!("fn f(x: u64) -> u64 {{ x = {r} return x }} fn main() -> u64 {{ return f(0) }}")
    };
    expect_ok("u64a-big", &asn("9223372036854775807"), "main", "9223372036854775807");
    expect_err("u64a-neg", &asn("0 - 1"), "main", "E-RUNTIME", &["u64", "-1"]);
    println!("u64 OK");
}

// ---------------------------------------------------------------------------
// u8: 0..255, now checked (was unchecked dynamic).
// ---------------------------------------------------------------------------

#[test]
fn u8_boundaries() {
    let param = |arg: &str| {
        format!("fn id(x: u8) -> u8 {{ return x }} fn main() -> i32 {{ return id({arg}) }}")
    };
    expect_ok("u8p-zero", &param("0"), "main", "0");
    expect_ok("u8p-one", &param("1"), "main", "1");
    expect_ok("u8p-max1", &param("254"), "main", "254");
    expect_ok("u8p-max", &param("255"), "main", "255");
    expect_err("u8p-max1b", &param("256"), "main", "E-RUNTIME", &["u8", "256"]);
    expect_err("u8p-neg", &param("0 - 1"), "main", "E-RUNTIME", &["u8", "-1"]);
    let ret = |r: &str| format!("fn f() -> u8 {{ return {r} }} fn main() -> i32 {{ return f() }}");
    expect_ok("u8r-max", &ret("255"), "main", "255");
    expect_err("u8r-max1", &ret("256"), "main", "E-RUNTIME", &["u8", "256"]);
    expect_err("u8r-neg", &ret("0 - 1"), "main", "E-RUNTIME", &["u8", "-1"]);
    let asn = |r: &str| {
        format!("fn f(x: u8) -> i32 {{ x = {r} return x }} fn main() -> i32 {{ return f(0) }}")
    };
    expect_ok("u8a-max", &asn("255"), "main", "255");
    expect_err("u8a-max1", &asn("256"), "main", "E-RUNTIME", &["u8", "256"]);
    expect_err("u8a-neg", &asn("0 - 1"), "main", "E-RUNTIME", &["u8", "-1"]);
    println!("u8 OK");
}

// ---------------------------------------------------------------------------
// Struct construction + field assignment boundaries.
// ---------------------------------------------------------------------------

#[test]
fn struct_field_boundaries() {
    // Construction at / inside / outside the u8 range.
    expect_ok(
        "scon-in",
        "struct S { n: u8 } fn main() -> i32 { let s = S { n: 255 } return s.n }",
        "main",
        "255",
    );
    expect_err(
        "scon-out",
        "struct S { n: u8 } fn main() -> i32 { let s = S { n: 256 } return s.n }",
        "main",
        "E-RUNTIME",
        &["u8", "256"],
    );
    // Field assignment at / inside / outside.
    expect_ok(
        "sset-in",
        "struct S { n: u8 } fn main() -> i32 { let s = S { n: 0 } s.n = 255 print(s.n) return s.n }",
        "main",
        "255",
    );
    expect_err(
        "sset-out",
        "struct S { n: u8 } fn main() -> i32 { let s = S { n: 0 } s.n = 256 return s.n }",
        "main",
        "E-RUNTIME",
        &["u8", "256"],
    );
    expect_err(
        "sset-neg",
        "struct S { n: u8 } fn main() -> i32 { let s = S { n: 0 } s.n = 0 - 1 return s.n }",
        "main",
        "E-RUNTIME",
        &["u8", "-1"],
    );
    // i32-annotated field: wide values now pass construction (used to
    // fail the old i32 return check only at `run` boundaries).
    expect_ok(
        "scon-i32-wide",
        "struct S { n: i32 } fn main() -> i32 { let s = S { n: 42 } return s.n }",
        "main",
        "42",
    );
    println!("struct boundaries OK");
}

// ---------------------------------------------------------------------------
// 64-bit overflow is still loud (interpreter).
// ---------------------------------------------------------------------------

#[test]
fn overflow_past_64_bits_is_error() {
    for (tag, src) in [
        ("add", "fn main() -> i32 { let a = 9223372036854775807 let b = a + 1 print(b) return 0 }"),
        ("sub", "fn main() -> i32 { let a = -9223372036854775808 let b = a - 1 print(b) return 0 }"),
        ("mul", "fn main() -> i32 { let a = 9223372036854775807 let b = a * 2 print(b) return 0 }"),
        ("div", "fn main() -> i32 { let a = -9223372036854775808 let b = a / -1 print(b) return 0 }"),
        ("mod", "fn main() -> i32 { let a = -9223372036854775808 let b = a % -1 print(b) return 0 }"),
        ("neg", "fn mymin() -> i64 { return 0 - 9223372036854775807 - 1 } fn main() -> i32 { return 0 - mymin() }"),
    ] {
        expect_err(&format!("ovf-{tag}"), src, "main", "E-OVERFLOW", &["i64"]);
    }
    // Just inside the range computes exactly.
    expect_ok(
        "ovf-noerr",
        "fn main() -> i64 { return 9223372036854775807 + 0 }",
        "main",
        "9223372036854775807",
    );
    println!("64-bit overflow OK");
}

// ---------------------------------------------------------------------------
// v2 tree-walker agrees on the 64-bit boundary.
// ---------------------------------------------------------------------------

#[test]
fn v2_overflow_parity() {
    use klang::parser::v2::parse_v2_program;
    let run = |src: &str| {
        let prog = parse_v2_program(src).expect("v2 parses");
        klang::runtime::v2::run_v2_program(&prog, "main")
    };
    // Values just inside the range run on v2 exactly like v1.
    let (v, out) = run("fn main() -> i32 { print(9223372036854775807) return 0 }")
        .expect("v2 runs max");
    assert_eq!(v, 0);
    assert_eq!(out, vec!["9223372036854775807".to_string()]);
    // Past the range is loud E-OVERFLOW on v2, like v1.
    for (tag, src) in [
        ("add", "fn main() -> i32 { print(9223372036854775807 + 1) return 0 }"),
        ("mul", "fn main() -> i32 { print(9223372036854775807 * 2) return 0 }"),
    ] {
        let d = run(src).expect_err(&format!("v2 {tag} must overflow"));
        assert_eq!(d.code, "E-OVERFLOW", "{tag}: {}", d.to_json());
    }
    // v1 agrees on the same programs (same code, reference behavior).
    let dir = tmpdir("v2agree");
    for (tag, src) in [
        ("add", "fn main() -> i32 { print(9223372036854775807 + 1) return 0 }"),
        ("mul", "fn main() -> i32 { print(9223372036854775807 * 2) return 0 }"),
    ] {
        let prog = load(&dir, &format!("{tag}.klang"), src);
        check_clean(&prog);
        let d = run_value(&prog, "main").expect_err("v1 must overflow too");
        assert_eq!(d.code, "E-OVERFLOW", "v1 {tag}: {}", d.to_json());
    }
    println!("v2 parity OK");
}

// ---------------------------------------------------------------------------
// CLI: boundary violations exit 1 with E-RUNTIME; wide values run.
// ---------------------------------------------------------------------------

#[test]
fn cli_boundary_exit_codes() {
    fn klang_bin() -> String {
        env!("CARGO_BIN_EXE_klang").to_string()
    }
    fn run_file(path: &str) -> (String, String, i32) {
        let out = std::process::Command::new(klang_bin())
            .arg("run")
            .arg(path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("klang runs");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
    let dir = tmpdir("cli");
    let ok = dir.join("ok.klang");
    std::fs::write(&ok, "fn id(x: u8) -> u8 { return x } fn main() -> i32 { print(id(255)) return 0 }").unwrap();
    let (stdout, _, code) = run_file(&ok.to_string_lossy());
    assert_eq!(code, 0);
    assert!(stdout.contains("255"), "{stdout}");
    let bad = dir.join("bad.klang");
    std::fs::write(&bad, "fn id(x: u8) -> u8 { return x } fn main() -> i32 { print(id(256)) return 0 }").unwrap();
    let (_, stderr, code) = run_file(&bad.to_string_lossy());
    assert_eq!(code, 1);
    assert!(stderr.contains("E-RUNTIME") && stderr.contains("u8") && stderr.contains("256"), "{stderr}");
    let wide = dir.join("wide.klang");
    std::fs::write(&wide, "fn main() -> i64 { print(5000000000) return 5000000000 }").unwrap();
    let (stdout, _, _) = run_file(&wide.to_string_lossy());
    assert!(stdout.contains("5000000000"), "{stdout}");
    println!("CLI boundaries OK");
}

//! Wave1 S4 (D2) — strict-truthiness gates.
//!
//! Default mode is unchanged: `Bool` and `Int` conditions (`if`/`while`,
//! `&&`/`||`/`!` operands) all check clean and run. Opt-in strict mode
//! (`klang run/check --strict`, or `strict = true` under `[project]` in
//! `klang.toml`) rejects every *statically-known* non-`Bool` condition
//! with `E-TYPE` naming the offending type. `Unknown` (dynamic: map
//! lookups, dynamic index) stays allowed in both modes — the runtime
//! erases `Bool` to `Int` 0/1, so no runtime rule could tell them apart;
//! strictness is a compile-time rule by necessity (see the D2 Decision).
//!
//! Each test uses its own unique temp dir (pid + atomic counter).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static CTR: AtomicU64 = AtomicU64::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    let c = CTR.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "klang-s4-strict-{}-{}-{}",
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

fn check_default(prog: &klang::ast::Program) -> Result<(), Vec<klang::diagnostics::Diagnostic>> {
    klang::hir::TypedHIR::check(prog.clone()).map(|_| ())
}

fn check_strict(prog: &klang::ast::Program) -> Result<(), Vec<klang::diagnostics::Diagnostic>> {
    klang::hir::TypedHIR::check_with_file_strict(prog.clone(), "probe.klang", true).map(|_| ())
}

fn run_ok(prog: &klang::ast::Program, entry: &str) -> (klang::runtime::Value, Vec<String>) {
    let mir = klang::mir::lower(prog);
    klang::runtime::run_with_output_value(&mir, entry, &[], &HashMap::new())
        .unwrap_or_else(|d| panic!("must run: {}", d.to_json()))
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn cli(dir: &PathBuf, args: &[&str]) -> (String, String, i32) {
    let out = std::process::Command::new(klang_bin())
        .args(args)
        .current_dir(dir)
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

// ---------------------------------------------------------------------------
// Default mode: Int conditions stay legal everywhere (unchanged behavior).
// ---------------------------------------------------------------------------

#[test]
fn default_mode_int_conditions_run() {
    for (tag, src, want_out) in [
        ("if", "fn main() -> i32 { if 1 { print(7) } return 0 }", vec!["7"]),
        ("if-else", "fn main() -> i32 { if 0 { print(1) } else { print(2) } return 0 }", vec!["2"]),
        ("while", "fn main() -> i32 { let i = 0 while i < 2 { i = i + 1 } print(i) return 0 }", vec!["2"]),
        ("and", "fn main() -> i32 { print(1 && 1) return 0 }", vec!["1"]),
        ("or", "fn main() -> i32 { print(0 || 2) return 0 }", vec!["1"]),
        ("not", "fn main() -> i32 { print(!0) return 0 }", vec!["1"]),
        ("var", "fn main() -> i32 { let x = 1 if x { print(9) } return 0 }", vec!["9"]),
    ] {
        let dir = tmpdir(&format!("def-{tag}"));
        let prog = load(&dir, "prog.klang", src);
        check_default(&prog).unwrap_or_else(|ds| {
            panic!("default must stay clean for {tag}: {:?}", ds.iter().map(|d| d.to_json()).collect::<Vec<_>>())
        });
        let (_, out) = run_ok(&prog, "main");
        assert_eq!(out, want_out.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{tag}");
    }
    println!("default mode OK");
}

// ---------------------------------------------------------------------------
// Strict mode: each condition position rejects statically-known Int.
// ---------------------------------------------------------------------------

fn strict_err_contains(src: &str, needles: &[&str]) {
    let dir = tmpdir("strict");
    let prog = load(&dir, "prog.klang", src);
    // Default still accepts it (nothing changes there).
    check_default(&prog).unwrap_or_else(|ds| {
        panic!("default must stay clean: {src}: {:?}", ds.iter().map(|d| d.to_json()).collect::<Vec<_>>())
    });
    let ds = check_strict(&prog).expect_err("strict must reject: {src}");
    assert!(
        ds.iter().any(|d| d.code == "E-TYPE"),
        "want E-TYPE for {src}, got {:?}",
        ds.iter().map(|d| d.to_json()).collect::<Vec<_>>()
    );
    let blob = ds.iter().map(|d| d.to_json()).collect::<Vec<_>>().join("\n");
    for n in needles {
        assert!(blob.contains(n), "{src}: missing {n:?} in:\n{blob}");
    }
}

#[test]
fn strict_rejects_int_if() {
    strict_err_contains(
        "fn main() -> i32 { if 1 { print(7) } return 0 }",
        &["if condition", "bool", "i32"],
    );
    strict_err_contains(
        "fn main() -> i32 { let x = 2 if x { print(7) } return 0 }",
        &["if condition", "bool", "i32"],
    );
    println!("strict if OK");
}

#[test]
fn strict_rejects_int_while() {
    strict_err_contains(
        "fn main() -> i32 { while 1 { break } return 0 }",
        &["while condition", "bool", "i32"],
    );
    strict_err_contains(
        "fn main() -> i32 { let x = 1 while x { x = x + 1 if x > 5 { break } } return x }",
        &["while condition", "bool", "i32"],
    );
    println!("strict while OK");
}

#[test]
fn strict_rejects_int_logic_ops() {
    strict_err_contains(
        "fn main() -> i32 { print(true && 1) return 0 }",
        &["&&", "bool", "i32"],
    );
    strict_err_contains(
        "fn main() -> i32 { print(1 || false) return 0 }",
        &["||", "bool", "i32"],
    );
    strict_err_contains(
        "fn main() -> i32 { print(!0) return 0 }",
        &["!", "bool", "i32"],
    );
    strict_err_contains(
        "fn main() -> i32 { print(1 && 1) return 0 }",
        &["bool", "i32"],
    );
    println!("strict logic OK");
}

#[test]
fn strict_rejects_non_bool_types_by_name() {
    // The error names the offending type, whatever it is. (`if "hi"`
    // already fails in default mode — the strict rule agrees with it and
    // names the type the same way.)
    let dir = tmpdir("strstrict");
    let prog = load(&dir, "prog.klang", "fn main() -> i32 { if \"hi\" { print(1) } return 0 }");
    assert!(check_default(&prog).is_err(), "str conditions fail in default too");
    let ds = check_strict(&prog).expect_err("strict rejects str too");
    let blob = ds.iter().map(|d| d.to_json()).collect::<Vec<_>>().join("\n");
    assert!(blob.contains("if condition") && blob.contains("bool") && blob.contains("str"), "{blob}");
    println!("strict naming OK");
}

#[test]
fn strict_accepts_bool_and_unknown() {
    // Bool everywhere: clean in both modes, runs the same.
    for (tag, src) in [
        ("if", "fn main() -> i32 { if true { print(7) } return 0 }"),
        ("while", "fn main() -> i32 { let b = 1 < 2 while b { b = false } print(1) return 0 }"),
        ("and", "fn main() -> i32 { print(true && false) return 0 }"),
        ("or", "fn main() -> i32 { print(false || true) return 0 }"),
        ("not", "fn main() -> i32 { print(!true) return 0 }"),
    ] {
        let dir = tmpdir(&format!("sok-{tag}"));
        let prog = load(&dir, "prog.klang", src);
        check_default(&prog).expect("default clean");
        check_strict(&prog).unwrap_or_else(|ds| {
            panic!("strict must accept bool {tag}: {:?}", ds.iter().map(|d| d.to_json()).collect::<Vec<_>>())
        });
        run_ok(&prog, "main");
    }
    // Unknown (dynamic map lookup): allowed even in strict — the checker
    // cannot know the type, and the runtime cannot distinguish Bool 1
    // from Int 1. Documented allowance, not a hole for static code.
    let dir = tmpdir("sok-unknown");
    let prog = load(
        &dir,
        "prog.klang",
        "fn main() -> i32 { let m = {\"k\": 1} if m[\"k\"] { print(7) } return 0 }",
    );
    check_default(&prog).expect("default clean");
    check_strict(&prog).unwrap_or_else(|ds| {
        panic!("strict must allow Unknown: {:?}", ds.iter().map(|d| d.to_json()).collect::<Vec<_>>())
    });
    let (_, out) = run_ok(&prog, "main");
    assert_eq!(out, vec!["7".to_string()]);
    println!("strict bool/unknown OK");
}

// ---------------------------------------------------------------------------
// CLI: --strict flag, klang.toml, and flag precedence.
// ---------------------------------------------------------------------------

const INT_PROG: &str = "fn main() -> i32 { if 1 { print(7) } return 0 }\n";
const BOOL_PROG: &str = "fn main() -> i32 { if true { print(7) } return 0 }\n";

#[test]
fn cli_strict_flag() {
    let dir = tmpdir("flag");
    std::fs::write(dir.join("int.klang"), INT_PROG).unwrap();
    std::fs::write(dir.join("bool.klang"), BOOL_PROG).unwrap();
    // No flag: Int program checks clean.
    let (_, _, code) = cli(&dir, &["check", "int.klang"]);
    assert_eq!(code, 0, "default check passes");
    // Flag: Int program fails with E-TYPE naming bool/i32 (`check`
    // prints diagnostics to stdout, `run` to stderr).
    let (stdout, stderr, code) = cli(&dir, &["check", "--strict", "int.klang"]);
    assert_eq!(code, 1);
    let both = format!("{stdout}{stderr}");
    assert!(both.contains("E-TYPE") && both.contains("bool") && both.contains("i32"), "{both}");
    // Flag after the file works too.
    let (stdout, stderr, code) = cli(&dir, &["check", "int.klang", "--strict"]);
    let both = format!("{stdout}{stderr}");
    assert_eq!(code, 1, "{both}");
    // `run --strict` fails the same way (check stage, before execution).
    let (stdout, stderr, code) = cli(&dir, &["run", "--strict", "int.klang"]);
    assert_eq!(code, 1);
    assert!(!stdout.contains('7'), "must not execute: {stdout}");
    assert!(stderr.contains("E-TYPE"), "{stderr}");
    // The flag overrides nothing it should not: a Bool program passes and
    // prints identically with and without the flag.
    let (out_plain, _, code_plain) = cli(&dir, &["run", "bool.klang"]);
    let (out_strict, _, code_strict) = cli(&dir, &["run", "--strict", "bool.klang"]);
    assert_eq!((out_plain.clone(), code_plain), (out_strict, code_strict));
    assert!(out_plain.contains('7'));
    println!("CLI flag OK");
}

#[test]
fn cli_strict_toml() {
    let dir = tmpdir("toml");
    std::fs::write(dir.join("int.klang"), INT_PROG).unwrap();
    // No manifest: default.
    let (_, _, code) = cli(&dir, &["check", "int.klang"]);
    assert_eq!(code, 0);
    // `strict = true` under [project]: enforced with no flag.
    std::fs::write(
        dir.join("klang.toml"),
        "name = \"demo\"\nversion = \"0.1.0\"\nentry = \"main\"\n[project]\nstrict = true\n",
    )
    .unwrap();
    let (stdout, stderr, code) = cli(&dir, &["check", "int.klang"]);
    assert_eq!(code, 1, "toml strict must fail");
    assert!(format!("{stdout}{stderr}").contains("E-TYPE"), "{stdout}{stderr}");
    // ... and for `run` too.
    let (_, stderr, code) = cli(&dir, &["run", "int.klang"]);
    assert_eq!(code, 1, "{stderr}");
    // `strict = false` is default mode.
    std::fs::write(
        dir.join("klang.toml"),
        "name = \"demo\"\nversion = \"0.1.0\"\nentry = \"main\"\n[project]\nstrict = false\n",
    )
    .unwrap();
    let (_, _, code) = cli(&dir, &["check", "int.klang"]);
    assert_eq!(code, 0);
    // A key outside [project] does nothing.
    std::fs::write(
        dir.join("klang.toml"),
        "name = \"demo\"\nversion = \"0.1.0\"\nentry = \"main\"\nstrict = true\n[dependencies]\n",
    )
    .unwrap();
    let (_, _, code) = cli(&dir, &["check", "int.klang"]);
    assert_eq!(code, 0, "top-level strict must not apply");
    println!("TOML OK");
}

#[test]
fn cli_strict_flag_beats_toml() {
    let dir = tmpdir("prec");
    std::fs::write(dir.join("int.klang"), INT_PROG).unwrap();
    std::fs::write(
        dir.join("klang.toml"),
        "[project]\nstrict = true\n",
    )
    .unwrap();
    // Explicit `--strict=false` disables the manifest setting.
    let (_, _, code) = cli(&dir, &["check", "--strict=false", "int.klang"]);
    assert_eq!(code, 0, "explicit false beats toml true");
    // Explicit `--strict=true` enables without any manifest key.
    std::fs::remove_file(dir.join("klang.toml")).unwrap();
    let (_, stderr, code) = cli(&dir, &["check", "--strict=true", "int.klang"]);
    assert_eq!(code, 1, "{stderr}");
    // `--strict` after `--` is program argv, not the flag (no crash,
    // default mode still applies to this Bool-free program shape).
    std::fs::write(
        dir.join("args.klang"),
        "fn main() -> i32 { print(len(args())) print(args()[0]) return 0 }\n",
    )
    .unwrap();
    let (stdout, _, code) = cli(&dir, &["run", "args.klang", "--", "--strict"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--strict"), "{stdout}");
    println!("precedence OK");
}

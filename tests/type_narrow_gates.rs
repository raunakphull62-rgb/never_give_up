//! Phase 1d — `W-TYPE-NARROW` gates, retired by D3 (Wave1 S4).
//!
//! D3 decision: `Int` is 64-bit with checked arithmetic and every integer
//! spelling (`i32`, `i64`, `u32`, `u64`, `u8`) is an honest checked type —
//! the narrower spellings enforce their range at value boundaries
//! (parameter passing, return, assignment) with `E-RUNTIME`. There is
//! nothing left to warn about that is now correct, so
//! `narrow_type_warnings` always returns empty. These gates pin that
//! retirement: no annotation (and no stdlib package) warns anymore.

use klang::parser::Parser;

fn lint(src: &str) -> Vec<klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    // Behavior is unchanged: narrowing programs still check clean.
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "narrowing must not fail check"
    );
    klang::hir::narrow_type_warnings(&prog, "probe.klang")
}

#[test]
fn narrow_aliases_no_longer_warn() {
    // D3: `i64`/`u32`/`u64` are honest checked spellings now — silent.
    // (Was: one `W-TYPE-NARROW` warning per annotation.)
    let ds = lint("fn f(x: i64, y: u32) -> u64 { return 1 } fn main() -> i32 { return 0 }");
    assert!(ds.is_empty(), "D3 retired the warning: {}", dump(&ds));
    println!("narrow aliases silent OK");
}

#[test]
fn u8_no_longer_warns_as_dynamic() {
    // D3: `u8` is a checked type (0..255 at boundaries) — silent.
    // (Was: one `W-TYPE-NARROW` "stays dynamic" warning.)
    let ds = lint("fn f(x: u8) -> i32 { return 1 } fn main() -> i32 { return 0 }");
    assert!(ds.is_empty(), "D3 retired the warning: {}", dump(&ds));
    println!("u8 checked OK");
}

#[test]
fn honest_annotations_stay_silent() {
    // Checked types, void, custom nominal types, type parameters, and
    // qualified module paths never warn.
    let ds = lint(
        "struct Box { x: i32 } enum Opt { Some(x: i32), None } \
         fn id<T>(x: T) -> T { return x } \
         fn f(a: i32, b: f64, c: str, d: bool, e: Box, g: Opt, h: json) -> void { print(a) } \
         fn main() -> i32 { return 0 }",
    );
    assert!(ds.is_empty(), "{}", dump(&ds));
    // A user type that reuses an alias spelling resolves nominally, so it
    // does not narrow either (mirrors `resolve_param_ty`).
    let ds = lint("struct i64 { x: i32 } fn f(a: i64) -> i32 { return 1 } fn main() -> i32 { return 0 }");
    assert!(ds.is_empty(), "{}", dump(&ds));
    println!("honest annotations OK");
}

#[test]
fn no_warnings_in_fields_variants_and_closures() {
    // Struct fields, enum payloads, closure literals, and closure-type
    // spellings are annotation positions too — all silent under D3.
    // (Was: 4 warnings for the first program, 1 for the closure-type one.)
    let ds = lint(
        "struct S { n: i64 } enum E { V(x: u8) } \
         fn main() -> i32 { let f = fn(x: i64) -> i64 { return x } return f(1) }",
    );
    assert!(ds.is_empty(), "D3 retired the warnings: {}", dump(&ds));
    // `fn(i64) -> i32` written as a parameter type.
    let ds = lint("fn apply(f: fn(i64) -> i32, x: i32) -> i32 { return f(x) } fn main() -> i32 { return 0 }");
    assert!(ds.is_empty(), "D3 retired the warning: {}", dump(&ds));
    // Generic arguments never warned without a narrow spelling.
    let ds = lint("enum Opt<T> { Some(x: T), None } fn main() -> i32 { let o = Opt::Some(1) return 1 }");
    assert!(ds.is_empty(), "no warning without a narrow spelling: {}", dump(&ds));
    println!("fields/variants/closures OK");
}

#[test]
fn no_stdlib_package_triggers_narrow() {
    // Phase 1d acceptance, kept under D3: walk every `.klang` file under
    // the 26 published packages and prove none of them would warn.
    // (Published versions are immutable, so a warning there would be
    // unactionable noise — and now there are no warnings at all.)
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("stdlib-packages");
    let mut pkgs = std::collections::HashSet::new();
    let mut files = 0;
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for ent in std::fs::read_dir(&dir).expect("stdlib-packages walks") {
            let ent = ent.expect("dir entry");
            let path = ent.path();
            if path.is_dir() {
                // Skip build/vendor residue if present.
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == ".klang_pkgs" || name == "target" {
                        continue;
                    }
                }
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("klang") {
                files += 1;
                if let Ok(rel) = path.strip_prefix(&root) {
                    if let Some(pkg) = rel.components().next() {
                        pkgs.insert(pkg.as_os_str().to_owned());
                    }
                }
                let src = std::fs::read_to_string(&path).expect("klang file reads");
                let mut p = Parser::new(&src);
                let prog = p.parse_program().expect("published package parses");
                let label = path.to_string_lossy().to_string();
                let ds = klang::hir::narrow_type_warnings(&prog, &label);
                assert!(ds.is_empty(), "{label} warns: {}", dump(&ds));
            }
        }
    }
    assert_eq!(pkgs.len(), 26, "all published packages walked, got {pkgs:?}");
    assert!(files > 26, "more than one file per package, got {files}");
    println!("stdlib narrow-clean OK: {} packages, {} files", pkgs.len(), files);
}

fn dump(ds: &[klang::diagnostics::Diagnostic]) -> String {
    ds.iter().map(|d| d.to_json()).collect::<Vec<_>>().join("\n")
}

#[test]
fn narrow_lint_emits_nothing() {
    // The retired lint never emits any diagnostic for any spelling
    // (future-proof against new spellings sneaking back in as warnings).
    assert!(lint("fn f(x: i64) -> i64 { return x } fn main() -> i32 { return 0 }").is_empty());
    assert!(lint("fn f(x: u8) -> u8 { return x } fn main() -> i32 { return 0 }").is_empty());
}

//! Phase 1d — `W-TYPE-NARROW` gates.
//!
//! `i64`/`u32`/`u64` annotations are accepted but enforce the `i32` range,
//! and `u8` is an unchecked `Unknown`. Behavior is unchanged (everything
//! still compiles and runs as before); the compiler now says so out loud
//! with a `warning`-severity diagnostic that never fails `check`.

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

fn codes(ds: &[klang::diagnostics::Diagnostic]) -> Vec<String> {
    ds.iter().map(|d| d.code.clone()).collect()
}

#[test]
fn narrow_aliases_warn_with_enforced_range() {
    // One warning per narrowing annotation, naming the `i32` range and a fix.
    let ds = lint("fn f(x: i64, y: u32) -> u64 { return 1 } fn main() -> i32 { return 0 }");
    assert_eq!(ds.len(), 3, "{}", dump(&ds));
    for d in &ds {
        assert_eq!(d.code, "W-TYPE-NARROW", "{}", d.to_json());
        assert_eq!(d.severity, "warning", "{}", d.to_json());
        assert!(d.message.contains("Klang enforces the `i32` range"), "{}", d.to_json());
        assert!(
            d.fixes.iter().any(|f| f.label.contains("use `i32`")),
            "{}",
            d.to_json()
        );
    }
    assert!(ds[0].message.contains("parameter `x`"), "{}", ds[0].to_json());
    assert!(ds[1].message.contains("parameter `y`"), "{}", ds[1].to_json());
    assert!(ds[2].message.contains("return type"), "{}", ds[2].to_json());
    println!("narrow aliases OK");
}

#[test]
fn unchecked_u8_warns_as_dynamic() {
    let ds = lint("fn f(x: u8) -> i32 { return 1 } fn main() -> i32 { return 0 }");
    assert_eq!(ds.len(), 1, "{}", dump(&ds));
    assert_eq!(ds[0].code, "W-TYPE-NARROW", "{}", ds[0].to_json());
    assert_eq!(ds[0].severity, "warning", "{}", ds[0].to_json());
    assert!(ds[0].message.contains("`u8`"), "{}", ds[0].to_json());
    assert!(ds[0].message.contains("stays dynamic"), "{}", ds[0].to_json());
    println!("u8 unchecked OK");
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
fn narrow_warns_in_fields_variants_and_closures() {
    // Struct fields, enum payloads, closure literals, and closure-type
    // spellings are annotation positions too.
    let ds = lint(
        "struct S { n: i64 } enum E { V(x: u8) } \
         fn main() -> i32 { let f = fn(x: i64) -> i64 { return x } return f(1) }",
    );
    assert_eq!(ds.len(), 4, "{}", dump(&ds));
    assert!(ds.iter().any(|d| d.message.contains("field `n` of struct `S`")), "{}", dump(&ds));
    assert!(
        ds.iter().any(|d| d.message.contains("variant `V` in enum `E`")),
        "{}",
        dump(&ds)
    );
    assert!(ds.iter().any(|d| d.message.contains("closure")), "{}", dump(&ds));
    // `fn(i64) -> i32` written as a parameter type.
    let ds = lint("fn apply(f: fn(i64) -> i32, x: i32) -> i32 { return f(x) } fn main() -> i32 { return 0 }");
    assert_eq!(ds.len(), 1, "{}", dump(&ds));
    assert!(ds[0].message.contains("`i64`"), "{}", ds[0].to_json());
    // Generic arguments narrow at instantiation (`Opt<i64>` holds `i32`s).
    let ds = lint("enum Opt<T> { Some(x: T), None } fn main() -> i32 { let o = Opt::Some(1) return 1 }");
    assert!(ds.is_empty(), "no warning without a narrow spelling: {}", dump(&ds));
    println!("fields/variants/closures OK");
}

#[test]
fn no_stdlib_package_triggers_narrow() {
    // Phase 1d acceptance: walk every `.klang` file under the 26 published
    // packages and prove none of them would warn. (Published versions are
    // immutable, so a warning there would be unactionable noise.)
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
fn narrow_codes_stay_warnings() {
    // Every diagnostic this lint can ever emit is a warning (future-proof
    // against new spellings sneaking in as errors).
    assert!(codes(&lint("fn f(x: i64) -> i64 { return x } fn main() -> i32 { return 0 }"))
        .iter()
        .all(|c| c == "W-TYPE-NARROW"));
}

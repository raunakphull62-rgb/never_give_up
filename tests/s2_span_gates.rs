//! Wave1 S2a — real diagnostic locations.
//!
//! Every test uses its own unique temp dir (pid + atomic counter).
//! Each diagnostic is asserted to carry a real file, non-zero byte span,
//! and 1-based line/column matching the source text at that span
//! (extracted text contains the expected token).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static CTR: AtomicU64 = AtomicU64::new(0);

fn tmpdir(tag: &str) -> PathBuf {
    let c = CTR.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("klang-s2-span-{}-{}-{}", tag, std::process::id(), c));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &PathBuf, name: &str, src: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, src).unwrap();
    p.to_string_lossy().to_string()
}

fn assert_real_span(diag: &klang::diagnostics::Diagnostic, expected_file: &str, expected_token: &str) {
    assert_eq!(diag.primary_span.file, expected_file, "file: {}", diag.to_json());
    assert_ne!(
        (diag.primary_span.start, diag.primary_span.end),
        (0, 0),
        "span must be non-zero: {}",
        diag.to_json()
    );
    assert!(diag.primary_span.end > diag.primary_span.start, "{}", diag.to_json());
    let text = std::fs::read_to_string(expected_file).expect("diag file must exist");
    assert!(
        diag.primary_span.end <= text.len(),
        "span end {} past file len {}: {}",
        diag.primary_span.end,
        text.len(),
        diag.to_json()
    );
    let (exp_line, exp_col) = klang::diagnostics::line_col_for(&text, diag.primary_span.start);
    assert_eq!(diag.primary_span.line, exp_line, "line: {}", diag.to_json());
    assert_eq!(diag.primary_span.column, exp_col, "column: {}", diag.to_json());
    assert!(diag.primary_span.line >= 1 && diag.primary_span.column >= 1, "{}", diag.to_json());
    let slice = &text[diag.primary_span.start..diag.primary_span.end];
    assert!(
        slice.contains(expected_token),
        "span text {slice:?} must contain {expected_token:?}: {}",
        diag.to_json()
    );
}

#[test]
fn s2_parse_error_has_real_span() {
    let dir = tmpdir("parse");
    let src = "fn main() -> i32 { return @ }\n";
    let path = write(&dir, "main.klang", src);
    let mut p = klang::parser::Parser::new_with_file(src, &path);
    let d = p.parse_program().expect_err("must fail parse");
    assert_eq!(d.code, "E-PARSE");
    assert_real_span(&d, &path, "@");
}

#[test]
fn s2_import_unresolved_has_real_span() {
    let dir = tmpdir("import-missing");
    let src = "import \"missing.klang\"\nfn main() -> i32 { return 0 }\n";
    let path = write(&dir, "main.klang", src);
    let e = klang::imports::load_program(&path).expect_err("load must fail");
    assert_eq!(e.code, "E-IO-NOT-FOUND");
    let j = e.to_json();
    assert!(j.contains("\"line\""), "{j}");
    // Rehydrate to Diagnostic to reuse the span assertions.
    let v = klang::json::parse_json(&j).expect("json");
    let file = v.get("primary_span").and_then(|s| s.get("file")).and_then(|f| f.as_str()).unwrap();
    assert_eq!(file, path.as_str());
    // Direct field assertions (no server round-trip needed).
    assert_ne!((e.start, e.end), (0, 0), "{j}");
    let text = std::fs::read_to_string(&path).unwrap();
    let (el, ec) = klang::diagnostics::line_col_for(&text, e.start);
    assert_eq!((e.line, e.column), (el, ec), "{j}");
    assert!(text[e.start..e.end].contains("missing.klang"), "{j}");
}

#[test]
fn s2_import_error_across_two_files_names_importer() {
    // entry -> lib -> missing: the failure must point at lib (the importer),
    // not the entry file.
    let dir = tmpdir("import-chain");
    let lib_src = "import \"missing2.klang\"\nfn libfn() -> i32 { return 1 }\n";
    let lib_path = write(&dir, "lib.klang", lib_src);
    let entry_src = "import \"lib.klang\"\nfn main() -> i32 { return libfn() }\n";
    let entry_path = write(&dir, "main.klang", entry_src);
    let e = klang::imports::load_program(&entry_path).expect_err("chain must fail");
    assert_eq!(e.code, "E-IO-NOT-FOUND");
    assert_eq!(e.file, lib_path, "must name importer lib, got {}", e.to_json());
    assert_ne!((e.start, e.end), (0, 0));
    let text = std::fs::read_to_string(&lib_path).unwrap();
    assert!(text[e.start..e.end].contains("missing2.klang"), "{}", e.to_json());
    let (el, ec) = klang::diagnostics::line_col_for(&text, e.start);
    assert_eq!((e.line, e.column), (el, ec));
}

#[test]
fn s2_type_error_has_real_span() {
    let dir = tmpdir("type");
    let src = "fn main() -> i32 { return \"hi\" - 1 }\n";
    let path = write(&dir, "main.klang", src);
    let mut p = klang::parser::Parser::new_with_file(src, &path);
    let prog = p.parse_program().expect("parses");
    let ds = klang::hir::TypedHIR::check_with_file(prog, &path).expect_err("must fail");
    let d = ds.iter().find(|d| d.code == "E-TYPE").expect("E-TYPE");
    assert_real_span(d, &path, "-");
}

#[test]
fn s2_undefined_has_real_span() {
    let dir = tmpdir("undef");
    let src = "fn main() -> i32 { return nope_xyz + 1 }\n";
    let path = write(&dir, "main.klang", src);
    let mut p = klang::parser::Parser::new_with_file(src, &path);
    let prog = p.parse_program().expect("parses");
    let ds = klang::hir::TypedHIR::check_with_file(prog, &path).expect_err("must fail");
    let d = ds.iter().find(|d| d.code == "E-UNDEFINED").expect("E-UNDEFINED");
    assert_real_span(d, &path, "nope_xyz");
}

#[test]
fn s2_arity_has_real_span() {
    let dir = tmpdir("arity");
    let src = "fn addit(a: i32, b: i32) -> i32 { return a + b }\nfn main() -> i32 { return addit(1) }\n";
    let path = write(&dir, "main.klang", src);
    let mut p = klang::parser::Parser::new_with_file(src, &path);
    let prog = p.parse_program().expect("parses");
    let ds = klang::hir::TypedHIR::check_with_file(prog, &path).expect_err("must fail");
    let d = ds.iter().find(|d| d.code == "E-ARITY").expect("E-ARITY");
    assert_real_span(d, &path, "addit");
}

#[test]
fn s2_duplicate_param_has_real_span() {
    let dir = tmpdir("dup");
    let src = "fn f(dupparam: i32, dupparam: i32) -> i32 { return dupparam }\nfn main() -> i32 { return f(1, 2) }\n";
    let path = write(&dir, "main.klang", src);
    let mut p = klang::parser::Parser::new_with_file(src, &path);
    let prog = p.parse_program().expect("parses");
    let ds = klang::hir::TypedHIR::check_with_file(prog, &path).expect_err("must fail");
    let d = ds.iter().find(|d| d.code == "E-DUPLICATE").expect("E-DUPLICATE");
    assert_real_span(d, &path, "dupparam");
}

#[test]
fn s2_match_exhaustive_has_real_span() {
    let dir = tmpdir("match");
    let src = "enum OptS2 { Some(x: i32), None }\nfn f(v: OptS2) -> i32 { return match v { OptS2::Some(n) => n } }\nfn main() -> i32 { return 0 }\n";
    let path = write(&dir, "main.klang", src);
    let mut p = klang::parser::Parser::new_with_file(src, &path);
    let prog = p.parse_program().expect("parses");
    let ds = klang::hir::TypedHIR::check_with_file(prog, &path).expect_err("must fail");
    let d = ds.iter().find(|d| d.code == "E-MATCH-EXHAUSTIVE").expect("exhaustive");
    assert_real_span(d, &path, "match");
}

#[test]
fn s2_strict_mode_has_real_span() {
    let dir = tmpdir("strict");
    let src = "fn main() -> i32 { if 1 { print(7) } return 0 }\n";
    let path = write(&dir, "main.klang", src);
    let mut p = klang::parser::Parser::new_with_file(src, &path);
    let prog = p.parse_program().expect("parses");
    // Default mode stays clean.
    assert!(klang::hir::TypedHIR::check_with_file_strict(prog.clone(), &path, false).is_ok());
    let ds = klang::hir::TypedHIR::check_with_file_strict(prog, &path, true).expect_err("strict must fail");
    let d = ds.iter().find(|d| d.code == "E-TYPE").expect("E-TYPE");
    assert_real_span(d, &path, "if");
}

#[test]
fn s2_d3_boundary_has_real_span() {
    use std::collections::HashMap;
    let dir = tmpdir("d3");
    let src = "fn takex(x: i32) -> i32 { return x }\nfn main() -> i32 { return takex(3000000000) }\n";
    let path = write(&dir, "main.klang", src);
    let loaded = klang::imports::load_program(&path).expect("loads");
    assert!(klang::hir::TypedHIR::check_with_file(loaded.program.clone(), &path).is_ok());
    let mir = klang::mir::lower(&loaded.program);
    let d = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("D3 must fail");
    assert_eq!(d.code, "E-RUNTIME");
    assert_real_span(&d, &path, "takex");
}

#[test]
fn s2_error_inside_imported_file_names_that_file() {
    let dir = tmpdir("imported-err");
    let lib_src = "fn libbad() -> i32 { return \"hi\" - 1 }\n";
    let lib_path = write(&dir, "lib.klang", lib_src);
    let entry_src = "import \"lib.klang\"\nfn main() -> i32 { return libbad() }\n";
    let entry_path = write(&dir, "main.klang", entry_src);
    let loaded = klang::imports::load_program(&entry_path).expect("loads");
    let ds = klang::hir::TypedHIR::check_with_file(loaded.program, &entry_path).expect_err("must fail");
    let d = ds.iter().find(|d| d.code == "E-TYPE").expect("E-TYPE in lib");
    // Must name the imported file, NOT the entry file.
    assert_eq!(d.primary_span.file, lib_path, "{}", d.to_json());
    assert_ne!(d.primary_span.file, entry_path);
    let text = std::fs::read_to_string(&lib_path).unwrap();
    let (el, ec) = klang::diagnostics::line_col_for(&text, d.primary_span.start);
    assert_eq!((d.primary_span.line, d.primary_span.column), (el, ec));
    assert!(text[d.primary_span.start..d.primary_span.end].contains("-"), "{}", d.to_json());
}

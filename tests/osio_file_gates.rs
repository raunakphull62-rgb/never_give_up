//! STDLIB-OSIO Phase 1 — File I/O gates.
//!
//! Covers `read_file` / `write_file` / `append_file` / `exists` per the
//! PRD: read-present, read-missing (`E-IO-NOT-FOUND`), write-then-read
//! roundtrip, double-append landing, `exists` true/false, plus the
//! `E-IO-*` code specificity and the unchanged unsafe-path guard
//! (`E-RUNTIME`, distinct from I/O failures).

use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new())
}

fn tmp_path(name: &str) -> String {
    let dir = std::env::temp_dir().join("klang-osio-file");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name).to_string_lossy().replace('\\', "/")
}

#[test]
fn osio_file_read_present() {
    let path = tmp_path("read_present.txt");
    std::fs::write(&path, "hello osio").unwrap();
    let src = format!("fn main() -> i32 {{ print(read_file(\"{path}\")) return 1 }}");
    let (v, out) = run_src(&src, "main").expect("runs");
    assert_eq!(out, vec!["hello osio".to_string()]);
    assert_eq!(v, 1);
    std::fs::remove_file(&path).ok();
}

#[test]
fn osio_file_read_missing_is_not_found() {
    let path = tmp_path("read_missing_xyz_12345.txt");
    let _ = std::fs::remove_file(&path);
    let src = format!("fn main() -> i32 {{ print(read_file(\"{path}\")) return 0 }}");
    let err = run_src(&src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn osio_file_write_then_read_roundtrip() {
    let path = tmp_path("roundtrip.txt");
    let _ = std::fs::remove_file(&path);
    let src = format!(
        "fn main() -> i32 {{ let n = write_file(\"{path}\", \"abc 123\") if read_file(\"{path}\") == \"abc 123\" {{ return n }} return 0 }}"
    );
    let (v, _) = run_src(&src, "main").expect("runs");
    assert_eq!(v, 7, "write_file returns byte count of content");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "abc 123");
    std::fs::remove_file(&path).ok();
}

#[test]
fn osio_file_append_twice_both_land() {
    let path = tmp_path("append_twice.txt");
    let _ = std::fs::remove_file(&path);
    let src = format!(
        "fn main() -> i32 {{ write_file(\"{path}\", \"a\") append_file(\"{path}\", \"b\") append_file(\"{path}\", \"c\") print(read_file(\"{path}\")) return 1 }}"
    );
    let (_, out) = run_src(&src, "main").expect("runs");
    assert_eq!(out, vec!["abc".to_string()]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "abc");
    std::fs::remove_file(&path).ok();
}

#[test]
fn osio_file_append_creates_when_absent() {
    let path = tmp_path("append_creates.txt");
    let _ = std::fs::remove_file(&path);
    let src = format!(
        "fn main() -> i32 {{ let n = append_file(\"{path}\", \"new\") print(read_file(\"{path}\")) return n }}"
    );
    let (v, out) = run_src(&src, "main").expect("runs");
    assert_eq!(out, vec!["new".to_string()]);
    assert_eq!(v, 3);
    std::fs::remove_file(&path).ok();
}

#[test]
fn osio_file_exists_true_and_false() {
    let real = tmp_path("exists_real.txt");
    std::fs::write(&real, "x").unwrap();
    let missing = tmp_path("exists_missing_xyz_12345.txt");
    let _ = std::fs::remove_file(&missing);
    let src = format!(
        "fn main() -> i32 {{ if exists(\"{real}\") {{ if exists(\"{missing}\") {{ return 99 }} return 42 }} return 0 }}"
    );
    let (v, _) = run_src(&src, "main").expect("runs");
    assert_eq!(v, 42);
    std::fs::remove_file(&real).ok();
}

#[test]
fn osio_file_error_codes_are_specific() {
    // Rust-level mapping: NotFound -> E-IO-NOT-FOUND, PermissionDenied ->
    // E-IO-PERMISSION, other -> E-IO-FAILED with the real OS message.
    let nf = std::io::Error::new(std::io::ErrorKind::NotFound, "entity not found");
    let d = klang::stdlib::file::map_io_error("read_file", "/tmp/x", &nf);
    assert_eq!(d.code, "E-IO-NOT-FOUND");
    let pd = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied xyz");
    let d = klang::stdlib::file::map_io_error("write_file", "/tmp/x", &pd);
    assert_eq!(d.code, "E-IO-PERMISSION");
    assert!(
        d.to_json().contains("denied xyz"),
        "real cause, not generic: {}",
        d.to_json()
    );
    let other = std::io::Error::new(std::io::ErrorKind::StorageFull, "no space abc123");
    let d = klang::stdlib::file::map_io_error("write_file", "/tmp/x", &other);
    assert_eq!(d.code, "E-IO-FAILED");
    assert!(
        d.to_json().contains("no space abc123"),
        "real cause, not generic: {}",
        d.to_json()
    );
    // Distinct causes per code (never one shared generic string).
    let a = klang::stdlib::file::map_io_error(
        "read_file",
        "/tmp/x",
        &std::io::Error::new(std::io::ErrorKind::NotFound, "nf"),
    );
    let b = klang::stdlib::file::map_io_error(
        "read_file",
        "/tmp/x",
        &std::io::Error::new(std::io::ErrorKind::PermissionDenied, "pd"),
    );
    assert_ne!(a.cause, b.cause);
}

#[test]
fn osio_file_unsafe_absolute_path_stays_runtime_error() {
    // The capability guard (`reject_unsafe_path`) is distinct from I/O
    // failures: absolute paths outside the temp dir are E-RUNTIME, not
    // E-IO-*. This pins the guard so the E-IO-* change cannot silently
    // widen filesystem access.
    let src =
        "fn main() -> i32 { print(read_file(\"/no/such/klang-file-xyz-outside-tmp\")) return 0 }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-RUNTIME", "got: {}", err.to_json());
}

#[test]
fn osio_file_append_arg_types_checked() {
    let mut p = Parser::new("fn main() -> i32 { return append_file(42, \"x\") }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { return append_file(\"a\") }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
}

#[test]
fn osio_file_remove_existing_then_exists_false() {
    let path = tmp_path("remove_existing.txt");
    std::fs::write(&path, "to delete").unwrap();
    let src = format!(
        "fn main() -> i32 {{ remove_file(\"{path}\") if exists(\"{path}\") {{ return 1 }} return 42 }}"
    );
    let (v, _) = run_src(&src, "main").expect("runs");
    assert_eq!(v, 42);
    assert!(
        !std::path::Path::new(&path).exists(),
        "file must actually be gone from the filesystem"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn osio_file_remove_missing_is_not_found() {
    let path = tmp_path("remove_missing_xyz_12345.txt");
    let _ = std::fs::remove_file(&path);
    let src = format!("fn main() -> i32 {{ remove_file(\"{path}\") return 0 }}");
    let err = run_src(&src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-IO-NOT-FOUND", "got: {}", err.to_json());
}

#[test]
fn osio_file_remove_unsafe_absolute_path_stays_runtime_error() {
    // Same capability guard as reads/writes (`reject_unsafe_path`):
    // absolute paths outside the temp dir are E-RUNTIME, not E-IO-*.
    let src =
        "fn main() -> i32 { remove_file(\"/no/such/klang-file-xyz-outside-tmp\") return 0 }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-RUNTIME", "got: {}", err.to_json());
}

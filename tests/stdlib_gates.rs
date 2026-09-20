use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> (i32, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("runs")
}

#[test]
fn stdlib_file_roundtrip() {
    let dir = std::env::temp_dir().join("klang-stdlib-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("roundtrip.txt");
    let path_s = path.to_string_lossy().replace('\\', "/");
    let src = format!(
        "fn main() -> i32 {{ let n = write_file(\"{path_s}\", \"hello klang\") print(read_file(\"{path_s}\")) if exists(\"{path_s}\") {{ return n }} return 0 }}"
    );
    let (v, out) = run_src(&src, "main");
    assert_eq!(out, vec!["hello klang".to_string()]);
    assert_eq!(v, 11, "write_file returns byte count");
    std::fs::remove_file(&path).ok();
}

#[test]
fn stdlib_env_reads_process_env() {
    std::env::set_var("KLANG_STDLIB_PROBE", "42");
    let (v, out) = run_src(
        "fn main() -> i32 { print(env(\"KLANG_STDLIB_PROBE\")) return int(env(\"KLANG_STDLIB_PROBE\")) }",
        "main",
    );
    assert_eq!(out, vec!["42".to_string()]);
    assert_eq!(v, 42);
}

#[test]
fn stdlib_missing_file_is_runtime_error() {
    let mut p = Parser::new(
        "fn main() -> i32 { print(read_file(\"/no/such/klang-file-xyz\")) return 0 }",
    );
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    assert_eq!(err.code, "E-RUNTIME");
}

#[test]
fn stdlib_arg_types_checked() {
    let mut p = Parser::new("fn main() -> i32 { return read_file(42) }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { return write_file(\"a\", 1) }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
}

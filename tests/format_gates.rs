//! FOUNDATION-1 Phase 2 — `format(...)` string builder.
//!
//! Variadic builtin returning `str`: each argument renders exactly like
//! `str()`/`print` (`Value::render`), parts joined with a single space.
//! Covers v1 (`run`) and v2 (`run-v2`); the JIT rejects it loudly like
//! every other `str` builtin (int-only backend, never silent code).

use std::collections::HashMap;

use klang::parser::Parser;

fn run_v1_value(src: &str) -> (klang::runtime::Value, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new())
        .expect("runs")
}

fn check_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(
        codes.contains(&want),
        "want {want} in {codes:?}: {src}"
    );
}

fn run_v2(src: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    klang::runtime::v2::run_v2_program(&prog, "main")
}

// ---- v1: the PRD minimum (i32 + f64 + str in one output string) ----

#[test]
fn format_interpolates_int_float_str() {
    let (v, _) = run_v1_value(
        "fn main() -> str { return format(\"age:\", 42, \"pi:\", 3.5, \"name:\", \"klang\") }",
    );
    assert_eq!(v.render(), "age: 42 pi: 3.5 name: klang");
}

#[test]
fn format_example_file_runs() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/examples/format.klang"
    ))
    .expect("example must exist");
    let mut p = Parser::new(&src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (code, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(code, 0);
    assert_eq!(out, vec!["age: 42 pi: 3.5 name: klang".to_string()]);
}

// ---- v1: edge shapes ----

#[test]
fn format_zero_args_is_empty() {
    let (v, _) = run_v1_value("fn main() -> str { return format() }");
    assert_eq!(v.render(), "");
}

#[test]
fn format_single_arg_passes_through() {
    let (v, _) = run_v1_value("fn main() -> str { return format(\"hi\") }");
    assert_eq!(v.render(), "hi");
    let (v, _) = run_v1_value("fn main() -> str { return format(42) }");
    assert_eq!(v.render(), "42");
    // Whole floats keep `.0`, same rule as `str()`/`print`.
    let (v, _) = run_v1_value("fn main() -> str { return format(2.0) }");
    assert_eq!(v.render(), "2.0");
}

#[test]
fn format_renders_non_scalar_values() {
    let (v, _) = run_v1_value("fn main() -> str { return format(true, [1, 2]) }");
    assert_eq!(v.render(), "1 [1, 2]");
    let (v, _) = run_v1_value("fn main() -> str { return format({a: 1}) }");
    assert_eq!(v.render(), "{\"a\": 1}");
}

#[test]
fn format_result_is_str_typed() {
    // A `format(...)` value flows as `str`: concat accepts it, an `i32`
    // slot rejects it with `E-TYPE`.
    let mut p = Parser::new("fn main() -> str { return format(\"a\", 1) + \"!\" }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
    check_err_code(
        "fn take(x: i32) -> i32 { return x } fn main() -> i32 { return take(format(\"a\")) }",
        "E-TYPE",
    );
}

#[test]
fn format_result_is_assignable_and_printable() {
    let (_, out) = run_v1_value(
        "fn main() -> str { let s = format(\"n=\", 7) print(s) return s }",
    );
    assert_eq!(out, vec!["n= 7".to_string()]);
}

// ---- v2: same builtin, same rule (no f64 values exist in v2 yet) ----

#[test]
fn format_v2_interpolates_int_and_str() {
    let src = r#"
fn main() -> i32 {
    print(format("age:", 42, "name:", "klang"))
    return 0
}
"#;
    let (v, out) = run_v2(src).expect("runs");
    assert_eq!(v, 0);
    assert_eq!(out, vec!["age: 42 name: klang".to_string()]);
}

#[test]
fn format_v2_zero_args_is_empty() {
    let src = r#"
fn main() -> i32 {
    print(format())
    return 0
}
"#;
    let (_, out) = run_v2(src).expect("runs");
    assert_eq!(out, vec![String::new()]);
}

// ---- backend boundary: JIT stays int-only and loud ----

#[test]
fn jit_rejects_format_builtin() {
    let mut p = Parser::new("fn main() -> i32 {\n    print(format(\"a\", 1))\n    return 0\n}\n");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported"), "{err}");
}

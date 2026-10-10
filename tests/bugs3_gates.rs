use std::collections::HashMap;

use klang::parser::Parser;

fn run_value(src: &str) -> (klang::runtime::Value, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new()).expect("runs")
}

fn run_err_code(src: &str) -> klang::diagnostics::Diagnostic {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must overflow")
}

#[test]
fn bugs3_unicode_escape_decoded() {
    // Exact repro from the report.
    let (v, out) = {
        let mut p = Parser::new(
            "fn main() -> i32 { let s = \"A\u{5c}u0041B\" print(s) print(len(s)) return 0 }",
        );
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
        let mir = klang::mir::lower(&prog);
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
        // Re-run via value channel for output inspection.
        let (_, out) =
            klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
        (0, out)
    };
    let _ = v;
    assert_eq!(out, vec!["AAB".to_string(), "3".to_string()]);
}

#[test]
fn bugs3_unicode_escape_positions() {
    // Escape at start, middle, end, plus multiple escapes in one string.
    let src = "fn main() -> i32 { let a = \"\u{5c}u0041BC\" let b = \"A\u{5c}u0041C\" let c = \"AB\u{5c}u0041\" let d = \"\u{5c}u0041\u{5c}u0042\u{5c}u0043\" print(a) print(b) print(c) print(d) print(len(d)) return 0 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (_, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(
        out,
        vec![
            "ABC".to_string(),
            "AAC".to_string(),
            "ABA".to_string(),
            "ABC".to_string(),
            "3".to_string(),
        ]
    );
}

#[test]
fn bugs3_other_escapes_still_work() {
    let src = "fn main() -> i32 { let s = \"a\u{5c}nb\u{5c}tc\u{5c}rd\u{5c}\u{5c}e\u{5c}\"f\" print(s) return 0 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (_, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(out, vec!["a\nb\tc\rd\\e\"f".to_string()]);
}

#[test]
fn bugs3_add_overflow_is_error() {
    // D3: the boundary moved from i32 to i64 (was `2147483647 + 1`).
    let d = run_err_code("fn main() -> i32 { let a = 9223372036854775807 let b = a + 1 print(b) return 0 }");
    assert_eq!(d.code, "E-OVERFLOW", "{}", d.to_json());
    assert_eq!(d.rule, "arithmetic/overflow");
}

#[test]
fn bugs3_sub_overflow_is_error() {
    // D3: the boundary moved from i32 to i64 (was `-2147483648 - 1`).
    let d = run_err_code("fn main() -> i32 { let a = -9223372036854775808 let b = a - 1 print(b) return 0 }");
    assert_eq!(d.code, "E-OVERFLOW", "{}", d.to_json());
}

#[test]
fn bugs3_mul_overflow_is_error() {
    // D3: the boundary moved from i32 to i64 (was `2147483647 * 2`).
    let d = run_err_code("fn main() -> i32 { let a = 9223372036854775807 let b = a * 2 print(b) return 0 }");
    assert_eq!(d.code, "E-OVERFLOW", "{}", d.to_json());
}

#[test]
fn bugs3_div_overflow_is_error() {
    // D3: i64::MIN / -1 is the division overflow edge case (was i32::MIN / -1).
    let d = run_err_code("fn main() -> i32 { let a = -9223372036854775808 let b = a / -1 print(b) return 0 }");
    assert_eq!(d.code, "E-OVERFLOW", "{}", d.to_json());
}

#[test]
fn bugs3_in_range_arithmetic_still_runs() {
    let mut p = Parser::new("fn main() -> i32 { return 20 + 22 }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn bugs3_f64_return_renders_fraction() {
    // Exact repro: same value, print vs summary must agree.
    let (v, out) = run_value("fn main() -> f64 { let x = 42.0 print(x) return x }");
    assert_eq!(out, vec!["42.0".to_string()]);
    assert_eq!(v.render(), "42.0");
}

#[test]
fn bugs3_f64_nonwhole_return_renders() {
    let (v, out) = run_value("fn main() -> f64 { let x = 3.14 print(x) return x }");
    assert_eq!(out, vec!["3.14".to_string()]);
    assert_eq!(v.render(), "3.14");
}

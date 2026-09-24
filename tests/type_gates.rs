use std::collections::HashMap;

use klang::parser::Parser;

fn check_err_code(src: &str, code: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(
        err.iter().any(|d| d.code == code),
        "want {code}, got {:?}",
        err.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    println!("{code} OK: {}", err[0].to_json());
}

fn check_clean(src: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog).is_ok(),
        "must check clean: {src}"
    );
}

#[test]
fn type_arith_mismatch_is_error() {
    check_err_code("fn main() -> i32 { return \"hi\" - 1 }", "E-TYPE");
    check_err_code("fn main() -> i32 { return 1 + true }", "E-TYPE");
    check_err_code("fn main() -> i32 { return \"a\" * \"b\" }", "E-TYPE");
    check_err_code("fn main() -> i32 { return 1 % 2.0 }", "E-TYPE");
}

#[test]
fn type_condition_must_be_bool_or_int() {
    check_err_code("fn main() -> i32 { if \"hi\" { return 1 } return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { while \"x\" { break } return 0 }", "E-TYPE");
    // Int and bool conditions still pass (backcompat with 1/0 logic values).
    check_clean("fn main() -> i32 { let a = 1 if a { return 1 } return 0 }");
    check_clean("fn main() -> i32 { if true { return 1 } return 0 }");
    check_clean("fn main() -> i32 { while true { break } return 0 }");
}

#[test]
fn type_return_must_match_signature() {
    check_err_code("fn main() -> i32 { return \"hi\" }", "E-TYPE");
    check_err_code("fn main() -> i32 { return 1.5 }", "E-TYPE");
    check_clean("fn main() -> i32 { return 42 }");
    // int coerces to float
    check_clean("fn main() -> f64 { return 1 }");
}

#[test]
fn type_call_args_checked() {
    check_err_code(
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(\"hi\", 1) }",
        "E-TYPE",
    );
    check_clean(
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(20, 22) }",
    );
}

#[test]
fn type_loops_checked() {
    check_err_code(
        "fn main() -> i32 { for i in 0..\"x\" { print(i) } return 0 }",
        "E-TYPE",
    );
    check_err_code("fn main() -> i32 { for x in 42 { print(x) } return 0 }", "E-TYPE");
    check_clean("fn main() -> i32 { for i in 0..10 { print(i) } return 0 }");
    check_clean("fn main() -> i32 { for x in [1,2] { print(x) } return 0 }");
}

#[test]
fn type_string_concat_and_numeric_coercion_ok() {
    check_clean("fn main() -> i32 { let s = \"kl\" + \"ang\" print(s) return 0 }");
    check_clean("fn main() -> f64 { return 1.5 + 1 }");
    // dynamic map/index stays wildcard, no false positives
    check_clean("fn main() -> i32 { let m = {\"a\": 20} print(m[\"a\"] + 22) return 0 }");
}

#[test]
fn type_bad_index_and_field_is_error() {
    check_err_code("fn main() -> i32 { let a = [1,2] print(a[\"x\"]) return 0 }", "E-TYPE");
    check_err_code(
        "struct P { x: i32 } fn main() -> i32 { let p = P { x: 1 } print(p.nope) return 0 }",
        "E-UNDEFINED",
    );
    check_err_code(
        "struct P { x: i32 } fn main() -> i32 { let p = P { x: \"hi\" } return 0 }",
        "E-TYPE",
    );
}

#[test]
fn type_mixed_eq_is_error_but_numeric_mix_ok() {
    check_err_code("fn main() -> i32 { if 1 == \"hi\" { return 1 } return 0 }", "E-TYPE");
    check_clean("fn main() -> i32 { if 1 == 1.0 { return 1 } return 0 }");
    check_clean("fn main() -> i32 { if \"a\" == \"b\" { return 1 } return 0 }");
}

#[test]
fn type_error_runs_end_to_end() {
    // A well-typed program still lowers + runs to 42.
    let src = "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(20, 22) }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn type_relet_shadows_and_loop_var_rebinds() {
    // Re-`let` shadows: runtime rebinds, so the checker follows.
    check_clean("fn main() -> i32 { let x = 1 let x = \"hi\" print(x) return 0 }");
    // Loop var rebinds to Int even when an outer `i` has another type.
    check_clean("fn main() -> i32 { let i = true for i in 0..3 { print(i + 1) } return 0 }");
    check_clean("fn main() -> i32 { let x = \"s\" for x in [1, 2] { print(x) } return 0 }");
    println!("shadow/rebind OK");
}

#[test]
fn type_duplicate_definitions_are_errors() {
    check_err_code(
        "fn main() -> i32 { return 1 } fn main() -> i32 { return 2 }",
        "E-DUPLICATE",
    );
    check_err_code(
        "fn f(a: i32, a: i32) -> i32 { return a } fn main() -> i32 { return f(1, 2) }",
        "E-DUPLICATE",
    );
    check_err_code(
        "struct P { x: i32, x: i32 } fn main() -> i32 { return 0 }",
        "E-DUPLICATE",
    );
}

#[test]
fn type_method_arity_is_exact() {
    check_err_code(
        "fn main() -> i32 { print(\"hi\".replace(\"i\")) return 0 }",
        "E-ARITY",
    );
    check_err_code(
        "fn main() -> i32 { print(\"a b\".split()) return 0 }",
        "E-ARITY",
    );
    check_err_code(
        "fn main() -> i32 { let a = [1] print(a.push()) return 0 }",
        "E-ARITY",
    );
    check_err_code(
        "fn main() -> i32 { print(\"hi\".frobnicate()) return 0 }",
        "E-TYPE",
    );
    check_clean("fn main() -> i32 { print(\"hi\".replace(\"i\", \"o\")) return 0 }");
    println!("method-arity OK");
}

#[test]
fn type_int_literals_checked_against_i32() {
    // Out-of-range literals would wrap silently at `run` time.
    check_err_code("fn main() -> i32 { return 3000000000 }", "E-TYPE");
    check_err_code("fn main() -> i32 { return -2147483649 }", "E-TYPE");
    check_clean("fn main() -> i32 { return 2147483647 }");
    check_clean("fn main() -> i32 { return -2147483648 }");
    check_clean("fn main() -> i32 { let x = 5 return -x }");
    println!("int-range OK");
}

#[test]
fn semantic_diagnostic_carries_real_file_path() {
    // F12: checker-stage diagnostics must report the real input path, not
    // the "input.warden" placeholder. Parser E-PARSE already did; every
    // semantic code must too. The path below is deliberately not the
    // placeholder so the test cannot pass by accident.
    let path = "/tmp/f12_semantic_file_path.klang";
    let src = "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(\"hi\", 1) }";
    let mut p = Parser::new_with_file(src, path);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check_with_file(prog, path).expect_err("must fail");
    assert!(
        err.iter().any(|d| d.code == "E-TYPE"),
        "want E-TYPE, got {:?}",
        err.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    for d in &err {
        assert_eq!(
            d.primary_span.file, path,
            "diagnostic file must be the real path, got {} ({})",
            d.primary_span.file, d.to_json()
        );
        assert_ne!(d.primary_span.file, "input.warden");
    }
    // Same for E-ARITY.
    let asrc = "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(1) }";
    let mut q = Parser::new_with_file(asrc, path);
    let prog2 = q.parse_program().expect("parses");
    let err2 = klang::hir::TypedHIR::check_with_file(prog2, path).expect_err("must fail");
    assert!(err2.iter().any(|d| d.code == "E-ARITY"));
    for d in &err2 {
        assert_eq!(d.primary_span.file, path, "{}", d.to_json());
    }
}

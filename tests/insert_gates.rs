//! FOUNDATION-1 Phase 3 — array positional `insert`.
//!
//! The PRD names "append/insert"; only append (`push`) existed. `insert`
//! ships in both call shapes the stdlib already uses for `push`/`pop`:
//! free builtin `insert(a, i, v)` and method `a.insert(i, v)`. Index
//! `== len` appends; negative or past-the-end is a loud `E-RUNTIME`.
//! Returns the new length (mirrors `push`).

use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str) -> (i32, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs")
}

fn run_err(src: &str) -> klang::diagnostics::Diagnostic {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail")
}

fn check_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&want), "want {want} in {codes:?}: {src}");
}

#[test]
fn insert_builtin_middle_front_back() {
    let (v, out) = run_src(
        "fn main() -> i32 { let a = [1, 3] print(insert(a, 1, 2)) print(a) print(insert(a, 0, 0)) print(a) print(insert(a, len(a), 4)) print(a) return len(a) }",
    );
    assert_eq!(
        out,
        vec![
            "3".to_string(),
            "[1, 2, 3]".to_string(),
            "4".to_string(),
            "[0, 1, 2, 3]".to_string(),
            "5".to_string(),
            "[0, 1, 2, 3, 4]".to_string(),
        ]
    );
    assert_eq!(v, 5);
}

#[test]
fn insert_method_same_semantics() {
    let (v, out) = run_src(
        "fn main() -> i32 { let a = [1, 3] print(a.insert(1, 2)) print(a) return len(a) }",
    );
    assert_eq!(out, vec!["3".to_string(), "[1, 2, 3]".to_string()]);
    assert_eq!(v, 3);
}

#[test]
fn insert_into_empty_at_zero() {
    let (v, out) = run_src(
        "fn main() -> i32 { let a = [] print(insert(a, 0, 9)) print(a) return len(a) }",
    );
    assert_eq!(out, vec!["1".to_string(), "[9]".to_string()]);
    assert_eq!(v, 1);
}

#[test]
fn insert_accepts_any_element_type() {
    let (v, out) = run_src(
        "fn main() -> i32 { let a = [1] insert(a, 1, \"s\") insert(a, 0, 2.5) print(a) return len(a) }",
    );
    assert_eq!(out, vec!["[2.5, 1, s]".to_string()]);
    assert_eq!(v, 3);
}

#[test]
fn insert_out_of_bounds_is_loud() {
    for src in [
        "fn main() -> i32 { let a = [1] insert(a, 2, 9) return 0 }",
        "fn main() -> i32 { let a = [1] insert(a, 0 - 1, 9) return 0 }",
        "fn main() -> i32 { let a = [1] a.insert(7, 9) return 0 }",
        "fn main() -> i32 { let a = [] a.insert(1, 9) return 0 }",
    ] {
        let d = run_err(src);
        assert_eq!(d.code, "E-RUNTIME", "{src}: {}", d.to_json());
        assert!(
            d.message.contains("insert() index out of bounds"),
            "{src}: {}",
            d.message
        );
    }
}

#[test]
fn insert_needs_array_variable() {
    // Temporaries cannot be insertion targets (same rule as `push`):
    // the mutation would be discarded.
    check_err_code(
        "fn main() -> i32 { insert([1], 0, 9) return 0 }",
        "E-TYPE",
    );
    check_err_code(
        "fn main() -> i32 { let a = [1] print([1].insert(0, 9)) return 0 }",
        "E-TYPE",
    );
}

#[test]
fn insert_checker_rejects_bad_types_and_arity() {
    check_err_code(
        "fn main() -> i32 { insert(\"s\", 0, 1) return 0 }",
        "E-TYPE",
    );
    check_err_code(
        "fn main() -> i32 { let a = [1] insert(a, \"s\", 1) return 0 }",
        "E-TYPE",
    );
    check_err_code("fn main() -> i32 { let a = [1] insert(a, 0) return 0 }", "E-ARITY");
    check_err_code(
        "fn main() -> i32 { let a = [1] print(a.insert(1)) return 0 }",
        "E-ARITY",
    );
    // Result is `Int`: usable as a length straight away.
    let mut p =
        Parser::new("fn take(x: i32) -> i32 { return x } fn main() -> i32 { let a = [1] return take(insert(a, 0, 0)) }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}

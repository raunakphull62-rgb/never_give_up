//! STDLIB-OSIO Phase 3 — Regex gates.
//!
//! Covers `regex_is_match(pattern, text)` / `regex_find(pattern, text)`
//! per the PRD: a simple match, a non-match (normal result, not an
//! error), indexed + named capture-group extraction (asserting the
//! captured substrings, not just match/no-match), and malformed
//! patterns (`E-REGEX-INVALID-PATTERN` with the real parse error).

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

#[test]
fn osio_regex_simple_match() {
    let src = "fn main() -> i32 { if regex_is_match(\"hello\", \"say hello world\") { return 42 } return 0 }";
    let (v, _) = run_src(src, "main").expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn osio_regex_non_match_is_false_not_error() {
    // A non-match is a normal `false` result, never a diagnostic —
    // parallel to Phase 2's non-zero-exit decision.
    let src = "fn main() -> i32 { if regex_is_match(\"xyz\", \"abc\") { return 99 } return 42 }";
    let (v, _) = run_src(src, "main").expect("non-match must still be Ok");
    assert_eq!(v, 42);
    let src =
        "fn main() -> i32 { let m = regex_find(\"xyz\", \"abc\") return m[\"matched\"] }";
    let (v, _) = run_src(src, "main").expect("runs");
    assert_eq!(v, 0);
}

#[test]
fn osio_regex_find_indexed_groups() {
    let src = "fn main() -> i32 { let m = regex_find(\"(\\\\w+)@(\\\\w+)\", \"user@example\") print(m[\"match\"]) print(m[\"groups\"][0]) print(m[\"groups\"][1]) return m[\"matched\"] }";
    let (v, out) = run_src(src, "main").expect("runs");
    assert_eq!(v, 1);
    assert_eq!(
        out,
        vec![
            "user@example".to_string(),
            "user".to_string(),
            "example".to_string()
        ]
    );
}

#[test]
fn osio_regex_find_named_groups() {
    let src = "fn main() -> i32 { let m = regex_find(\"(?P<user>\\\\w+)@(?P<host>\\\\w+)\", \"user@example\") print(m[\"named\"][\"user\"]) print(m[\"named\"][\"host\"]) return m[\"matched\"] }";
    let (v, out) = run_src(src, "main").expect("runs");
    assert_eq!(v, 1);
    assert_eq!(out, vec!["user".to_string(), "example".to_string()]);
}

#[test]
fn osio_regex_invalid_pattern_is_match_errors() {
    let src = "fn main() -> i32 { let b = regex_is_match(\"([\", \"text\") return 0 }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(
        err.code, "E-REGEX-INVALID-PATTERN",
        "got: {}",
        err.to_json()
    );
    assert!(
        err.to_json().contains("unclosed") || err.to_json().contains("group"),
        "real regex parse error, not generic: {}",
        err.to_json()
    );
}

#[test]
fn osio_regex_invalid_pattern_find_errors() {
    let src = "fn main() -> i32 { let m = regex_find(\"([\", \"text\") return m[\"matched\"] }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(
        err.code, "E-REGEX-INVALID-PATTERN",
        "got: {}",
        err.to_json()
    );
}

#[test]
fn osio_regex_error_carries_real_cause() {
    // Rust-level: the regex-crate parse error text is preserved verbatim
    // and differs per pattern (never one shared generic string).
    let a = klang::stdlib::regex::map_regex_error(
        "regex_is_match",
        "([",
        &regex::Regex::new("([").expect_err("must fail"),
    );
    assert_eq!(a.code, "E-REGEX-INVALID-PATTERN");
    let b = klang::stdlib::regex::map_regex_error(
        "regex_find",
        "(?P<bad>",
        &regex::Regex::new("(?P<bad>").expect_err("must fail"),
    );
    assert_eq!(b.code, "E-REGEX-INVALID-PATTERN");
    assert_ne!(a.cause, b.cause, "distinct real causes: {a:?} vs {b:?}");
    assert!(
        a.to_json().contains(&a.cause[..a.cause.len().min(12)]),
        "cause visible in JSON: {}",
        a.to_json()
    );
}

#[test]
fn osio_regex_arg_types_checked() {
    let mut p = Parser::new("fn main() -> i32 { return regex_is_match(42, \"x\") }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { return regex_find(\"a\") }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
}

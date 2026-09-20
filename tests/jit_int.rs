use std::collections::HashMap;

use klang::parser::Parser;

/// Run `src` through both backends; assert identical value + output.
fn diff(src: &str, entry: &str) -> (i32, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("interp runs");
    let (jv, jout) = klang::jit::run_jit(&mir, entry, &[]).expect("jit runs");
    assert_eq!(jv as i32, v, "jit value == interp value");
    assert_eq!(jout, out, "jit output == interp output");
    println!("jit==interp: {entry}() = {v}, printed={out:?}");
    (v, out)
}

#[test]
fn jit_arithmetic_precedence() {
    let (v, _) = diff("fn main() -> i32 { return 2 + 3 * 4 }", "main");
    assert_eq!(v, 14);
    let (v, _) = diff("fn main() -> i32 { return (2 + 3) * 4 }", "main");
    assert_eq!(v, 20);
    let (v, _) = diff("fn main() -> i32 { return 10 - 2 * 3 }", "main");
    assert_eq!(v, 4);
    let (v, _) = diff("fn main() -> i32 { return 20 / 4 + 1 }", "main");
    assert_eq!(v, 6);
    let (v, _) = diff("fn main() -> i32 { return -5 + 12 }", "main");
    assert_eq!(v, 7);
    let (v, _) = diff("fn main() -> i32 { return 17 % 5 }", "main");
    assert_eq!(v, 2);
}

#[test]
fn jit_calls_and_let() {
    let (v, _) = diff(
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { let x = 20 let y = x + 22 return add(x, y - 20) }",
        "main",
    );
    assert_eq!(v, 42);
    let (v, _) = diff(
        "fn double(n: i32) -> i32 { return n * 2 } fn main() -> i32 { return double(double(10)) + 2 }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn jit_if_else_and_logic() {
    let (v, _) = diff(
        "fn max(a: i32, b: i32) -> i32 { if a < b { return b } else { return a } } fn main() -> i32 { return max(20, 22) }",
        "main",
    );
    assert_eq!(v, 22);
    let (v, _) = diff(
        "fn main() -> i32 { if 1 <= 1 && 2 >= 2 && 3 != 4 && !(1 == 2) { return 42 } return 0 }",
        "main",
    );
    assert_eq!(v, 42);
    let (v, _) = diff(
        "fn main() -> i32 { let a = 0 || 0 if a { return 1 } return 7 }",
        "main",
    );
    assert_eq!(v, 7);
}

#[test]
fn jit_while_and_for_range() {
    let (v, _) = diff(
        "fn main() -> i32 { let t = 0 let i = 1 while i <= 10 { t = t + i i = i + 1 } return t }",
        "main",
    );
    assert_eq!(v, 55);
    let (v, _) = diff(
        "fn main() -> i32 { let t = 0 for i in 0..10 { t = t + i } return t }",
        "main",
    );
    assert_eq!(v, 45);
    let (v, _) = diff(
        "fn main() -> i32 { let t = 0 for i in 0..100 { if i == 10 { break } if i % 2 == 0 { continue } t = t + i } return t }",
        "main",
    );
    assert_eq!(v, 25);
}

#[test]
fn jit_print_captures_output() {
    let (v, out) = diff(
        "fn main() -> i32 { print(40 + 2) return 0 }",
        "main",
    );
    assert_eq!(v, 0);
    assert_eq!(out, vec!["42".to_string()]);
}

#[test]
fn jit_rejects_non_int_mir() {
    let mut p = Parser::new("fn main() -> i32 { let a = [1, 2] return len(a) }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("arrays unsupported");
    assert!(err.contains("unsupported"), "{err}");
    println!("jit rejects arrays OK: {err}");
}

#[test]
fn jit_unknown_entry_is_error() {
    let mut p = Parser::new("fn main() -> i32 { return 1 }");
    let prog = p.parse_program().expect("parses");
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "nope", &[]).expect_err("must fail");
    assert!(err.contains("unknown entry"), "{err}");
}

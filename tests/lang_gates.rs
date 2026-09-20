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

#[test]
fn lang_arithmetic_precedence() {
    // 2 + 3 * 4 = 14, not 20.
    let (v, _) = run_src("fn main() -> i32 { return 2 + 3 * 4 }", "main");
    assert_eq!(v, 14);
    let (v, _) = run_src("fn main() -> i32 { return (2 + 3) * 4 }", "main");
    assert_eq!(v, 20);
    let (v, _) = run_src("fn main() -> i32 { return 10 - 2 * 3 }", "main");
    assert_eq!(v, 4);
    let (v, _) = run_src("fn main() -> i32 { return 20 / 4 + 1 }", "main");
    assert_eq!(v, 6);
    let (v, _) = run_src("fn main() -> i32 { return -5 + 12 }", "main");
    assert_eq!(v, 7);
    println!("lang arithmetic OK");
}

#[test]
fn lang_let_and_vars() {
    let (v, _) = run_src(
        "fn main() -> i32 { let x = 20 let y = x + 22 return y }",
        "main",
    );
    assert_eq!(v, 42);
    println!("lang let OK: {v}");
}

#[test]
fn lang_function_args() {
    let (v, _) = run_src(
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(20, 22) }",
        "main",
    );
    assert_eq!(v, 42);
    // Nested calls.
    let (v, _) = run_src(
        "fn double(n: i32) -> i32 { return n * 2 } fn main() -> i32 { return double(double(10)) + 2 }",
        "main",
    );
    assert_eq!(v, 42);
    println!("lang args OK");
}

#[test]
fn lang_if_else() {
    let src = "fn max(a: i32, b: i32) -> i32 { if a < b { return b } else { return a } } fn main() -> i32 { return max(20, 22) }";
    let (v, _) = run_src(src, "main");
    assert_eq!(v, 22);
    let src2 = "fn max(a: i32, b: i32) -> i32 { if a < b { return b } else { return a } } fn main() -> i32 { return max(30, 12) }";
    let (v, _) = run_src(src2, "main");
    assert_eq!(v, 30);
    // if without else, equality.
    let (v, _) = run_src(
        "fn main() -> i32 { let x = 5 if x == 5 { return 42 } return 0 }",
        "main",
    );
    assert_eq!(v, 42);
    println!("lang if/else OK");
}

#[test]
fn lang_print_captures_output() {
    let (v, out) = run_src(
        "fn main() -> i32 { print(40 + 2) print(\"hi\") return 0 }",
        "main",
    );
    assert_eq!(v, 0);
    assert_eq!(out, vec!["42".to_string(), "hi".to_string()]);
    println!("lang print OK: {out:?}");
}

#[test]
fn lang_strings_and_bool() {
    let (v, out) = run_src(
        "fn main() -> i32 { print(\"hello\") if true { return 1 } return 0 }",
        "main",
    );
    assert_eq!(v, 1);
    assert_eq!(out, vec!["hello".to_string()]);
    let (v, _) = run_src(
        "fn main() -> i32 { if false { return 1 } return 7 }",
        "main",
    );
    assert_eq!(v, 7);
    println!("lang str/bool OK");
}

#[test]
fn lang_tasks_with_real_values() {
    // spawn user functions with args, await, combine.
    let src = "fn fetch(n: i32) -> i32 { return n * 2 } fn combine() -> i32 async { task_group { let a = spawn fetch(20) let b = spawn fetch(1) return await a + await b } }";
    let (v, _) = run_src(src, "combine");
    assert_eq!(v, 42);
    println!("lang tasks+args OK: {v}");
}

#[test]
fn lang_arity_mismatch_is_error() {
    check_err_code(
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { return add(1) }",
        "E-ARITY",
    );
}

#[test]
fn lang_undefined_var_is_error() {
    check_err_code("fn main() -> i32 { return nope + 1 }", "E-UNDEFINED");
}

#[test]
fn lang_effect_still_enforced() {
    // throws callee without throws caller.
    check_err_code(
        "fn boom() -> i32 throws { return 1 } fn main() -> i32 { return boom() }",
        "E-EFFECT-MISMATCH",
    );
}

#[test]
fn lang_task_leak_still_fires() {
    check_err_code(
        "fn fetch_a() -> i32 throws {} fn bad() -> i32 throws async { task_group { let a = spawn fetch_a() return 0 } }",
        "E-TASK-CANCEL",
    );
}

#[test]
fn lang_node_identity_holds_for_new_nodes() {
    use klang::ast::Stmt;
    let mut p = Parser::new(
        "fn add(a: i32, b: i32) -> i32 { let s = a + b if s == 42 { print(s) } return s }",
    );
    let prog = p.parse_program().expect("parses");
    let f = &prog.functions[0];
    assert!(f.body.id.starts_with(&f.id));
    for s in &f.body.stmts {
        assert!(s.id().starts_with(&f.body.id), "stmt in block");
        assert!(s.id().starts_with(&f.id), "stmt in fn");
        if let Stmt::If(i) = s {
            assert!(i.id.starts_with(&f.body.id));
            assert!(i.then_block.id.starts_with(&i.id));
            for inner in &i.then_block.stmts {
                assert!(inner.id().starts_with(&i.then_block.id));
                assert!(inner.id().starts_with(&i.id));
            }
        }
    }
    println!("lang identity OK");
}

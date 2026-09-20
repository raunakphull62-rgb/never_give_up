use std::collections::HashMap;

use klang::parser::Parser;

fn parse(src: &str) -> klang::ast::Program {
    let mut p = Parser::new(src);
    p.parse_program().expect("parses")
}

#[test]
fn fmt_is_idempotent() {
    for src in [
        include_str!("../examples/simple.klang"),
        include_str!("../examples/full.klang"),
        "fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { if 1 < 2 { return add(20, 22) } else { return 0 } }",
        "struct P { x: i32, y: i32 } fn main() -> i32 { let p = P { x: 1, y: 2 } while p.x < 10 { p.x = p.x + 1 } for i in 0..3 { print(i) } return p.x }",
    ] {
        let once = klang::fmt::fmt_program(&parse(src));
        let twice = klang::fmt::fmt_program(&parse(&once));
        assert_eq!(once, twice, "fmt must be idempotent for:\n{src}");
    }
}

#[test]
fn fmt_output_reparses_and_runs_same() {
    let src = include_str!("../examples/simple.klang");
    let formatted = klang::fmt::fmt_program(&parse(src));
    let orig = parse(src);
    let reformatted = parse(&formatted);
    assert_eq!(
        orig.functions.len(),
        reformatted.functions.len(),
        "same function count after fmt"
    );
    assert!(klang::hir::TypedHIR::check(reformatted.clone()).is_ok());
    let mir = klang::mir::lower(&reformatted);
    let (v, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
    assert_eq!(out, vec!["14".to_string(), "22".to_string()]);
}

#[test]
fn fmt_preserves_task_groups() {
    let src = "fn fetch(n: i32) -> i32 { return n * 2 } fn combine() -> i32 async { task_group { let a = spawn fetch(20) let b = spawn fetch(1) return await a + await b } }";
    let formatted = klang::fmt::fmt_program(&parse(src));
    assert!(formatted.contains("task_group"), "keeps task_group:\n{formatted}");
    let prog = parse(&formatted);
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "combine", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

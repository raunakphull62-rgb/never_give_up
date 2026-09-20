//! Task-group failure semantics: lossless groups + cooperative cancel.
//! Phase-0 failing-first tests: MULTI_FAIL currently returns only the
//! first failure (gap); the rest pass before and after.

use std::collections::HashMap;
use std::time::Duration;

use klang::parser::Parser;

const MULTI_FAIL: &str = r#"
fn fail_a() -> i32 { return 1 / 0 }
fn fail_b() -> i32 { assert(1 == 2) return 0 }
fn main() -> i32 async {
    task_group {
        let a = spawn fail_a()
        let b = spawn fail_b()
        let x = await a
        let y = await b
        return x + y
    }
}
"#;

const SINGLE_FAIL: &str = r#"
fn boom() -> i32 { return 1 / 0 }
fn ok() -> i32 { return 40 }
fn main() -> i32 async {
    task_group {
        let a = spawn boom()
        let b = spawn ok()
        let x = await a
        let y = await b
        return x + y
    }
}
"#;

const LOOPER: &str = r#"
fn loop_forever() -> i32 { while true { } return 0 }
fn boom() -> i32 { return 1 / 0 }
fn main() -> i32 async {
    task_group {
        let t = spawn loop_forever()
        let b = spawn boom()
        let x = await b
        let y = await t
        return x + y
    }
}
"#;

fn check(src: &str) -> klang::ast::Program {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    prog
}

/// Run with a wall-time guard: a regressed drain (joining an unstopped
/// looper forever) fails the test instead of hanging the suite.
fn run_guarded(src: &str) -> Result<(i32, Vec<String>), klang::Diagnostic> {
    let prog = check(src);
    let mir = klang::mir::lower(&prog);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
        let _ = tx.send(r);
    });
    match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(r) => r,
        Err(_) => panic!("task group did not finish in 15s (drain hung?)"),
    }
}

#[test]
fn task_group_reports_all_failures() {
    let prog = check(MULTI_FAIL);
    let mir = klang::mir::lower(&prog);
    let err = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new())
        .expect_err("both children fail");
    assert_eq!(err.code, "E-TASK-GROUP", "got: {}", err.to_json());
    assert_eq!(err.related.len(), 2, "got: {}", err.to_json());
    // Deterministic order: awaited failure first, drained siblings sorted.
    assert!(err.related[0].message.contains("division"), "got: {}", err.to_json());
    assert!(err.related[1].message.contains("assert"), "got: {}", err.to_json());
    println!("group OK: {}", err.to_json());
}

#[test]
fn spawn_only_as_direct_let_binding() {
    let bad = [
        // nested in an expression: handle unreachable
        "fn f() -> i32 { return 1 } fn main() -> i32 async { task_group { let y = spawn f() + 1 return await y } }",
        // bare statement: never awaitable
        "fn f() -> i32 { return 1 } fn main() -> i32 async { task_group { spawn f() return 0 } }",
        // returned directly: escapes the group
        "fn f() -> i32 { return 1 } fn main() -> i32 async { task_group { let a = spawn f() return spawn f() } }",
    ];
    for src in bad {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses");
        let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
        assert!(
            err.iter().any(|d| d.code == "E-SPAWN-POSITION"),
            "want E-SPAWN-POSITION, got {:?}",
            err.iter().map(|d| &d.code).collect::<Vec<_>>()
        );
    }
    // Direct binding stays clean.
    let mut p = Parser::new(
        "fn f() -> i32 { return 1 } fn main() -> i32 async { task_group { let a = spawn f() return await a } }",
    );
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
    println!("spawn-position OK");
}

#[test]
fn single_task_failure_stays_unwrapped() {
    let prog = check(SINGLE_FAIL);
    let mir = klang::mir::lower(&prog);
    let err = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new())
        .expect_err("boom fails");
    assert_eq!(err.code, "E-RUNTIME", "got: {}", err.to_json());
    assert!(err.related.is_empty());
    println!("single-failure OK: {}", err.to_json());
}

#[test]
fn looping_sibling_is_cancelled_not_joined_forever() {
    let err = run_guarded(LOOPER).expect_err("boom fails");
    // Only boom's failure surfaces; the cancelled looper is excluded.
    assert_eq!(err.code, "E-RUNTIME", "got: {}", err.to_json());
    assert!(err.related.is_empty());
    println!("cancel OK: {}", err.to_json());
}

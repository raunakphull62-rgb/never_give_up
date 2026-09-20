use klang::ast::Stmt;
use klang::hir::TypedHIR;
use klang::parser::Parser;

const MILESTONE: &str = r#"
fn fetch_a() -> i32 throws {
}

fn fetch_b() -> i32 throws {
}

fn combine() -> i32 throws async {
    task_group {
        let a = spawn fetch_a()
        let b = spawn fetch_b()
        return await a + await b
    }
}
"#;

#[test]
fn sibling_functions_have_distinct_paths() {
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    let a = &prog.functions[0];
    let b = &prog.functions[1];
    println!("fn a path: {}", a.id);
    println!("fn b path: {}", b.id);
    assert_ne!(a.id, b.id);
    assert_ne!(a.id.path, b.id.path);
}

#[test]
fn stmts_in_same_block_are_distinct() {
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    let combine = prog.functions.iter().find(|f| f.name == "combine").unwrap();
    let g = combine
        .body
        .stmts
        .iter()
        .find_map(|s| match s {
            Stmt::TaskGroup(g) => Some(g),
            _ => None,
        })
        .unwrap();
    let p0 = g.body.stmts[0].id().display();
    let p1 = g.body.stmts[1].id().display();
    println!("stmt0: {p0}");
    println!("stmt1: {p1}");
    assert_ne!(p0, p1);
}

#[test]
fn parent_prefix_containment_everywhere() {
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    for f in &prog.functions {
        assert!(f.body.id.starts_with(&f.id), "block extends fn {}", f.name);
        for s in &f.body.stmts {
            assert!(s.id().starts_with(&f.body.id), "stmt extends block");
            assert!(s.id().starts_with(&f.id), "stmt extends fn");
            if let Stmt::TaskGroup(g) = s {
                assert!(g.id.starts_with(&f.id));
                assert!(g.body.id.starts_with(&g.id));
                for inner in &g.body.stmts {
                    assert!(inner.id().starts_with(&g.body.id));
                    assert!(inner.id().starts_with(&g.id));
                    assert!(inner.id().starts_with(&f.id));
                }
            }
        }
    }
}

#[test]
fn milestone_checks_clean() {
    let mut p = Parser::new(MILESTONE);
    let prog = p.parse_program().expect("parses");
    assert!(TypedHIR::check(prog).is_ok(), "milestone must check clean");
}

#[test]
fn unawaited_spawn_emits_task_cancel() {
    let src = r#"
fn fetch_a() -> i32 throws {
}
fn bad() -> i32 throws async {
    task_group {
        let a = spawn fetch_a()
        return await b
    }
}
"#;
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = TypedHIR::check(prog).expect_err("must fail");
    println!("diag: {}", err[0].to_json());
    assert!(err.iter().any(|d| d.code == "E-TASK-CANCEL"));
}

#[test]
fn spawn_outside_group_is_error() {
    let src = r#"
fn fetch_a() -> i32 throws {
}
fn bad() -> i32 throws {
    let a = spawn fetch_a()
    return await a
}
"#;
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-SPAWN-OUTSIDE-GROUP"));
}

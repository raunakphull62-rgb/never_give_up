use std::collections::HashMap;
use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().map_err(|e| e)?;
    klang::hir::TypedHIR::check(prog.clone()).map_err(|ds| ds.into_iter().next().unwrap())?;
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new())
}

#[test]
fn heavy_deep_nesting_2000_parens() {
    let mut src = String::from("fn main() -> i32 { return ");
    for _ in 0..2000 { src.push('('); }
    src.push_str("42");
    for _ in 0..2000 { src.push(')'); }
    src.push_str(" }");

    let (v, _) = klang::with_deep_stack(move || {
        let mut p = Parser::new(&src);
        let prog = p.parse_program().expect("parses 2k parens");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok(), "must check clean on deep stack");
        let mir = klang::mir::lower(&prog);
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs")
    });
    assert_eq!(v, 42);
}

#[test]
fn heavy_concurrency_limit_static_boundary() {
    let mut src = String::from("fn work(n: i32) -> i32 { return n }\nfn main() -> i32 async {\n  task_group {\n");
    for i in 0..257 {
        src.push_str(&format!("    let t{} = spawn work({})\n", i, i));
    }
    for i in 0..257 {
        src.push_str(&format!("    let _v{} = await t{}\n", i, i));
    }
    src.push_str("    return 0\n  }\n}\n");

    let mut p = Parser::new(&src);
    let prog = p.parse_program().expect("parses 257 static spawns");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "257 static spawns with awaits must check clean");

    let mir = klang::mir::lower(&prog);
    let err = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new())
        .expect_err("must fail at runtime with too many concurrent tasks");
    assert_eq!(err.code, "E-RUNTIME", "got: {}", err.to_json());
    assert!(err.message.contains("too many concurrent tasks"),
        "expected concurrency limit message, got: {}", err.message);
}

#[test]
fn heavy_generic_module_effect_matrix() {
    let src = r#"
    mod a {
        pub enum Opt<T> { Some(v: T), None }
        pub struct Box<T> { inner: Opt<T> }
        pub fn wrap<T>(x: T) -> Box<T> {
            return Box { inner: Opt::Some(x) }
        }
    }
    mod b {
        pub fn extract(b: a::Box<i32>) -> i32 throws {
            return match b.inner {
                a::Opt::Some(v) => v,
                a::Opt::None => 0
            }
        }
    }
    fn main() -> i32 throws {
        let b = a::wrap(42)
        return b::extract(b)
    }
    "#;
    let (v, _) = run_src(src, "main").expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn heavy_massive_match_5000_arms() {
    let mut src = String::from("enum E { ");
    for i in 0..5000 {
        if i > 0 { src.push_str(", "); }
        src.push_str(&format!("V{}", i));
    }
    src.push_str(" } fn f(e: E) -> i32 { return match e { ");
    for i in 0..5000 {
        if i > 0 { src.push_str(", "); }
        src.push_str(&format!("E::V{} => {}", i, i));
    }
    src.push_str(" } } fn main() -> i32 { return f(E::V4999()) }");

    let (v, _) = run_src(&src, "main").expect("runs");
    assert_eq!(v, 4999);
}

#[test]
fn heavy_scope_planner_50_functions() {
    let mut src = String::new();
    for i in 0..50 {
        if i == 25 {
            src.push_str(&format!("fn f{}() -> i32 {{ return undefined_var }}\n", i));
        } else {
            src.push_str(&format!("fn f{}() -> i32 {{ return {} }}\n", i, i));
        }
    }
    src.push_str("fn main() -> i32 { return f0() }\n");

    let diags = klang::repair::splice::check_candidate(&src).diagnostics;
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "E-UNDEFINED");

    let scope = klang::repair::plan_scope(&diags, &src, false);
    assert_eq!(scope, klang::repair::scope::RepairScope::Functions(vec!["f25".into()]));
}

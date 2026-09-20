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

#[test]
fn v04_else_if_chains() {
    let (v, _) = run_src(
        "fn grade(s: i32) -> i32 { if s >= 90 { return 4 } else if s >= 80 { return 3 } else if s >= 70 { return 2 } else { return 0 } } fn main() -> i32 { return grade(95) * 10 + grade(85) }",
        "main",
    );
    // 4*10+3 = 43... use values that hit 42: grade(85)=3, grade(75)=2 -> 32? compute directly:
    assert_eq!(v, 43);
    let (v, _) = run_src(
        "fn grade(s: i32) -> i32 { if s >= 90 { return 4 } else if s >= 80 { return 3 } else if s >= 70 { return 2 } else { return 0 } } fn main() -> i32 { return grade(72) }",
        "main",
    );
    assert_eq!(v, 2);
    let (v, _) = run_src(
        "fn grade(s: i32) -> i32 { if s >= 90 { return 4 } else if s >= 80 { return 3 } else if s >= 70 { return 2 } else { return 0 } } fn main() -> i32 { return grade(10) }",
        "main",
    );
    assert_eq!(v, 0);
}

#[test]
fn v04_string_methods() {
    let (v, out) = run_src(
        "fn main() -> i32 { let s = \"  Hello World  \" print(s.trim()) print(s.trim().upper()) print(s.trim().lower()) print(s.trim().split(\" \")) print(\"hi\".replace(\"i\", \"o\")) return 0 }",
        "main",
    );
    assert_eq!(v, 0);
    assert_eq!(
        out,
        vec![
            "Hello World".to_string(),
            "HELLO WORLD".to_string(),
            "hello world".to_string(),
            "[Hello, World]".to_string(),
            "ho".to_string(),
        ]
    );
    let (v, _) = run_src(
        "fn main() -> i32 { let s = \"klang\" if s.starts_with(\"kl\") && s.ends_with(\"ang\") && s.contains(\"la\") && s.len() == 5 { return 42 } return 0 }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn v04_array_methods() {
    let (v, out) = run_src(
        "fn main() -> i32 { let a = [3, 1] a.push(2) print(a) print(a.join(\", \")) print(a.pop()) print(a.len()) if a.contains(1) { return 42 } return 0 }",
        "main",
    );
    assert_eq!(v, 42);
    assert_eq!(
        out,
        vec![
            "[3, 1, 2]".to_string(),
            "3, 1, 2".to_string(),
            "2".to_string(),
            "2".to_string(),
        ]
    );
}

#[test]
fn v04_maps() {
    let (v, out) = run_src(
        "fn main() -> i32 { let m = {\"a\": 20, \"b\": 22} print(m[\"a\"] + m[\"b\"]) m[\"c\"] = 100 print(m.len()) print(m.keys()) m[\"a\"] = 0 if m.contains(\"b\") { return 42 } return 0 }",
        "main",
    );
    assert_eq!(v, 42);
    assert_eq!(
        out,
        vec!["42".to_string(), "3".to_string(), "[a, b, c]".to_string()]
    );
    // ident keys + builtin keys() + render
    let (v, out) = run_src(
        "fn main() -> i32 { let m = {a: 1} print(m) print(keys(m)) return len(m) }",
        "main",
    );
    assert_eq!(v, 1);
    assert_eq!(out, vec!["{\"a\": 1}".to_string(), "[a]".to_string()]);
}

#[test]
fn v04_int_float_conversions() {
    let (v, out) = run_src(
        "fn main() -> i32 { print(int(3.9)) print(int(\"41\")) print(float(2)) print(float(\"0.5\") + 41.5) return int(42.0) }",
        "main",
    );
    assert_eq!(v, 42);
    assert_eq!(
        out,
        vec![
            "3".to_string(),
            "41".to_string(),
            "2.0".to_string(),
            "42.0".to_string()
        ]
    );
}

#[test]
fn v04_concurrent_tasks_still_42() {
    // Same milestone, now on real threads.
    let src = "fn fetch(n: i32) -> i32 { return n * 2 } fn combine() -> i32 async { task_group { let a = spawn fetch(20) let b = spawn fetch(1) return await a + await b } }";
    let (v, _) = run_src(src, "combine");
    assert_eq!(v, 42);
    // Parallelism smoke: many spawns joined in order.
    let (v, _) = run_src(
        "fn one(n: i32) -> i32 { return n } fn main() -> i32 async { task_group { let a = spawn one(10) let b = spawn one(20) let c = spawn one(12) return await a + await b + await c } }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn v04_tasks_inside_loops() {
    let (v, _) = run_src(
        "fn dbl(n: i32) -> i32 { return n * 2 } fn main() -> i32 async { let total = 0 for i in 0..5 { task_group { let t = spawn dbl(i) total = total + await t } } return total }",
        "main",
    );
    // 2*(0+1+2+3+4) = 20
    assert_eq!(v, 20);
}

#[test]
fn v04_identity_holds_for_new_nodes() {
    use klang::ast::Stmt;
    let mut p = Parser::new(
        "fn f(x: i32) -> i32 { let m = {\"a\": x} if x >= 1 { return m.upper().len() } else if x <= 0 { while x < 0 { x = x + 1 } } return 0 }",
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
            if let Some(e) = &i.else_block {
                assert!(e.id.starts_with(&i.id));
                for inner in &e.stmts {
                    assert!(inner.id().starts_with(&e.id));
                    assert!(inner.id().starts_with(&i.id));
                }
            }
        }
    }
    println!("v0.4 identity OK");
}

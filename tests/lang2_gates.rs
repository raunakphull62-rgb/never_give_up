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

fn run_err(src: &str) -> String {
    let mut p = Parser::new(src);
    match p.parse_program() {
        Err(d) => d.to_json(),
        Ok(prog) => match klang::hir::TypedHIR::check(prog.clone()) {
            Err(ds) => ds[0].to_json(),
            Ok(_) => {
                let mir = klang::mir::lower(&prog);
                match klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()) {
                    Err(d) => d.to_json(),
                    Ok(_) => "NO-ERROR".to_string(),
                }
            }
        },
    }
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
}

#[test]
fn real_while_loop() {
    let (v, _) = run_src(
        "fn main() -> i32 { let t = 0 let i = 1 while i <= 10 { t = t + i i = i + 1 } return t }",
        "main",
    );
    assert_eq!(v, 55);
}

#[test]
fn real_for_range() {
    let (v, _) = run_src(
        "fn main() -> i32 { let t = 0 for i in 0..10 { t = t + i } return t }",
        "main",
    );
    assert_eq!(v, 45);
}

#[test]
fn real_for_in_array() {
    let (v, _) = run_src(
        "fn main() -> i32 { let t = 0 for x in [1, 2, 3, 4] { t = t + x } return t }",
        "main",
    );
    assert_eq!(v, 10);
}

#[test]
fn real_break_continue() {
    let (v, _) = run_src(
        "fn main() -> i32 { let t = 0 for i in 0..100 { if i == 10 { break } if i % 2 == 0 { continue } t = t + i } return t }",
        "main",
    );
    // 1+3+5+7+9 = 25
    assert_eq!(v, 25);
    let (v, _) = run_src(
        "fn main() -> i32 { let i = 0 while true { i = i + 1 if i >= 5 { break } } return i }",
        "main",
    );
    assert_eq!(v, 5);
}

#[test]
fn real_nested_loops() {
    let (v, _) = run_src(
        "fn main() -> i32 { let t = 0 for i in 0..3 { for j in 0..3 { t = t + 1 } } return t }",
        "main",
    );
    assert_eq!(v, 9);
}

#[test]
fn real_arrays_index_assign() {
    let (v, out) = run_src(
        "fn main() -> i32 { let a = [1, 2, 3] a[0] = 40 print(a[0] + a[2]) return len(a) }",
        "main",
    );
    assert_eq!(v, 3);
    assert_eq!(out, vec!["43".to_string()]);
}

#[test]
fn real_push_len_range() {
    let (v, _) = run_src(
        "fn main() -> i32 { let a = [1] push(a, 2) push(a, 3) let t = 0 for x in range(0, len(a)) { t = t + a[x] } return t }",
        "main",
    );
    assert_eq!(v, 6);
}

#[test]
fn real_structs() {
    let (v, out) = run_src(
        "struct P { x: i32, y: i32 } fn main() -> i32 { let p = P { x: 20, y: 22 } print(p.x + p.y) p.x = 1 return p.x + p.y }",
        "main",
    );
    assert_eq!(v, 23);
    assert_eq!(out, vec!["42".to_string()]);
}

#[test]
fn real_floats() {
    let (v, out) = run_src(
        "fn main() -> i32 { let f = 0.5 * 4.0 + 40.0 print(f) if f == 42.0 { return 1 } return 0 }",
        "main",
    );
    assert_eq!(v, 1);
    assert_eq!(out, vec!["42.0".to_string()]);
}

#[test]
fn real_strings() {
    let (v, out) = run_src(
        "fn main() -> i32 { let s = \"kl\" + \"ang\" print(s) print(len(s)) print(s[0]) return 0 }",
        "main",
    );
    assert_eq!(v, 0);
    assert_eq!(
        out,
        vec!["klang".to_string(), "5".to_string(), "k".to_string()]
    );
}

#[test]
fn real_logic_and_comparisons() {
    let (v, _) = run_src(
        "fn main() -> i32 { if 1 <= 1 && 2 >= 2 && 3 != 4 && !(1 == 2) { return 42 } return 0 }",
        "main",
    );
    assert_eq!(v, 42);
    let (v, _) = run_src(
        "fn main() -> i32 { let a = 0 || 0 if a { return 1 } return 7 }",
        "main",
    );
    assert_eq!(v, 7);
}

#[test]
fn real_assign_and_augmented_patterns() {
    let (v, _) = run_src(
        "fn main() -> i32 { let x = 1 x = x * 10 + 2 return x }",
        "main",
    );
    assert_eq!(v, 12);
}

#[test]
fn real_builtin_str_assert() {
    let (v, out) = run_src(
        "fn main() -> i32 { let s = str(42) print(s) assert(s == \"42\") return 1 }",
        "main",
    );
    assert_eq!(v, 1);
    assert_eq!(out, vec!["42".to_string()]);
    let err = run_err("fn main() -> i32 { assert(0 == 1) return 0 }");
    assert!(
        err.contains("E-RUNTIME"),
        "assert must fail at runtime: {err}"
    );
}

#[test]
fn real_div_zero_is_runtime_error() {
    let err = run_err("fn main() -> i32 { return 1 / 0 }");
    assert!(err.contains("E-RUNTIME"), "{err}");
}

#[test]
fn real_break_outside_loop_is_error() {
    check_err_code("fn main() -> i32 { break return 0 }", "E-LOOP");
}

#[test]
fn real_assign_undefined_is_error() {
    check_err_code("fn main() -> i32 { x = 1 return x }", "E-UNDEFINED");
}

#[test]
fn real_import_merges_files() {
    use std::io::Write;
    let dir = std::env::temp_dir().join("klang-import-test");
    std::fs::create_dir_all(&dir).unwrap();
    let lib = dir.join("mylib.klang");
    let app = dir.join("myapp.klang");
    std::fs::write(&lib, "fn triple(n: i32) -> i32 { return n * 3 }").unwrap();
    let mut f = std::fs::File::create(&app).unwrap();
    writeln!(f, "import \"mylib.klang\"").unwrap();
    writeln!(f, "fn main() -> i32 {{ return triple(14) }}").unwrap();
    drop(f);
    // Load like the driver does: parse both, prefix, merge, check, run.
    let load = |path: &std::path::Path| {
        let src = std::fs::read_to_string(path).unwrap();
        let mut p = Parser::new(&src);
        p.parse_program().expect("parses")
    };
    let lp = load(&lib).with_file_prefix(0);
    let mut ap = load(&app);
    ap.imports.clear();
    let ap = ap.with_file_prefix(1);
    let merged = klang::ast::Program {
        mods: [lp.mods, ap.mods].concat(),
        enums: [lp.enums, ap.enums].concat(),
        structs: [lp.structs, ap.structs].concat(),
        imports: vec![],
        functions: [lp.functions, ap.functions].concat(),
    };
    assert!(klang::hir::TypedHIR::check(merged.clone()).is_ok());
    let mir = klang::mir::lower(&merged);
    let (v, _) = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

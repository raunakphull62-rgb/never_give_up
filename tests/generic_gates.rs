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
}

#[test]
fn generic_fn_two_instantiations_stay_distinct() {
    // Same generic function with i32 and str: each site infers its own T,
    // values come back with the right type and content.
    let (v, out) = run_src(
        "fn identity<T>(x: T) -> T { return x } fn main() -> i32 { let x = identity(40) print(identity(\"yo\")) return x + 2 }",
        "main",
    );
    assert_eq!(v, 42);
    assert_eq!(out, vec!["yo".to_string()]);
}

#[test]
fn generic_struct_two_instantiations_do_not_share_state() {
    // Box<i32> and Box<String> coexist; neither corrupts the other.
    let (v, out) = run_src(
        "struct Box<T> { value: T } fn main() -> i32 { let a = Box { value: 41 } let b = Box { value: \"hi\" } print(b.value) return a.value + 1 }",
        "main",
    );
    assert_eq!(v, 42);
    assert_eq!(out, vec!["hi".to_string()]);
}

#[test]
fn generic_kv_container_holds_swapped_instantiations() {
    // Entry<K, V> used as Entry<str, i32> and Entry<i32, str> in one program.
    let (v, out) = run_src(
        "struct Entry<K, V> { key: K, val: V } fn main() -> i32 { let e1 = Entry { key: \"a\", val: 1 } let e2 = Entry { key: 2, val: \"b\" } print(e2.val) return e1.val + 2 }",
        "main",
    );
    assert_eq!(v, 3);
    assert_eq!(out, vec!["b".to_string()]);
}

#[test]
fn generic_enum_two_instantiations_match() {
    // Opt<T> as Opt<i32> and Opt<str>: match dispatches on each correctly.
    let (v, _) = run_src(
        "enum Opt<T> { Some(x: T), None } fn is_some(o: Opt) -> i32 { return match o { Opt::Some(n) => 1, Opt::None => 0 } } fn main() -> i32 { return is_some(Opt::Some(41)) + is_some(Opt::Some(\"hi\")) + is_some(Opt::None()) }",
        "main",
    );
    assert_eq!(v, 2);
}

#[test]
fn generic_call_mismatch_is_diagnostic() {
    // Conflicting instantiation is a compile-time E-TYPE, not a panic and
    // not a silently-accepted program.
    check_err_code(
        "fn same<T>(a: T, b: T) -> T { return a } fn main() -> i32 { return same(1, \"hi\") }",
        "E-TYPE",
    );
    check_err_code(
        "struct Pair<T> { first: T, second: T } fn main() -> i32 { let p = Pair { first: 1, second: \"hi\" } return p.first }",
        "E-TYPE",
    );
    // Inference flows into the caller: returning inferred str as i32 fails.
    check_err_code(
        "fn identity<T>(x: T) -> T { return x } fn main() -> i32 { return identity(\"hi\") }",
        "E-TYPE",
    );
}

#[test]
fn generic_decls_survive_fmt_roundtrip() {
    let src = "struct Box<T> { value: T } fn identity<T>(x: T) -> T { return x } fn main() -> i32 { return identity(42) }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let out = klang::fmt::fmt_program(&prog);
    assert!(out.contains("struct Box<T>"), "{out}");
    assert!(out.contains("fn identity<T>"), "{out}");
    let mut q = Parser::new(&out);
    let prog2 = q.parse_program().expect("fmt output re-parses");
    assert_eq!(prog2.structs[0].type_params, vec!["T".to_string()]);
    assert_eq!(prog2.functions[0].type_params, vec!["T".to_string()]);
    assert!(klang::hir::TypedHIR::check(prog2).is_ok());
}

#[test]
fn generic_mismatch_names_param_and_inference_site() {
    // F13: `same(1, "two")` must not read as a hardcoded "want i32".
    // T was inferred from arg 0; the message must say so for repair.
    let src =
        "fn same<T>(a: T, b: T) -> T { return a } fn main() -> i32 { return same(1, \"two\") }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    let diag = err
        .iter()
        .find(|d| d.code == "E-TYPE")
        .expect("want E-TYPE");
    assert!(
        diag.message.contains('T'),
        "message must name the type parameter, got: {}",
        diag.message
    );
    assert!(
        diag.message.to_lowercase().contains("infer"),
        "message must reference inference, got: {}",
        diag.message
    );
    assert!(
        diag.message.contains("arg 0"),
        "message must name where T was bound, got: {}",
        diag.message
    );
    println!("generic-mismatch OK: {}", diag.message);
}

#[test]
fn explicit_generic_type_annotation_parses_and_checks() {
    // F15 case 1: `Opt<i32>` as a parameter type annotation. Previously
    // `expected ')'` at the `<`; now parses and resolves to the nominal
    // enum for checking and execution.
    let (v, _) = run_src(
        "enum Opt<T> { Some(v: T), None } fn unwrap_or(o: Opt<i32>, default: i32) -> i32 { return match o { Opt::Some(v) => v, Opt::None => default } } fn main() -> i32 { return unwrap_or(Opt::Some(40), 0) + unwrap_or(Opt::None(), 2) }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn explicit_generic_call_site_parses_without_comparison_diagnostics() {
    // F15 case 2: `count<T>(...)` / `count<i32>(...)` must parse as generic
    // calls, not as `count < T` comparisons (which produced 8 cascading
    // `undefined T` / `comparison operands` diagnostics with no hint of
    // the real ambiguity).
    let src = "fn count<T>(n: i32) -> i32 { if n <= 0 { return 0 } return 1 + count<T>(n - 1) } fn main() -> i32 { return count<i32>(5) }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("explicit generic calls must parse");
    match klang::hir::TypedHIR::check(prog.clone()) {
        Ok(_) => {}
        Err(ds) => {
            for d in &ds {
                assert!(
                    !d.message.contains("comparison operands"),
                    "must not misparse as comparison: {}",
                    d.to_json()
                );
                assert!(
                    !(d.code == "E-UNDEFINED" && d.message.contains("`T`")),
                    "T must not leak as undefined variable: {}",
                    d.to_json()
                );
            }
            panic!("explicit generic calls must check clean, got: {:?}", ds.iter().map(|d| d.to_json()).collect::<Vec<_>>());
        }
    }
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 5);
}

#[test]
fn plain_comparison_still_parses_as_comparison() {
    // F15 guard: fixing the ambiguity must not break genuine `a < b`.
    // `a < b` with two variables must remain a comparison (checks clean,
    // runs correctly), never a generic instantiation.
    let (v, _) = run_src(
        "fn main() -> i32 { let a = 1 let b = 2 if a < b { return 1 } return 0 }",
        "main",
    );
    assert_eq!(v, 1);
    // `f < 123` without `> (` must also stay a comparison, not a generic.
    let mut p = Parser::new("fn main() -> i32 { let f = 1 if f < 123 { return 1 } return 0 }");
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}

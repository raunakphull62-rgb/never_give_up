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

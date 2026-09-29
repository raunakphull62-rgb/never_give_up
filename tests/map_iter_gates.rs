//! FOUNDATION-1 Phase 3 — `for k in <map>` iteration.
//!
//! The checker used to reject maps (`E-TYPE`: "want array/str"), against
//! SPEC §2 ("`it` must be array/map/str"). The fix changes the checker
//! (and `reference.md` §3) to match the SPEC — not the reverse — because
//! the SPEC is the language contract and the old HIR comment admitted
//! the rejection was a lowering gap, now closed: an integer index on a
//! map yields its i-th key in insertion order, which is exactly what the
//! integer-indexed `for-in` lowering needs.

use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str) -> (i32, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs")
}

fn run_err(src: &str) -> klang::diagnostics::Diagnostic {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect_err("must fail")
}

fn check_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&want), "want {want} in {codes:?}: {src}");
}

#[test]
fn map_iteration_yields_keys_in_order() {
    let (v, out) = run_src(
        "fn main() -> i32 { let seen = \"\" for k in {a: 1, b: 2, c: 3} { seen = seen + k } print(seen) return len(seen) }",
    );
    assert_eq!(out, vec!["abc".to_string()]);
    assert_eq!(v, 3);
}

#[test]
fn map_iteration_values_reachable_via_index() {
    // The canonical pattern: keys out, values via `m[k]`.
    let (v, _) = run_src(
        "fn main() -> i32 { let m = {a: 10, b: 20} let total = 0 for k in m { total = total + m[k] } return total }",
    );
    assert_eq!(v, 30);
}

#[test]
fn map_iteration_matches_keys_builtin() {
    // `for k in m` visits exactly `keys(m)` in the same order.
    let (v, out) = run_src(
        "fn main() -> i32 { let m = {x: 1, y: 2} let a = \"\" for k in m { a = a + k } let b = \"\" for k in keys(m) { b = b + k } print(a == b) return len(keys(m)) }",
    );
    assert_eq!(out, vec!["1".to_string()]);
    assert_eq!(v, 2);
}

#[test]
fn map_iteration_empty_map_runs_zero_times() {
    let (v, out) = run_src(
        "fn main() -> i32 { let n = 0 for k in {} { n = n + 1 } print(n) return n }",
    );
    assert_eq!(out, vec!["0".to_string()]);
    assert_eq!(v, 0);
}

#[test]
fn int_index_on_map_is_positional_key() {
    // The rule `for-in` relies on: `m[0]` is the first key.
    let (v, out) = run_src(
        "fn main() -> i32 { let m = {a: 1, b: 2} print(m[0]) print(m[1]) return len(m) }",
    );
    assert_eq!(out, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(v, 2);
}

#[test]
fn str_index_on_map_is_still_key_lookup() {
    // Key lookup is untouched: `m[\"a\"]` is the value, and an integer
    // index never consults stringified keys (`m[0]` on `{\"0\": ...}`
    // is positional, `m[\"0\"]` is the lookup).
    let (v, out) = run_src(
        "fn main() -> i32 { let m = {a: 7, \"0\": 8} print(m[\"a\"]) print(m[\"0\"]) print(m[0]) return len(m) }",
    );
    assert_eq!(
        out,
        vec!["7".to_string(), "8".to_string(), "a".to_string()]
    );
    assert_eq!(v, 2);
}

#[test]
fn map_positional_index_out_of_bounds_is_loud() {
    let d = run_err("fn main() -> i32 { let m = {a: 1} print(m[5]) return 0 }");
    assert_eq!(d.code, "E-RUNTIME", "{}", d.to_json());
    let d = run_err("fn main() -> i32 { let m = {a: 1} print(m[0 - 1]) return 0 }");
    assert_eq!(d.code, "E-RUNTIME", "{}", d.to_json());
}

#[test]
fn for_in_still_rejects_non_iterables() {
    // Only the map rule changed: ints (and other non-iterables) stay
    // `E-TYPE` with the updated "array/map/str" expectation.
    check_err_code(
        "fn main() -> i32 { for x in 42 { print(x) } return 0 }",
        "E-TYPE",
    );
    let mut p = Parser::new("fn main() -> i32 { for x in 42 { print(x) } return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(
        err.iter()
            .any(|d| d.message.contains("array/map/str")),
        "names the full contract: {err:?}"
    );
}

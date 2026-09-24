//! Phase 2 robustness gates: pathological input must never crash the
//! compiler -- it must either check clean or report a diagnostic.

use std::collections::HashMap;

use klang::parser::Parser;

/// `fn main() -> i32 { return 1+1+...+1 }` with `n` operands. A flat
/// left-associative chain is maximally deep in the AST while staying
/// trivially *parseable*, which is exactly the shape that used to abort the
/// process (stack overflow) instead of returning a result.
fn chain_src(n: usize) -> String {
    let mut s = String::from("fn main() -> i32 { return 1");
    for _ in 0..n {
        s.push_str("+1");
    }
    s.push_str(" }");
    s
}

#[test]
fn deep_flat_expression_does_not_crash_the_compiler() {
    // Regression for the Phase 2 finding: 800+ operands overflowed the
    // default stack and aborted (exit 134). Real work runs on
    // `with_deep_stack`, so this must return an answer.
    let src = chain_src(2000);
    let (ok, diags) = klang::with_deep_stack(move || {
        let mut p = Parser::new(&src);
        let prog = p.parse_program().expect("flat chain must parse");
        match klang::hir::TypedHIR::check(prog.clone()) {
            Ok(_) => {
                let mir = klang::mir::lower(&prog);
                let list = klang::codegen::emit_listing(&mir);
                assert!(list.contains("func main"), "lowering must survive");
                (true, Vec::new())
            }
            Err(ds) => (false, ds),
        }
    });
    assert!(ok, "2000-term chain should check clean, got {diags:?}");
}

#[test]
fn deep_flat_expression_runs_and_returns_expected_value() {
    // Correctness, not just survival: 1 + 1000*1 = 1001.
    let src = chain_src(1000);
    let v = klang::with_deep_stack(move || {
        let mut p = Parser::new(&src);
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
        let mir = klang::mir::lower(&prog);
        let (v, _) = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
        v
    });
    assert_eq!(v, 1001);
}

#[test]
fn deep_nesting_does_not_crash_the_compiler() {
    // Nested parens and nested blocks: depth, not breadth.
    let parens = format!("fn main() -> i32 {{ return {}1{} }}", "(".repeat(200), ")".repeat(200));
    let v = klang::with_deep_stack(move || {
        let mut p = Parser::new(&parens);
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
        let mir = klang::mir::lower(&prog);
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs").0
    });
    assert_eq!(v, 1);
    let mut nested = String::from("fn main() -> i32 {\n");
    for _ in 0..300 {
        nested.push_str("    if true {\n");
    }
    nested.push_str("        return 42\n");
    for _ in 0..300 {
        nested.push_str("    }\n");
    }
    nested.push_str("}\n");
    let v = klang::with_deep_stack(move || {
        let mut p = Parser::new(&nested);
        let prog = p.parse_program().expect("300 nested blocks must parse");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
        let mir = klang::mir::lower(&prog);
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs").0
    });
    assert_eq!(v, 42);
}

#[test]
fn malformed_input_is_a_diagnostic_never_a_panic() {
    // Every one of these must produce a diagnostic (or clean parse), never a
    // crash: empty file, truncated fn, unterminated string/comment, garbage
    // bytes, absurd comment volume.
    let cases: Vec<String> = vec![
        String::new(),
        "fn".to_string(),
        "fn main() -> i32 {".to_string(),
        "fn main() -> i32 { return \"unterminated }".to_string(),
        "/* unterminated\nfn main() -> i32 { return 1 }".to_string(),
        "\u{e9}\u{1f600} fn main() -> i32 { return 1 }".to_string(),
        format!("// {}\nfn main() -> i32 {{ return 1 }}\n", "x".repeat(100_000)),
        format!("fn main() -> i32 {{ {} }}", "if true { return 1 } else { return 0 }\n".repeat(2000)),
    ];
    let results = klang::with_deep_stack(move || {
        cases
            .into_iter()
            .map(|src| {
                let mut p = Parser::new(&src);
                match p.parse_program() {
                    Err(d) => format!("E:{}", d.code),
                    Ok(prog) => match klang::hir::TypedHIR::check(prog) {
                        Ok(_) => "CLEAN".to_string(),
                        Err(ds) => format!("E:{}", ds[0].code),
                    },
                }
            })
            .collect::<Vec<String>>()
    });
    assert_eq!(results.len(), 8);
    for (i, r) in results.iter().enumerate() {
        assert!(
            r == "CLEAN" || r.starts_with("E:"),
            "case {i} produced no verdict: {r}"
        );
    }
    // The two unambiguously broken-but-parseable shapes must be diagnosed.
    assert!(results[5].starts_with("E:"), "garbage bytes: {}", results[5]);
}

#[test]
fn call_depth_limit_is_loud_never_a_crash() {
    // F20: the interpreter caps call depth at 64 frames. Depth 63 runs;
    // deeper recursion fails with E-RUNTIME "call depth exceeded", never
    // a native stack overflow. Pinned because the limit is low enough to
    // matter for realistic recursion and must stay loud, not silent.
    let ok_src = "fn countdown(n: i32) -> i32 { if n <= 0 { return 0 } return countdown(n - 1) + 1 } fn main() -> i32 { return countdown(63) }";
    let mut p = Parser::new(ok_src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    // Deep-stack worker: the interpreter recurses natively per call, so
    // even the allowed 63 frames need headroom on a default test thread.
    let (v, _) = klang::with_deep_stack(move || {
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("depth 63 runs")
    });
    assert_eq!(v, 63);
    let deep_src = "fn countdown(n: i32) -> i32 { if n <= 0 { return 0 } return countdown(n - 1) + 1 } fn main() -> i32 { return countdown(600) }";
    let mut q = Parser::new(deep_src);
    let prog2 = q.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog2.clone()).is_ok());
    let mir2 = klang::mir::lower(&prog2);
    let err = klang::with_deep_stack(move || {
        klang::runtime::run_with_output(&mir2, "main", &[], &HashMap::new()).expect_err("depth 600 must fail loudly")
    });
    assert_eq!(err.code, "E-RUNTIME", "got: {}", err.to_json());
    println!("depth-limit OK: {}", err.message);
}

#[test]
fn call_depth_exceeded_cause_is_not_concurrency() {
    // F14 (live heavy-testing): plain single-threaded unbounded recursion
    // reported cause "runtime failure during concurrent execution" — wrong,
    // no concurrency is involved. The depth guard gets its own accurate
    // cause; the concurrency string stays reserved for real task failures.
    let deep_src = "fn countdown(n: i32) -> i32 { if n <= 0 { return 0 } return countdown(n - 1) + 1 } fn main() -> i32 { return countdown(600) }";
    let mut q = Parser::new(deep_src);
    let prog2 = q.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog2.clone()).is_ok());
    let mir2 = klang::mir::lower(&prog2);
    let err = klang::with_deep_stack(move || {
        klang::runtime::run_with_output(&mir2, "main", &[], &HashMap::new()).expect_err("depth 600 must fail loudly")
    });
    assert_eq!(err.code, "E-RUNTIME", "got: {}", err.to_json());
    assert!(
        !err.cause.to_lowercase().contains("concurrent")
            && !err.cause.to_lowercase().contains("concurrency"),
        "depth-exceeded cause must not mention concurrency, got: {}",
        err.cause
    );
    assert!(
        err.cause.contains("depth") || err.cause.contains("stack"),
        "depth-exceeded cause should name the depth limit, got: {}",
        err.cause
    );
}

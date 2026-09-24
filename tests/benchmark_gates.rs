//! Phase 3 — AI-benchmark harness (offline baseline + live-model procedure).
//!
//! Task corpus across difficulty tiers, each run zero-shot (check only)
//! and repair-assisted (repair loop, <=5 iters). This file is the
//! checked-in corpus + the offline-runnable runner: `MockBackend`
//! stands in for the harness-owned model with a queued oracle fix, so
//! every metric below is computed through the real pipeline
//! (`check_candidate` for syntax/semantics, `run_repair` +
//! `RepairOutcome::iters_used` for convergence, diagnostic codes for the
//! "almost-right" category). Live-model convergence is measured inside
//! harnesses via `klang mcp`; `bench/run_live.sh` (companion) checks the
//! harness signal per task with zero model calls.
//!
//! Tiers: simple (single fn, no advanced features), moderate (one
//! advanced feature), complex (generics + enums + modules together).

use std::collections::HashMap;

use klang::repair::{
    MockBackend, RepairCliOverrides, RepairConfig, RepairFileConfig, Scope, check_candidate,
    run_repair,
};

struct Task {
    id: &'static str,
    tier: &'static str,
    broken: &'static str,
    /// Oracle fix a perfect model would return (queued into the mock).
    fixed: &'static str,
    /// Expected `main()` value after repair, when the task is runnable.
    expect_run: Option<i32>,
    /// The "almost-right" diagnostic family this task exercises.
    family: &'static str,
}

fn cfg() -> RepairConfig {
    RepairConfig::resolve(
        &RepairCliOverrides {
            scope: Some(Scope::Function),
            max_iters: Some(5),
            ..Default::default()
        },
        &RepairFileConfig::default(),
    )
}

fn corpus() -> Vec<Task> {
    vec![
        // ---- simple tier: single function, no generics/enums/modules ----
        Task {
            id: "S1-arity",
            tier: "simple",
            broken: "fn add(a: i32, b: i32) -> i32 {\n    return a + b\n}\n\nfn main() -> i32 {\n    return add(1)\n}\n",
            fixed: "fn main() -> i32 {\n    return add(1, 2)\n}\n",
            expect_run: Some(3),
            family: "E-ARITY",
        },
        Task {
            id: "S2-undefined",
            tier: "simple",
            broken: "fn main() -> i32 {\n    return x\n}\n",
            fixed: "fn main() -> i32 {\n    return 42\n}\n",
            expect_run: Some(42),
            family: "E-UNDEFINED",
        },
        Task {
            id: "S3-type",
            tier: "simple",
            broken: "fn main() -> i32 {\n    return \"hi\"\n}\n",
            fixed: "fn main() -> i32 {\n    return 42\n}\n",
            expect_run: Some(42),
            family: "E-TYPE",
        },
        // ---- moderate tier: exactly one advanced feature ----
        Task {
            id: "M1-exhaustive",
            tier: "moderate",
            broken: "enum Opt { Some(x: i32), None }\n\nfn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n }\n}\n\nfn main() -> i32 {\n    return pick(Opt::Some(1))\n}\n",
            fixed: "fn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n, Opt::None => 0 }\n}\n",
            expect_run: Some(1),
            family: "E-MATCH-EXHAUSTIVE",
        },
        Task {
            id: "M2-generic",
            tier: "moderate",
            broken: "fn same<T>(a: T, b: T) -> T {\n    return a\n}\n\nfn main() -> i32 {\n    return same(1, \"hi\")\n}\n",
            // Whole-file oracle: the conflicting-instantiation fix rewrites
            // the caller, and file scope accepts the complete program.
            fixed: "fn same<T>(a: T, b: T) -> T {\n    return a\n}\n\nfn main() -> i32 {\n    return same(1, 2)\n}\n",
            expect_run: Some(1),
            family: "E-TYPE",
        },
        Task {
            id: "M3-effect",
            tier: "moderate",
            broken: "fn f() -> i32 throws {\n    return 1\n}\n\nfn g() -> i32 {\n    return f()\n}\n\nfn main() -> i32 {\n    return 0\n}\n",
            fixed: "fn g() -> i32 throws {\n    return f()\n}\n",
            expect_run: None, // effect shape: convergence + clean check is the verdict
            family: "E-EFFECT-MISMATCH",
        },
        // ---- complex tier: generics + enums + modules together ----
        Task {
            id: "C1-full-stack",
            tier: "complex",
            broken: "enum Result<T> { Ok(v: T), Err }\nstruct Store<T> { val: T }\nmod math {\n    pub fn double(x: i32) -> i32 {\n        return x + x\n    }\n}\nfn get(r: Result) -> i32 {\n    return match r { Result::Ok(v) => v }\n}\nfn main() -> i32 {\n    let r = Result::Ok(10)\n    let s = Store { val: get(r) }\n    return math::double(s.val)\n}\n",
            fixed: "fn get(r: Result) -> i32 {\n    return match r { Result::Ok(v) => v, Result::Err => 0 }\n}\n",
            expect_run: Some(20),
            family: "E-MATCH-EXHAUSTIVE",
        },
        Task {
            id: "C2-multi-target",
            tier: "complex",
            broken: "fn bad_a() -> i32 {\n    return nope_a\n}\n\nfn bad_b() -> i32 {\n    return nope_b\n}\n\nfn keep(x: i32) -> i32 {\n    return x + 1\n}\n\nfn main() -> i32 {\n    return keep(1)\n}\n",
            fixed: "fn bad_a() -> i32 {\n    return 1\n}\n\nfn bad_b() -> i32 {\n    return 2\n}\n",
            expect_run: None, // convergence + keep-preservation is the verdict
            family: "E-UNDEFINED",
        },
    ]
}

fn parses(src: &str) -> bool {
    let mut p = klang::parser::Parser::new(src);
    p.parse_program().is_ok()
}

fn run_main(src: &str) -> Option<i32> {
    let mut p = klang::parser::Parser::new(src);
    let prog = p.parse_program().ok()?;
    if klang::hir::TypedHIR::check(prog.clone()).is_err() {
        return None;
    }
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).ok().map(|(v, _)| v)
}

#[test]
fn benchmark_offline_baseline() {
    let tasks = corpus();
    let cfg = cfg();
    // Per-task rows: (id, tier, syntax_ok, zero_shot_clean, repair_ok, iters, run_ok, family)
    let mut rows: Vec<(String, String, bool, bool, bool, u32, bool, String)> = Vec::new();
    for t in &tasks {
        let syntax_ok = parses(t.broken);
        let zero_shot_clean = check_candidate(t.broken).diagnostics.is_empty();
        let backend = MockBackend::new(vec![t.fixed.to_string()]);
        let outcome = run_repair(t.broken, "bench.klang", &cfg, &backend, false);
        let run_ok = match t.expect_run {
            Some(want) => run_main(&outcome.source) == Some(want),
            None => check_candidate(&outcome.source).diagnostics.is_empty(),
        };
        // No-regression guard for C2: untouched `keep` must survive byte-identical.
        if t.id == "C2-multi-target" && outcome.success {
            assert!(
                outcome.source.contains("fn keep(x: i32) -> i32"),
                "repair must preserve untouched fn: {}",
                outcome.source
            );
        }
        rows.push((
            t.id.to_string(),
            t.tier.to_string(),
            syntax_ok,
            zero_shot_clean,
            outcome.success,
            outcome.iters_used,
            run_ok,
            t.family.to_string(),
        ));
    }

    let total = rows.len() as f64;
    let syntax_valid = rows.iter().filter(|r| r.2).count() as f64;
    let repair_ok = rows.iter().filter(|r| r.4).count() as f64;
    let run_ok = rows.iter().filter(|r| r.6).count() as f64;
    let mut iters: Vec<u32> = rows.iter().map(|r| r.5).collect();
    iters.sort_unstable();
    let median = iters[iters.len() / 2];
    let simple: Vec<_> = rows.iter().filter(|r| r.1 == "simple").collect();
    let simple_syntax = simple.iter().filter(|r| r.2).count() as f64 / simple.len() as f64;

    // Human-readable table (visible with `cargo test -- --nocapture`).
    println!("id,tier,syntax_ok,zero_shot_clean,repair_ok,iters,run_ok,family");
    for r in &rows {
        println!("{},{},{},{},{},{},{},{}", r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7);
    }
    println!(
        "SUMMARY total={} syntax={:.0}% repair={:.0}% run={:.0}% median_iters={} simple_syntax={:.0}%",
        total,
        syntax_valid / total * 100.0,
        repair_ok / total * 100.0,
        run_ok / total * 100.0,
        median,
        simple_syntax * 100.0,
    );
    println!("NOTE live-model zero-shot semantic rates require two real endpoints (see bench/run_live.sh); mock-oracle rows above measure loop mechanics, not model intelligence.");

    // PRD 6.2 offline-measurable targets (mock oracle = perfect model):
    // - syntax validity of the corpus inputs is 100% by construction
    //   (mutated checked-in programs, not model output);
    // - repair-assisted convergence with an oracle fix must be 100% in <=5
    //   iters with median <= 2 and every runnable task executing correctly.
    assert_eq!(syntax_valid as usize, rows.len(), "corpus inputs must all parse");
    assert_eq!(repair_ok as usize, rows.len(), "oracle repair must converge on every task");
    assert!(median <= 2, "median iters {median} exceeds PRD target <= 2");
    assert!(iters.iter().all(|&i| i <= 5), "all tasks must converge within 5 iters: {iters:?}");
    assert_eq!(run_ok as usize, rows.len(), "every converged task must run/check correctly");
    // Zero-shot column documents the gap the loop closes: every broken
    // input must indeed fail the check (otherwise the task is vacuous).
    assert!(rows.iter().all(|r| !r.3), "each broken input must fail zero-shot check");
    // "Almost-right" families represented: arity/type/undefined, effect,
    // and match-exhaustiveness across tiers.
    for want in ["E-ARITY", "E-TYPE", "E-UNDEFINED", "E-EFFECT-MISMATCH", "E-MATCH-EXHAUSTIVE"] {
        assert!(rows.iter().any(|r| r.7 == want), "missing family {want}");
    }
}

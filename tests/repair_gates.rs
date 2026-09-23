//! `klang repair` gates: bounded diagnostic-guided regeneration (PRD v1).
//!
//! All tests use `MockBackend` (no network). Live-endpoint validation is
//! manual (Phase 1): point `KLANG_MODEL_ENDPOINT` at the Kaggle/ngrok URL
//! and run the binary.

use klang::repair::{
    MockBackend, ModelBackend, RepairCliOverrides, RepairConfig, RepairFileConfig, Scope,
    build_system_prompt, build_user_prompt, check_candidate, plan_scope, run_repair,
};
use klang::repair::scope::RepairScope;

fn cfg_with(scope: Scope, iters: u32) -> RepairConfig {
    RepairConfig::resolve_with(
        &RepairCliOverrides {
            endpoint: Some("http://mock/v1".into()),
            scope: Some(scope),
            max_iters: Some(iters),
            ..Default::default()
        },
        &RepairFileConfig::default(),
        &[],
    )
    .expect("cfg")
}

#[test]
fn clean_source_is_noop_without_model_calls() {
    let b = MockBackend::new(vec![]);
    let o = run_repair(
        "fn main() -> i32 { return 42 }\n",
        "t.klang",
        &cfg_with(Scope::Function, 5),
        &b,
        false,
    );
    assert!(o.success);
    assert_eq!(o.iters_used, 0);
    assert!(o.attempts.is_empty());
    assert_eq!(b.calls(), 0, "clean input must not call the model");
}

#[test]
fn dry_run_plans_prompt_and_never_calls() {
    let b = MockBackend::new(vec!["fn main() -> i32 { return 1 }\n".into()]);
    let o = run_repair(
        "fn main() -> i32 { return x }\n",
        "t.klang",
        &cfg_with(Scope::File, 5),
        &b,
        true,
    );
    assert!(!o.success, "dry-run never succeeds");
    assert_eq!(b.calls(), 0, "dry-run must make zero model calls");
    assert_eq!(o.attempts.len(), 1);
    assert!(o.attempts[0].prompt_user.contains("E-UNDEFINED"), "{}", o.attempts[0].prompt_user);
    assert!(o.attempts[0].prompt_system.contains("task_group"), "{}", o.attempts[0].prompt_system);
}

#[test]
fn file_scope_fixes_type_mismatch_in_one_iter() {
    let b = MockBackend::new(vec!["fn main() -> i32 { return 42 }\n".into()]);
    let o = run_repair(
        "fn main() -> i32 { return x }\n",
        "t.klang",
        &cfg_with(Scope::File, 5),
        &b,
        false,
    );
    assert!(o.success, "final: {:?}", o.final_diagnostics);
    assert_eq!(o.iters_used, 1, "median convergence <= 2 (PRD success metric)");
    assert!(o.source.contains("return 42"), "{}", o.source);
}

#[test]
fn function_scope_leaves_unrelated_code_untouched() {
    let src = "fn keep() -> i32 {\n    return 7\n}\n\nfn bad() -> i32 {\n    return x\n}\n";
    let b = MockBackend::new(vec!["fn bad() -> i32 {\n    return 8\n}\n".into()]);
    let o = run_repair(src, "t.klang", &cfg_with(Scope::Function, 5), &b, false);
    assert!(o.success, "final: {:?}", o.final_diagnostics);
    assert!(o.source.contains("return 7"), "unrelated fn must survive: {}", o.source);
    assert!(o.source.contains("return 8"), "repaired fn present: {}", o.source);
    assert!(!o.source.contains("return x"), "bug gone: {}", o.source);
}

#[test]
fn effect_mismatch_attributed_to_caller_and_fixed() {
    let src = "fn f() -> i32 throws {\n    return 1\n}\n\nfn g() -> i32 {\n    return f()\n}\n";
    // Planner must pick `g` (the caller missing `throws`), not `f`.
    let diags = check_candidate(src).diagnostics;
    assert!(diags.iter().any(|d| d.code == "E-EFFECT-MISMATCH"));
    assert_eq!(
        plan_scope(&diags, src, false),
        RepairScope::Functions(vec!["g".into()])
    );
    let b = MockBackend::new(
        vec!["fn g() -> i32 throws {\n    return f()\n}\n".into()],
    );
    let o = run_repair(src, "t.klang", &cfg_with(Scope::Function, 5), &b, false);
    assert!(o.success, "final: {:?}", o.final_diagnostics);
    assert!(o.source.contains("throws"), "{}", o.source);
}

#[test]
fn arity_error_fixed_at_caller() {
    let src = "fn add(a: i32, b: i32) -> i32 {\n    return a + b\n}\n\nfn main() -> i32 {\n    return add(1)\n}\n";
    let b = MockBackend::new(
        vec!["fn main() -> i32 {\n    return add(1, 2)\n}\n".into()],
    );
    let o = run_repair(src, "t.klang", &cfg_with(Scope::Function, 5), &b, false);
    assert!(o.success, "final: {:?}", o.final_diagnostics);
}

#[test]
fn exhausted_budget_returns_final_diagnostics() {
    // Mock keeps returning the same broken source: repeat detected +
    // budget of 2 exhausted -> failure with final JSON diagnostics.
    let b = MockBackend::new(vec!["fn main() -> i32 { return x }\n".into()]);
    let o = run_repair(
        "fn main() -> i32 { return x }\n",
        "t.klang",
        &cfg_with(Scope::File, 2),
        &b,
        false,
    );
    assert!(!o.success);
    assert_eq!(o.iters_used, 2);
    assert!(!o.final_diagnostics.is_empty());
    assert!(o.final_diagnostics.iter().any(|d| d.code == "E-UNDEFINED"));
}

#[test]
fn model_parse_failure_is_failed_attempt_not_panic() {
    let b = MockBackend::new(vec!["this is not klang at all (((".into()]);
    let o = run_repair(
        "fn main() -> i32 { return x }\n",
        "t.klang",
        &cfg_with(Scope::File, 1),
        &b,
        false,
    );
    assert!(!o.success);
    assert!(o.final_diagnostics.iter().any(|d| d.code == "E-PARSE" || d.code == "E-UNDEFINED"));
}

#[test]
fn unsafe_import_in_response_rejected() {
    let b = MockBackend::new(vec!["import \"../../etc/passwd\"\nfn main() -> i32 { return 1 }\n".into()]);
    let o = run_repair(
        "fn main() -> i32 { return x }\n",
        "t.klang",
        &cfg_with(Scope::File, 1),
        &b,
        false,
    );
    assert!(!o.success);
    assert!(o.attempts[0].note.contains("unsafe import"), "{}", o.attempts[0].note);
}

#[test]
fn prompt_carries_json_and_history() {
    let d = klang::diagnostics::Diagnostic::error("E-TYPE", "m", "f", 0, 1, "c", &["fix it"], "r");
    let u = build_user_prompt("function f", "fn f() -> i32 { return 1 }", &[d], &["prior try failed".into()], None);
    assert!(u.contains("E-TYPE"), "{u}");
    assert!(u.contains("fix it"), "{u}");
    assert!(u.contains("do NOT repeat"), "{u}");
    assert!(build_system_prompt().contains("E-TASK-CANCEL") || build_system_prompt().contains("task_group"));
}

// ---------------------------------------------------------------------
// Adversarial Test 1: ambiguous function attribution must fall back to
// file scope, never silently guess one of several plausible functions.
//
// Design: three independent callers each trigger their own diagnostic
// naming a *different* function (one E-ARITY per caller plus one
// E-UNDEFINED naming a third). The planner's stated rule is "file
// fallback when > MAX_FUNCTION_SCOPE (=2) functions" -- this test
// proves that rule fires on real compiler diagnostics, not just on
// hand-built Diagnostic structs.
//
// API mapping (draft -> real):
//   `klang::check_source` does not exist -> `check_candidate(src)`
//     (parse + full TypedHIR::check, same path the driver uses).
//   `scope::candidate_functions` does not exist -> the union of
//     per-diagnostic attributions is observable through `plan_scope`
//     (private `attribute()` has no public accessor by design); the
//     fallback itself (File vs Functions) IS the externally visible
//     verdict, asserted with the real `RepairScope::File`.
//   `scope::plan_scope(src, diags)` -> real order is
//     `plan_scope(diags, src, force_file)`.
//   `ScopeKind::File` -> real `RepairScope::File` (scope::RepairScope).
// Finding per instruction (3): there is no public `target_span()` or
// `ScopeKind` on the planner, and no `candidate_functions` accessor.
// The verdict (File vs Functions(names)) is fully inspectable -- that
// is the public contract -- but per-function byte spans live in
// `scope::function_spans`, not on the plan. No backdoor added.
// ---------------------------------------------------------------------
#[test]
fn ambiguous_attribution_falls_back_to_file_scope() {
    use klang::repair::scope::function_spans;

    let ambiguous_src = "fn add(a: i32, b: i32) -> i32 {\n    return a + b\n}\n\nfn caller_a() -> i32 {\n    return add(1)\n}\n\nfn caller_b() -> i32 {\n    return add(1, 2, 3)\n}\n\nfn caller_c() -> i32 {\n    return missing_var\n}\n";
    let diagnostics = check_candidate(ambiguous_src).diagnostics;
    // Sanity: the fixture must actually produce diagnostics touching >2
    // distinct functions, otherwise the test is not exercising the
    // fallback path at all. (Two E-ARITY diagnostics naming caller_a /
    // caller_b, plus one E-UNDEFINED naming `missing_var` used only in
    // caller_c -> three distinct attribution targets.)
    assert!(
        diagnostics.len() >= 3,
        "fixture must yield >=3 diagnostics to exercise fallback; got {:?}",
        diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    let names: std::collections::HashSet<String> = diagnostics
        .iter()
        .flat_map(|d| {
            // Mirror the planner's own name extraction well enough to
            // count distinct attribution targets: backticked tokens that
            // name a known function, plus use-sites of other backticked
            // names. (Uses only public API: diagnostics + function_spans.)
            let spans = function_spans(ambiguous_src);
            let known: Vec<&str> = spans.iter().map(|(n, _, _)| n.as_str()).collect();
            let mut hits = Vec::new();
            let mut parts = d.message.split('`');
            let mut idx = 0;
            for part in parts.by_ref() {
                idx += 1;
                if idx % 2 == 0 && known.contains(&part) {
                    hits.push(part.to_string());
                }
            }
            // E-ARITY/E-EFFECT name the caller first in backticks; for
            // other backticked names, count every function mentioning them.
            if hits.is_empty() {
                let mut parts = d.message.split('`');
                let mut idx = 0;
                for part in parts.by_ref() {
                    idx += 1;
                    if idx % 2 == 0 && !part.is_empty() {
                        for (n, s, e) in &spans {
                            if ambiguous_src[*s..*e].contains(part) && !hits.contains(n) {
                                hits.push(n.clone());
                            }
                        }
                    }
                }
            }
            hits
        })
        .collect();
    assert!(
        names.len() > 2,
        "fixture must attribute to >2 distinct functions to exercise fallback; got {names:?}"
    );

    assert_eq!(
        plan_scope(&diagnostics, ambiguous_src, false),
        RepairScope::File,
        "attribution should refuse to guess among 3 distinct targets \
         and fall back to file scope, not silently pick one"
    );
    // Boundary control: the same fixture with only two failing callers
    // must stay at function scope. This proves the File verdict above
    // comes from the >MAX_FUNCTION_SCOPE count rule firing -- not from
    // attribution failing altogether (which also yields File, and would
    // make the main assertion vacuous).
    let two_diags: Vec<_> = diagnostics.iter().take(2).cloned().collect();
    assert_eq!(two_diags.len(), 2);
    let two_plan = plan_scope(&two_diags, ambiguous_src, false);
    assert!(
        matches!(two_plan, RepairScope::Functions(_)),
        "two failing callers must stay at function scope (boundary control); got {two_plan:?}"
    );
}

// ---------------------------------------------------------------------
// Adversarial Test 2: function-scoped repair must leave every byte
// outside the target function's span untouched.
//
// Design: two functions -- one broken (undefined variable, unambiguous
// single-function scope), one clean carrying splice-fragile content
// (comment, odd spacing, trailing blank line). MockBackend returns a
// fix for ONLY the broken function. After repair, everything outside
// the target's original byte range must match byte-for-byte.
//
// API mapping (draft -> real):
//   `ScopeKind::Function` -> real `RepairScope::Functions(names)`;
//     single-target scope asserted as `Functions(vec!["bad".into()])`.
//   `plan.target_span()` does not exist -> target byte range comes from
//     the public `scope::function_spans(src)` table (name -> start/end),
//     the same table the splicer itself uses. Finding per instruction
//     (3): the plan carries names, not spans; spans are looked up, not
//     stored. No backdoor added.
//   `MockBackend::returning(s)` -> real `MockBackend::new(vec![s])`.
//   `driver::run_repair_with_backend(src, plan, backend, iters)` -> real
//     `run_repair(src, label, &cfg, backend, dry_run)` with a function-
//     scope config (planner runs inside the driver on real diagnostics).
//   `repaired.source` -> real `RepairOutcome.source`; re-check via
//     `check_candidate(&source).diagnostics.is_empty()`.
// ---------------------------------------------------------------------
#[test]
fn function_scope_splice_preserves_untouched_bytes_exactly() {
    use klang::repair::scope::function_spans;

    let two_fn_src = "fn bad() -> i32 {\n    return nope\n}\n\n// this comment and the odd spacing below must survive untouched\nfn clean(  x: i32 ) -> i32 {\n    return x + 1\n}\n";
    let diagnostics = check_candidate(two_fn_src).diagnostics;
    assert!(
        diagnostics.iter().any(|d| d.code == "E-UNDEFINED"),
        "expected an undefined-variable diagnostic on `bad`, got {:?}",
        diagnostics.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
    // This fixture must resolve to a single unambiguous function scope --
    // otherwise the byte-range assertion below is meaningless.
    assert_eq!(
        plan_scope(&diagnostics, two_fn_src, false),
        RepairScope::Functions(vec!["bad".into()]),
        "this fixture should resolve to a single unambiguous function scope"
    );

    // Target byte range from the same public span table the splicer uses.
    let (t_start, t_end) = function_spans(two_fn_src)
        .into_iter()
        .find(|(n, _, _)| n == "bad")
        .map(|(_, s, e)| (s, e))
        .expect("function-scope plan target must have a span");
    let original_prefix = &two_fn_src[..t_start];
    let original_suffix = &two_fn_src[t_end..];

    // Mock model returns a fix for exactly the targeted function.
    let backend = MockBackend::new(vec!["fn bad() -> i32 {\n    return 1\n}\n".into()]);
    let outcome = run_repair(
        two_fn_src,
        "t.klang",
        &cfg_with(Scope::Function, 1),
        &backend,
        false,
    );
    assert!(outcome.success, "repair should converge in one attempt, got {:?}", outcome.final_diagnostics);
    // The repaired file must re-check clean.
    assert!(
        check_candidate(&outcome.source).diagnostics.is_empty(),
        "repaired source must pass check"
    );
    // Byte-diff everything outside the target function's original span.
    assert!(
        outcome.source.starts_with(original_prefix),
        "bytes before the target function span were modified by the splice"
    );
    assert!(
        outcome.source.ends_with(original_suffix),
        "bytes after the target function span were modified by the splice \
         (comment, spacing, or trailing content in `clean` was disturbed)"
    );
}

// ---------------------------------------------------------------------
// Adversarial Test 3: multi-target splice (2 functions, one pass) must
// patch BOTH targets while leaving a third function's bytes identical.
//
// Design: exactly 2 independent, unambiguous diagnostics in 2 different
// functions (E-UNDEFINED `nope_a` used only in `bad_a`, E-UNDEFINED
// `nope_b` used only in `bad_b`) -- not 3, which would hit the File
// fallback instead of the Functions(_) path under test. A third clean
// function carries splice-fragile content (comment, odd spacing,
// trailing newline). MockBackend returns fixes for BOTH targets in a
// single response; the driver budget is 1 so convergence must happen in
// one pass (no second-attempt rescue masking a broken first splice).
//
// Byte check: with 2 non-adjacent target spans (bad_a first, bad_b
// middle, keep last), the untouched region is verified as (a) the exact
// keep-function slice re-appearing byte-identical in the output, and
// (b) the inter-target gap (the "\n\n" between bad_a and bad_b) still
// separating the two repaired functions. Prefix-before-first-target is
// empty here (bad_a is at offset 0), which is itself asserted -- the
// meaningful untouched assertions are the gap and the keep slice.
// ---------------------------------------------------------------------
#[test]
fn multi_target_splice_patches_both_and_preserves_third() {
    use klang::repair::scope::function_spans;

    let three_fn_src = "fn bad_a() -> i32 {\n    return nope_a\n}\n\nfn bad_b() -> i32 {\n    return nope_b\n}\n\n// keep: comment and odd spacing must survive untouched\nfn keep(  x: i32 ) -> i32 {\n    return x + 1\n}\n";
    let diagnostics = check_candidate(three_fn_src).diagnostics;
    assert_eq!(
        diagnostics.len(),
        2,
        "fixture must yield exactly 2 diagnostics (not 3 -> File fallback), got {:?}",
        diagnostics.iter().map(|d| (&d.code, &d.message)).collect::<Vec<_>>()
    );
    // Exactly the 2-function boundary: must stay Functions(_), in file
    // order. This is the non-vacuous gate -- if the planner escalated to
    // File here, the splice path under test would never run.
    assert_eq!(
        plan_scope(&diagnostics, three_fn_src, false),
        RepairScope::Functions(vec!["bad_a".into(), "bad_b".into()]),
        "exactly 2 failing functions must stay at function scope"
    );

    // Original spans, in file order: bad_a | gap | bad_b | gap | keep.
    let spans = function_spans(three_fn_src);
    assert_eq!(spans.len(), 3, "fixture must parse to 3 functions");
    let (a_s, a_e) = spans.iter().find(|(n, _, _)| n == "bad_a").map(|(_, s, e)| (*s, *e)).expect("bad_a span");
    let (b_s, b_e) = spans.iter().find(|(n, _, _)| n == "bad_b").map(|(_, s, e)| (*s, *e)).expect("bad_b span");
    let (k_s, k_e) = spans.iter().find(|(n, _, _)| n == "keep").map(|(_, s, e)| (*s, *e)).expect("keep span");
    assert_eq!(a_s, 0, "bad_a leads the file so the pre-target prefix is empty (asserted, not assumed)");
    assert!(a_e <= b_s && b_e <= k_s, "spans must be ordered and disjoint");
    let keep_slice = &three_fn_src[k_s..k_e];
    let gap_a_b = &three_fn_src[a_e..b_s];

    // One response carrying BOTH fixes; budget 1 forces single-pass success.
    let backend = MockBackend::new(vec![
        "fn bad_a() -> i32 {\n    return 1\n}\n\nfn bad_b() -> i32 {\n    return 2\n}\n".into(),
    ]);
    let outcome = run_repair(
        three_fn_src,
        "t.klang",
        &cfg_with(Scope::Function, 1),
        &backend,
        false,
    );
    assert!(outcome.success, "both targets must converge in ONE pass, got {:?}", outcome.final_diagnostics);
    assert_eq!(outcome.iters_used, 1, "must converge in a single attempt (no second-pass rescue)");
    assert!(
        check_candidate(&outcome.source).diagnostics.is_empty(),
        "repaired source must pass check"
    );
    // Both targets actually changed to the mocked fix content...
    assert!(outcome.source.contains("return 1"), "bad_a fix missing: {}", outcome.source);
    assert!(outcome.source.contains("return 2"), "bad_b fix missing: {}", outcome.source);
    assert!(!outcome.source.contains("nope_a"), "bad_a bug survives: {}", outcome.source);
    assert!(!outcome.source.contains("nope_b"), "bad_b bug survives: {}", outcome.source);
    // ...while the third function's bytes are byte-identical.
    assert!(
        outcome.source.contains(keep_slice),
        "untouched `keep` function bytes were modified by the multi-target splice: {}",
        outcome.source
    );
    // Inter-target gap preserved: repaired bad_a and bad_b still separated
    // by the original gap bytes (offset-shift after the first replacement
    // must not corrupt the second target's placement).
    let out_spans = function_spans(&outcome.source);
    let out_a_e = out_spans.iter().find(|(n, _, _)| n == "bad_a").map(|(_, _, e)| *e).expect("repaired bad_a span");
    let out_b_s = out_spans.iter().find(|(n, _, _)| n == "bad_b").map(|(_, s, _)| *s).expect("repaired bad_b span");
    assert_eq!(
        &outcome.source[out_a_e..out_b_s], gap_a_b,
        "gap between the two repaired functions changed (offset-shift corruption)"
    );
}

// ---------------------------------------------------------------------
// Phase 1 decision (PRD): declaration-level fixes.
//
// The enums/match work surfaced a real gap: a fix that requires editing a
// type/enum/module DECLARATION (outside any function body) is not
// expressible by function-scoped splicing. Two behaviors are pinned here:
//   1. the planner never claims such a diagnostic is function-scoped
//      (`is_declaration_level` -> File scope), and
//   2. the splicer never silently drops declaration text from a model
//      response: it either accepts a complete file or fails loudly.
// ---------------------------------------------------------------------

const DECL_SRC: &str = "enum Opt { Some(x: i32), None }\n\nfn helper() -> i32 {\n    return 1\n}\n\nfn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n }\n}\n";

#[test]
fn declaration_edit_in_incomplete_response_is_never_silently_dropped() {
    use klang::repair::splice::{response_declarations, splice_functions};
    // The model re-emits the enum + the target fn but omits `helper`.
    // Silently splicing only `pick` would discard the declaration edit, so
    // this must be a loud failure instead.
    let raw = "enum Opt { Some(x: i32), None }\n\nfn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n, Opt::None => 0 }\n}\n";
    assert!(
        !response_declarations(raw).is_empty(),
        "fixture response must contain declaration text"
    );
    let err = splice_functions(DECL_SRC, &["pick".to_string()], raw)
        .expect_err("must refuse to drop the declaration edit");
    assert!(err.contains("declaration-level edit"), "{err}");
    assert!(err.contains("helper"), "error must name what is missing: {err}");
    // Same through the driver: a failed attempt whose note says why, never
    // a silent rewrite of `pick` only.
    let b = MockBackend::new(vec![raw.into()]);
    let o = run_repair(DECL_SRC, "t.klang", &cfg_with(Scope::Function, 1), &b, false);
    assert!(!o.success, "must not report success after dropping a declaration edit");
    assert!(
        o.attempts[0].note.contains("declaration-level edit"),
        "{note}",
        note = o.attempts[0].note
    );
    assert_eq!(o.source, DECL_SRC, "source must be left untouched on refusal");
}

#[test]
fn declaration_edit_with_complete_file_is_accepted() {
    // Whole-file response (declarations + every function) is accepted and
    // re-checked clean.
    let raw = "enum Opt { Some(x: i32), None }\n\nfn helper() -> i32 {\n    return 1\n}\n\nfn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n, Opt::None => 0 }\n}\n";
    let b = MockBackend::new(vec![raw.into()]);
    let o = run_repair(DECL_SRC, "t.klang", &cfg_with(Scope::Function, 1), &b, false);
    assert!(o.success, "complete file must be accepted, got {:?}", o.final_diagnostics);
    assert!(check_candidate(&o.source).diagnostics.is_empty());
    assert!(o.source.contains("Opt::None => 0"), "{}", o.source);
    assert!(o.source.contains("fn helper"), "declarations/other fns preserved: {}", o.source);
}

#[test]
fn declaration_free_response_preserves_original_declarations() {
    // A pure function-scope response (no declaration text) must leave the
    // original enum declaration byte-identical.
    use klang::repair::scope::function_spans;
    let b = MockBackend::new(vec!["fn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n, Opt::None => 7 }\n}\n".into()]);
    let o = run_repair(DECL_SRC, "t.klang", &cfg_with(Scope::Function, 1), &b, false);
    assert!(o.success, "got {:?}", o.final_diagnostics);
    let orig_prefix = &DECL_SRC[..function_spans(DECL_SRC)[0].1];
    assert!(
        o.source.starts_with(orig_prefix),
        "enum declaration must survive byte-identical: {}",
        o.source
    );
    assert!(o.source.contains("fn helper"), "{}", o.source);
}

#[test]
fn declaration_level_diagnostics_force_file_scope() {
    use klang::repair::scope::is_declaration_level;
    // Duplicate declaration: rule `names/duplicate` -> File scope.
    let dup = "struct Point { x: i32 }\nstruct Point { y: i32 }\nfn main() -> i32 {\n    return 1\n}\n";
    let d = check_candidate(dup).diagnostics;
    assert!(d.iter().any(|x| x.code == "E-DUPLICATE"), "got {:?}", d.iter().map(|x| &x.code).collect::<Vec<_>>());
    assert!(d.iter().any(is_declaration_level), "E-DUPLICATE is declaration-level");
    assert_eq!(plan_scope(&d, dup, false), RepairScope::File);
    // Visibility: rule `modules/visibility` -> File scope, even though the
    // callee name IS a function (name attribution alone used to pick it).
    let priv_src = "mod lexer {\n    fn helper() -> i32 {\n        return 1\n    }\n}\nfn main() -> i32 {\n    return lexer::helper()\n}\n";
    let d = check_candidate(priv_src).diagnostics;
    let pd = d.iter().find(|x| x.code == "E-PRIVATE").expect("E-PRIVATE");
    assert!(is_declaration_level(pd), "E-PRIVATE is declaration-level");
    assert_eq!(plan_scope(&d, priv_src, false), RepairScope::File);
    // Boundary control: a function-body diagnostic is NOT declaration-level
    // (the escalation must not swallow the normal function-scope path).
    let body_src = "fn bad() -> i32 {\n    return nope\n}\n";
    let d = check_candidate(body_src).diagnostics;
    assert!(!d.iter().any(is_declaration_level), "body diagnostics stay function-scoped");
    assert_eq!(plan_scope(&d, body_src, false), RepairScope::Functions(vec!["bad".into()]));
}

#[test]
fn declaration_detection_ignores_comments_and_strings() {
    use klang::repair::splice::response_declarations;
    // Top-level comment and in-body text must not register as declarations.
    let src = "// enum Fake, struct Fake, mod fake\nfn main() -> i32 {\n    // enum AlsoFake\n    print(\"enum InString\")\n    return 1\n}\n";
    assert!(
        response_declarations(src).is_empty(),
        "false positive: {:?}",
        response_declarations(src)
    );
    // Real declarations still register (both file level and module members).
    let real = "import \"lib.klang\"\nenum Opt { A }\nstruct S { v: i32 }\nmod util { pub fn w() -> i32 { return 1 } }\nfn main() -> i32 { return util::w() }\n";
    let found = response_declarations(real);
    for want in ["enum", "struct", "mod", "import"] {
        assert!(
            found.iter().any(|(k, _)| k == want),
            "missing {want} in {found:?}"
        );
    }
}

#[test]
fn match_diagnostic_scopes_to_function_containing_the_match() {
    // Phase 0 finding: `E-MATCH-EXHAUSTIVE` names the enum, so the generic
    // use-site scan widened the scope to every function that merely
    // mentions the enum (`main` below calls `pick(Opt::Some(1))`). The
    // scope must stay on the function that actually contains the `match`.
    let src = "enum Opt { Some(x: i32), None }\n\nfn helper() -> i32 {\n    return 1\n}\n\nfn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n }\n}\n\nfn main() -> i32 {\n    return pick(Opt::Some(1)) + helper()\n}\n";
    let diags = check_candidate(src).diagnostics;
    assert!(diags.iter().any(|d| d.rule == "match/exhaustiveness"), "fixture must produce the match diagnostic");
    assert_eq!(
        plan_scope(&diags, src, false),
        RepairScope::Functions(vec!["pick".into()]),
        "scope must be the function containing the match, not every enum mention"
    );
    // And a fn-only response for exactly that scope now converges while
    // preserving the declaration and the untouched functions.
    let b = MockBackend::new(vec!["fn pick(v: Opt) -> i32 {\n    return match v { Opt::Some(n) => n, Opt::None => 7 }\n}\n".into()]);
    let o = run_repair(src, "t.klang", &cfg_with(Scope::Function, 1), &b, false);
    assert!(o.success, "got {:?}", o.final_diagnostics);
    assert!(o.source.contains("fn helper"), "untouched fn preserved: {}", o.source);
    assert!(o.source.contains("Opt::None => 7"), "{}", o.source);
    assert!(o.source.contains("enum Opt { Some(x: i32), None }"), "declaration preserved: {}", o.source);
}

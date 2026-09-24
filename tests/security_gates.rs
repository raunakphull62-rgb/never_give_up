//! Security regression gates: the compiler as an attack surface.
//!
//! Distinct from the Phase 2 correctness findings (F1–F11): these pin
//! that malicious or adversarial input — hostile `.klang` files,
//! hostile model responses, hostile endpoints — cannot do worse than
//! produce a diagnostic or a failed attempt. Findings fixed here are
//! F12–F17 in AUDIT.md; non-findings the audit confirmed (import-cycle
//! termination, task-group drain, FNV non-use, clean dependency scan)
//! are pinned where cheap.

use std::collections::HashMap;

use klang::parser::Parser;

// ---------------------------------------------------------------------
// F12 — `match` checking was quadratic in arm count (21s for 20k arms).
// ---------------------------------------------------------------------

fn big_match_src(n: usize) -> String {
    let mut s = String::from("enum E { ");
    for i in 0..n {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("V{i}"));
    }
    s.push_str(" } fn f(e: E) -> i32 { return match e { ");
    for i in 0..n {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("E::V{i} => {i}"));
    }
    s.push_str(" } } fn main() -> i32 { return f(E::V7()) }");
    s
}

#[test]
fn large_match_checks_clean_and_dispatches() {
    // 3000 arms must check clean (HashSet coverage, not Vec scans) and
    // still dispatch to the right arm at runtime.
    let src = big_match_src(3000);
    let mut p = Parser::new(&src);
    let prog = p.parse_program().expect("parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 7);
}

// ---------------------------------------------------------------------
// F13 — model-output import screen missed separator variants.
// ---------------------------------------------------------------------

#[test]
fn unsafe_import_screen_covers_all_separator_forms() {
    use klang::repair::{is_unsafe_import_path, splice::has_unsafe_import};
    // Every separator the parser accepts between keyword and path.
    assert!(has_unsafe_import("import \"../../evil.klang\"").is_some());
    assert!(has_unsafe_import("import\t\"../../evil.klang\"").is_some());
    assert!(has_unsafe_import("import\"../../evil.klang\"").is_some());
    assert!(has_unsafe_import("   import   \"/abs.klang\"  ").is_some());
    assert!(has_unsafe_import("import \"~/x.klang\"").is_some());
    // Benign lines never flag.
    assert!(has_unsafe_import("import \"lib.klang\"").is_none());
    assert!(has_unsafe_import("import \"./sub/mod.klang\"").is_none());
    assert!(has_unsafe_import("fn main() -> i32 { return 0 }").is_none());
    assert!(has_unsafe_import("important \"../../x.klang\"").is_none());
    // The shared predicate matches the CLI loader's rule exactly. Note it
    // is deliberately substring-based: `a..b.klang` is rejected too
    // (conservative over-blocking, same as the loader — the two can never
    // disagree on what gets followed).
    for bad in ["/abs.klang", "../../e.klang", "a/../../e.klang", "~/x.klang", "a\0b.klang", "a..b.klang"] {
        assert!(is_unsafe_import_path(bad), "{bad}");
    }
    for good in ["lib.klang", "./a.klang", "sub/dir/m.klang"] {
        assert!(!is_unsafe_import_path(good), "{good}");
    }
}

#[test]
fn model_response_unsafe_import_fails_the_attempt_loudly() {
    // End to end through the driver: a model answer smuggling a hostile
    // import is a failed attempt with a clear note, never a splice.
    use klang::repair::{
        MockBackend, RepairCliOverrides, RepairConfig, RepairFileConfig, run_repair,
    };
    let src = "fn main() -> i32 { return nope }";
    let cfg = RepairConfig::resolve(
        &RepairCliOverrides {
            max_iters: Some(1),
            ..Default::default()
        },
        &RepairFileConfig::default(),
    );
    let backend = MockBackend::new(vec![
        "import \"../../evil.klang\"\nfn main() -> i32 { return 1 }\n".into(),
    ]);
    let outcome = run_repair(src, "t.klang", &cfg, &backend, false);
    assert!(!outcome.success);
    assert!(
        outcome.attempts.iter().any(|a| a.note.contains("rejected unsafe import")),
        "{:?}",
        outcome.attempts.iter().map(|a| &a.note).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------
// Phase 4: the direct-model connector is gone (OpenAiCurlBackend,
// endpoint/key config, response parsing all removed). Its F14/F15
// tests went with it: there is no HTTP client left to probe, no
// secrets to argv-leak, no endpoint response to parse. What remains
// pinned: the repair loop treats backend text as untrusted (F13
// above), and the log carries prompts/code only (below).
// ---------------------------------------------------------------------

// ---------------------------------------------------------------------
// F16 — raw control bytes in file-derived errors reached the terminal.
// ---------------------------------------------------------------------

#[test]
fn sanitize_for_terminal_escapes_controls_only() {
    use klang::diagnostics::sanitize_for_terminal;
    assert_eq!(sanitize_for_terminal("lib.klang"), "lib.klang");
    assert_eq!(sanitize_for_terminal("\u{1b}[2Jx"), "\\u{1b}[2Jx");
    assert_eq!(sanitize_for_terminal("a\nb\tc"), "a\\u{a}b\\u{9}c");
    assert_eq!(sanitize_for_terminal("\u{7f}"), "\\u{7f}");
    // Non-ASCII printables pass through untouched.
    assert_eq!(sanitize_for_terminal("é — vals"), "é — vals");
}

// ---------------------------------------------------------------------
// F17 — block-comment braces fooled the repair scope scanner.
// ---------------------------------------------------------------------

#[test]
fn block_comment_braces_do_not_shift_function_spans() {
    use klang::repair::scope::function_spans;
    let src = "fn a() -> i32 {\n    /* } { still comment } */\n    return 1\n}\n\nfn bad() -> i32 {\n    return x\n}\n";
    let spans = function_spans(src);
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert_eq!(spans[0].0, "a");
    assert_eq!(spans[1].0, "bad");
    assert!(spans[0].2 <= spans[1].1, "{spans:?}");
    assert_eq!(src[spans[0].1..spans[0].2].trim_end().chars().last(), Some('}'));
}

#[test]
fn splicer_never_panics_on_hostile_unicode() {
    // Adversarial model-shaped text through every text-level repair
    // entry point: multibyte names/strings/comments, unbalanced braces,
    // unterminated strings. Verdict may be an error, never a panic.
    use klang::repair::{check_candidate, scope::function_spans};
    use klang::repair::splice::extract_functions;
    let cases = vec![
        "fn \u{e9}() -> i32 { return \"\u{1f600}\" }".to_string(),
        "fn a() -> i32 { /* unterminated".to_string(),
        "fn a() -> i32 { let s = \"unterminated }".to_string(),
        "fn a() -> i32 { return 1 } fn \u{1f600}() -> i32 { return 2 }".to_string(),
        "\u{1f600}\u{1f600}\u{1f600}".to_string(),
        String::new(),
    ];
    for src in &cases {
        let spans = function_spans(src);
        for (n, s, e) in &spans {
            assert!(s <= e && *e <= src.len(), "{n} {s} {e}");
            assert!(src.is_char_boundary(*s) && src.is_char_boundary(*e));
        }
        let _ = extract_functions(src);
        let _ = check_candidate(src);
    }
    // A well-formed unicode function still splices.
    let orig = "fn a() -> i32 {\n    return 1\n}\n";
    let raw = "fn a() -> i32 {\n    return \"é\"\n}\n";
    let out =
        klang::repair::splice::splice_functions(orig, &["a".to_string()], raw).expect("splices");
    assert!(out.contains('é'), "{out}");
}

// ---------------------------------------------------------------------
// The repair log carries prompts and code only — never credentials.
// (Phase 4 removed endpoint/key config entirely, so there is nothing
//  secret to leak; this pins that the log contains no such fields.)
// ---------------------------------------------------------------------

#[test]
fn repair_log_contains_no_credential_fields() {
    use klang::repair::{
        MockBackend, RepairConfig, run_repair, write_log,
    };
    let cfg = RepairConfig {
        max_iters: 1,
        scope: klang::repair::Scope::Function,
    };
    let src = "fn main() -> i32 { return nope }";
    let backend = MockBackend::new(vec!["fn main() -> i32 { return 1 }".into()]);
    let outcome = run_repair(src, "t.klang", &cfg, &backend, false);
    assert!(outcome.success);
    let dir = std::env::temp_dir().join(format!("klang-logsec-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join(".klang-repair-log.json");
    write_log(
        log.to_str().unwrap(),
        "t.klang",
        outcome.success,
        &outcome.attempts,
        &outcome.final_diagnostics,
    )
    .expect("log writes");
    let text = std::fs::read_to_string(&log).unwrap();
    // Prompts, responses, and diagnostics ARE logged (transparency);
    // the log schema has no credential-shaped fields at all.
    assert!(text.contains("prompt_system"), "{text}");
    assert!(text.contains("raw_response"), "{text}");
    assert!(!text.contains("api_key"), "{text}");
    assert!(!text.contains("endpoint"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// Pinned non-findings (confirmed during the audit).
// ---------------------------------------------------------------------

#[test]
fn import_cycle_terminates_at_check_level() {
    // check_candidate (what repair re-runs) never touches the filesystem:
    // imports are inert text there, so cycles cannot hang repair.
    let src = "import \"b.klang\"\nfn main() -> i32 { return nope }";
    let r = klang::repair::check_candidate(src);
    assert!(r.diagnostics.iter().any(|d| d.code == "E-UNDEFINED"));
}

#[test]
fn unknown_builtin_is_diagnostic_not_panic() {
    let mut p = Parser::new("fn main() -> i32 { return frobnicate(1) }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-UNDEFINED"));
}

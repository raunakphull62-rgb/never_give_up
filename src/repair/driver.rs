//! Bounded repair driver: one `contracts::repair_loop` over prompt ->
//! model -> splice -> re-check (PRD 5.2). Deterministic outside the
//! model call itself; every attempt logged.

use crate::diagnostics::Diagnostic;
use crate::hir::TypedHIR;
use crate::parser::Parser;

use super::backend::ModelBackend;
use super::config::RepairConfig;
use super::prompt::{build_system_prompt, build_user_prompt};
use super::scope::{RepairScope, function_spans, plan_scope};
use super::splice::{check_candidate, has_unsafe_import, splice_functions};

/// One logged attempt.
#[derive(Debug, Clone)]
pub struct AttemptLog {
    pub iter: u32,
    pub scope: String,
    pub target: String,
    pub diagnostics_in: Vec<Diagnostic>,
    pub prompt_system: String,
    pub prompt_user: String,
    pub raw_response: String,
    pub diagnostics_out: Vec<Diagnostic>,
    pub note: String,
}

/// Final outcome of a repair run.
#[derive(Debug, Clone)]
pub struct RepairOutcome {
    pub success: bool,
    pub source: String,
    pub attempts: Vec<AttemptLog>,
    pub final_diagnostics: Vec<Diagnostic>,
    /// `repair_loop` iterations actually consumed.
    pub iters_used: u32,
}

/// Short history line for the next prompt (what was tried + result).
fn history_line(raw: &str, diags: &[Diagnostic]) -> String {
    let codes: Vec<String> = diags.iter().map(|d| d.code.clone()).collect();
    let snippet: String = raw.chars().take(240).collect();
    if codes.is_empty() {
        format!("response `{}` checked clean", snippet.replace('\n', " "))
    } else {
        format!("response `{}` still failed with {}", snippet.replace('\n', " "), codes.join(","))
    }
}

/// Source text of the target functions (or whole file for file scope).
fn scope_source(src: &str, scope: &RepairScope) -> (String, String) {
    match scope {
        RepairScope::File => ("whole file".to_string(), src.to_string()),
        RepairScope::Functions(fs) => {
            let spans = function_spans(src);
            let mut parts = Vec::new();
            for f in fs {
                if let Some((_, s, e)) = spans.iter().find(|(n, _, _)| n == f) {
                    parts.push(src[*s..*e].to_string());
                }
            }
            (RepairScope::Functions(fs.clone()).desc(), parts.join("\n\n"))
        }
    }
}

/// Run the bounded repair loop over in-memory `src`.
/// `file_label` names the file in diagnostics parsing (usually the path).
/// Pure orchestration: deterministic given diagnostics + model outputs.
pub fn run_repair(
    src: &str,
    file_label: &str,
    cfg: &RepairConfig,
    backend: &dyn ModelBackend,
    dry_run: bool,
) -> RepairOutcome {
    // Initial check: parse + full TypedHIR::check.
    let initial_diags = initial_check(src, file_label);
    if initial_diags.is_empty() {
        return RepairOutcome {
            success: true,
            source: src.to_string(),
            attempts: vec![],
            final_diagnostics: vec![],
            iters_used: 0,
        };
    }
    if dry_run {
        // No model calls: show the planned first prompt.
        let scope = plan_scope(&initial_diags, src, cfg.scope == super::config::Scope::File);
        let (desc, ssrc) = scope_source(src, &scope);
        let sys = build_system_prompt();
        let usr = build_user_prompt(&desc, &ssrc, &initial_diags, &[], Some(src));
        let scope_name = match &scope {
            RepairScope::File => "file",
            RepairScope::Functions(_) => "function",
        };
        return RepairOutcome {
            success: false,
            source: src.to_string(),
            attempts: vec![AttemptLog {
                iter: 0,
                scope: scope_name.to_string(),
                target: desc,
                diagnostics_in: initial_diags.clone(),
                prompt_system: sys,
                prompt_user: usr,
                raw_response: String::new(),
                diagnostics_out: initial_diags.clone(),
                note: "dry-run: no model call made".to_string(),
            }],
            final_diagnostics: initial_diags,
            iters_used: 0,
        };
    }
    let max = cfg.max_iters.clamp(1, RepairConfig::HARD_MAX_ITERS);
    let system = build_system_prompt();
    // Mutable loop state driven through contracts::repair_loop.
    let mut current = src.to_string();
    let mut current_diags = initial_diags;
    let mut history: Vec<String> = Vec::new();
    let mut seen_responses: Vec<String> = Vec::new();
    let mut logs: Vec<AttemptLog> = Vec::new();
    let mut file_scope = cfg.scope == super::config::Scope::File;
    let mut succeeded = false;
    let mut iters_used = 0u32;
    let res: Result<(), Vec<Diagnostic>> =
        crate::contracts::repair_loop(max, |i| {
            iters_used = i + 1;
            let scope = plan_scope(&current_diags, &current, file_scope);
            // Repeat detection: if the last response repeats verbatim,
            // escalate to whole-file scope for the next attempt.
            let (desc, ssrc) = scope_source(&current, &scope);
            let user = build_user_prompt(&desc, &ssrc, &current_diags, &history, Some(&current));
            let scope_name = match &scope {
                RepairScope::File => "file",
                RepairScope::Functions(_) => "function",
            };
            let raw = match backend.complete(&system, &user) {
                Ok(r) => r,
                Err(e) => {
                    logs.push(AttemptLog {
                        iter: i,
                        scope: scope_name.to_string(),
                        target: desc,
                        diagnostics_in: current_diags.clone(),
                        prompt_system: system.clone(),
                        prompt_user: user,
                        raw_response: String::new(),
                        diagnostics_out: current_diags.clone(),
                        note: format!("model call failed: {e}"),
                    });
                    history.push(format!("model call failed: {e}"));
                    return Err(current_diags.clone());
                }
            };
            let norm = raw.trim().to_string();
            if seen_responses.contains(&norm) {
                // Escalate: retry same iteration budget at file scope.
                file_scope = true;
                logs.push(AttemptLog {
                    iter: i,
                    scope: scope_name.to_string(),
                    target: desc,
                    diagnostics_in: current_diags.clone(),
                    prompt_system: system.clone(),
                    prompt_user: user,
                    raw_response: raw.clone(),
                    diagnostics_out: current_diags.clone(),
                    note: "repeat response detected: escalating to whole-file scope next attempt".to_string(),
                });
                history.push(history_line(&raw, &current_diags));
                return Err(current_diags.clone());
            }
            seen_responses.push(norm);
            // Safety: unsafe imports in model output are a failed attempt.
            if let Some(bad) = has_unsafe_import(&raw) {
                logs.push(AttemptLog {
                    iter: i,
                    scope: scope_name.to_string(),
                    target: desc,
                    diagnostics_in: current_diags.clone(),
                    prompt_system: system.clone(),
                    prompt_user: user,
                    raw_response: raw.clone(),
                    diagnostics_out: current_diags.clone(),
                    note: format!("rejected unsafe import `{bad}`"),
                });
                history.push(format!("response rejected: unsafe import `{bad}`"));
                return Err(current_diags.clone());
            }
            // Splice + full re-check.
            let candidate = match &scope {
                RepairScope::File => raw.clone(),
                RepairScope::Functions(fs) => match splice_functions(&current, fs, &raw) {
                    Ok(s) => s,
                    Err(e) => {
                        logs.push(AttemptLog {
                            iter: i,
                            scope: scope_name.to_string(),
                            target: desc,
                            diagnostics_in: current_diags.clone(),
                            prompt_system: system.clone(),
                            prompt_user: user,
                            raw_response: raw.clone(),
                            diagnostics_out: current_diags.clone(),
                            note: format!("splice failed: {e}"),
                        });
                        history.push(format!("splice failed: {e}; response was: {}", raw.chars().take(200).collect::<String>().replace('\n', " ")));
                        return Err(current_diags.clone());
                    }
                },
            };
            let checked = check_candidate(&candidate);
            logs.push(AttemptLog {
                iter: i,
                scope: scope_name.to_string(),
                target: desc,
                diagnostics_in: current_diags.clone(),
                prompt_system: system.clone(),
                prompt_user: user,
                raw_response: raw.clone(),
                diagnostics_out: checked.diagnostics.clone(),
                note: if checked.diagnostics.is_empty() { "clean".to_string() } else { format!("{} diagnostic(s) remain", checked.diagnostics.len()) },
            });
            if checked.diagnostics.is_empty() {
                current = checked.source;
                current_diags = vec![];
                succeeded = true;
                return Ok(());
            }
            history.push(history_line(&raw, &checked.diagnostics));
            current = checked.source;
            current_diags = checked.diagnostics.clone();
            Err(checked.diagnostics)
        });
    let success = res.is_ok() && succeeded;
    RepairOutcome { success, source: current, attempts: logs, final_diagnostics: current_diags, iters_used }
}

/// Initial parse + check with the file label applied.
fn initial_check(src: &str, file_label: &str) -> Vec<Diagnostic> {
    let mut p = Parser::new_with_file(src, file_label);
    let prog = match p.parse_program() {
        Ok(prog) => prog,
        Err(d) => return vec![d],
    };
    match TypedHIR::check(prog) {
        Ok(_) => vec![],
        Err(ds) => ds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repair::{MockBackend, RepairCliOverrides, RepairFileConfig};

    fn cfg(scope: &str) -> RepairConfig {
        RepairConfig::resolve_with(
            &RepairCliOverrides { endpoint: Some("http://x/v1".into()), scope: super::super::config::Scope::parse(scope), ..Default::default() },
            &RepairFileConfig::default(),
            &[],
        )
        .expect("cfg")
    }

    #[test]
    fn clean_is_noop() {
        let b = MockBackend::new(vec![]);
        let o = run_repair("fn main() -> i32 { return 42 }\n", "t.klang", &cfg("file"), &b, false);
        assert!(o.success);
        assert_eq!(o.iters_used, 0);
        assert_eq!(b.calls(), 0);
    }

    #[test]
    fn dry_run_makes_no_calls() {
        let b = MockBackend::new(vec!["fn main() -> i32 { return 1 }\n".into()]);
        let o = run_repair("fn main() -> i32 { return x }\n", "t.klang", &cfg("file"), &b, true);
        assert!(!o.success);
        assert_eq!(b.calls(), 0);
        assert!(o.attempts[0].prompt_user.contains("E-UNDEFINED"), "{}", o.attempts[0].prompt_user);
    }

    #[test]
    fn file_scope_converges() {
        let b = MockBackend::new(vec!["fn main() -> i32 { return 42 }\n".into()]);
        let o = run_repair("fn main() -> i32 { return x }\n", "t.klang", &cfg("file"), &b, false);
        assert!(o.success, "{:?}", o.final_diagnostics);
        assert_eq!(o.iters_used, 1);
        assert!(o.source.contains("return 42"));
    }

    #[test]
    fn function_scope_preserves_untouched() {
        let src = "fn a() -> i32 {\n    return 1\n}\n\nfn bad() -> i32 {\n    return x\n}\n";
        let b = MockBackend::new(vec!["fn bad() -> i32 {\n    return 2\n}\n".into()]);
        let o = run_repair(src, "t.klang", &cfg("function"), &b, false);
        assert!(o.success, "{:?}", o.final_diagnostics);
        assert!(o.source.contains("return 1"), "{}", o.source);
        assert!(o.source.contains("return 2"), "{}", o.source);
        assert!(!o.source.contains("return x"), "{}", o.source);
    }

    #[test]
    fn exhaustion_returns_final_diags() {
        let b = MockBackend::new(vec!["fn main() -> i32 { return x }\n".into()]);
        let mut flags = RepairCliOverrides { endpoint: Some("http://x/v1".into()), max_iters: Some(2), ..Default::default() };
        flags.scope = super::super::config::Scope::parse("file");
        let c = RepairConfig::resolve_with(&flags, &RepairFileConfig::default(), &[]).expect("cfg");
        let o = run_repair("fn main() -> i32 { return x }\n", "t.klang", &c, &b, false);
        assert!(!o.success);
        assert_eq!(o.iters_used, 2);
        assert!(!o.final_diagnostics.is_empty());
    }
}

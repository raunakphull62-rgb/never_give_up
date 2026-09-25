//! Repair prompts: fixed system prompt + per-attempt user prompt (PRD 5.4).
//!
//! Diagnostics render as-is from `Diagnostic::to_json()` plus the relevant
//! source slice. `FixIt` labels ride along as extra hints (PRD open Q:
//! try both, compare convergence).

use crate::diagnostics::Diagnostic;

/// Fixed system prompt: Klang grammar + effect rules the model must obey.
/// Kept short and normative; SPEC.md remains the full reference.
pub fn build_system_prompt() -> String {
    "You repair Klang source so it passes the Klang compiler.\n\
     Rules: output ONLY corrected Klang source for the requested scope, no prose, no markdown fences.\n\
     Klang: `fn name(params) -> type [throws] [async] [cancel] { ... }`; types i32/f64/str/bool;\n\
     `return a + b`; `let x = expr`; `print(x)`; `if/else`, `while`, `for i in a..b`, `for x in iterable`.\n\
     Effects: calling a `throws` function requires `throws` on the caller (E-EFFECT-MISMATCH).\n\
     Structured concurrency: `spawn` only inside `task_group` (E-SPAWN-OUTSIDE-GROUP);\n\
     every `let h = spawn ...` must be `await`ed before its group ends (E-TASK-CANCEL).\n\
     Calls must match arity (E-ARITY); every variable must be a param or prior `let` (E-UNDEFINED);\n\
     binary/unary ops, calls, returns, conditions are type-checked (E-TYPE).\n\
     Unknown (map lookup/dynamic index) never emits E-TYPE. Do not invent new functions, imports, or language features.".to_string()
}

/// One diagnostic rendered for the prompt: JSON + FixIt hint labels.
pub fn render_diagnostic(d: &Diagnostic) -> String {
    let mut s = d.to_json();
    let hints: Vec<String> = crate::contracts::FixIt::for_diagnostic(d)
        .iter()
        .map(|f| f.label.clone())
        .collect();
    if !hints.is_empty() {
        s.push_str(&format!("\nSuggested fixes: {}", hints.join("; ")));
    }
    s
}

/// Per-attempt user prompt: scope source + diagnostics JSON + history.
/// `history` holds short summaries of prior failed attempts (prevents the
/// model repeating the same mistake, PRD 5.2.i).
pub fn build_user_prompt(
    scope_desc: &str,
    scope_source: &str,
    diagnostics: &[Diagnostic],
    history: &[String],
    full_source: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Repair scope: {scope_desc}\n\n"));
    out.push_str("Current source for this scope:\n");
    out.push_str(scope_source);
    if !scope_source.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\nCompiler diagnostics (JSON):\n");
    for d in diagnostics {
        out.push_str(&render_diagnostic(d));
        out.push('\n');
    }
    if let Some(full) = full_source {
        out.push_str("\nFull file for context (do NOT rewrite untouched parts):\n");
        out.push_str(full);
        if !full.ends_with('\n') {
            out.push('\n');
        }
    }
    if !history.is_empty() {
        out.push_str("\nPrior failed attempts (do NOT repeat these):\n");
        for (i, h) in history.iter().enumerate() {
            out.push_str(&format!("Attempt {}: {}\n", i + 1, h));
        }
    }
    out.push_str("\nRespond with ONLY the corrected Klang source for the repair scope.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_contains_json_and_hints() {
        let d = Diagnostic::error(
            "E-TASK-CANCEL",
            "leak",
            "f",
            0,
            1,
            "c",
            &["await task before leaving scope"],
            "r",
        );
        let u = build_user_prompt(
            "function bad",
            "fn bad() -> i32 { return 1 }",
            &[d],
            &[],
            None,
        );
        assert!(u.contains("E-TASK-CANCEL"), "{u}");
        assert!(u.contains("await task before leaving scope"), "{u}");
        assert!(u.contains("ONLY the corrected"), "{u}");
        let s = build_system_prompt();
        assert!(s.contains("task_group"), "{s}");
        assert!(s.contains("E-TYPE"), "{s}");
    }
}

//! Attempt logging: `.klang-repair-log.json` writer (PRD G4 / NFR).
//!
//! Every attempt: diagnostics in, prompt sent, raw model response,
//! diagnostics out. Full transparency, nothing silently rewritten.

use super::driver::AttemptLog;
use crate::diagnostics::Diagnostic;

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Render diagnostics as their existing `to_json()` objects, joined.
fn diags_json(diags: &[Diagnostic]) -> String {
    diags.iter().map(|d| d.to_json()).collect::<Vec<_>>().join(", ")
}

fn attempt_json(a: &AttemptLog) -> String {
    format!(
        "{{\"iter\": {}, \"scope\": \"{}\", \"target\": \"{}\", \"diagnostics_in\": [{}], \"prompt_system\": \"{}\", \"prompt_user\": \"{}\", \"raw_response\": \"{}\", \"diagnostics_out\": [{}], \"note\": \"{}\"}}",
        a.iter,
        esc(&a.scope),
        esc(&a.target),
        diags_json(&a.diagnostics_in),
        esc(&a.prompt_system),
        esc(&a.prompt_user),
        esc(&a.raw_response),
        diags_json(&a.diagnostics_out),
        esc(&a.note),
    )
}

/// Serialize the full attempt history (what `main.rs` writes to disk).
 pub fn render_log(
    target: &str,
    success: bool,
    attempts: &[AttemptLog],
    final_diags: &[Diagnostic],
) -> String {
    let items: Vec<String> = attempts.iter().map(attempt_json).collect();
    format!(
        "{{\"tool\": \"klang repair\", \"target\": \"{}\", \"success\": {}, \"attempts\": [{}], \"final_diagnostics\": [{}]}}",
        esc(target),
        success,
        items.join(", "),
        diags_json(final_diags),
    )
}

/// Write the log file. Returns the path written.
 pub fn write_log(
    path: &str,
    target: &str,
    success: bool,
    attempts: &[AttemptLog],
    final_diags: &[Diagnostic],
) -> Result<String, String> {
    let text = render_log(target, success, attempts, final_diags);
    std::fs::write(path, text).map_err(|e| format!("cannot write log {path}: {e}"))?;
    Ok(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_renders_attempts() {
        let d = Diagnostic::error("E-TYPE", "m", "f", 0, 1, "c", &["fix"], "r");
        let a = AttemptLog {
            iter: 0,
            scope: "file".into(),
            target: "a.klang".into(),
            diagnostics_in: vec![d.clone()],
            prompt_system: "sys".into(),
            prompt_user: "usr".into(),
            raw_response: "fn main() -> i32 { return 1 }".into(),
            diagnostics_out: vec![],
            note: "ok".into(),
        };
        let s = render_log("a.klang", true, &[a], &[]);
        assert!(s.contains("klang repair"));
        assert!(s.contains("E-TYPE"));
        assert!(s.contains("\"success\": true"), "{s}");
    }
}

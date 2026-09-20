//! Stage 6 (part 2): IDE protocol rendering (tower-lsp plugs in here).
//!
//! Converts structured `Diagnostic`s to LSP-compatible JSON without new deps.

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

/// Render one diagnostic as an LSP `publishDiagnostics` entry.
pub fn diagnostic_to_lsp(d: &Diagnostic) -> String {
    format!(
        "{{\"code\":\"{}\",\"severity\":1,\"message\":\"{}\",\"range\":{{\"start\":{{\"line\":0,\"character\":{}}},\"end\":{{\"line\":0,\"character\":{}}},\"source\":\"klang\"}}}}",
        esc(&d.code),
        esc(&d.message),
        d.primary_span.start,
        d.primary_span.end
    )
}

//! Klang stdin input boundary (STDLIB-IO-1).
//!
//! Flat builtins `read_line` / `parse_int` / `parse_float` (see
//! `hir::is_builtin` and `runtime::exec_builtin`). Flat names are
//! deliberate, matching the existing `read_file` / `int` / `float`
//! convention.
//!
//! All I/O goes through the free functions here so the interpreter and
//! unit tests share one behavior mapping. Pure helpers
//! ([`strip_line`], [`parse_int_str`], [`parse_float_str`]) carry the
//! semantics; only [`read_line`] touches the process.
//!
//! Equivalence note: `int(s)` already converts strings (plus ints and
//! floats) with a generic `E-RUNTIME` on failure and `i64` range, while
//! `parse_int(s)` is strict decimal (`-` only, digits only, `i32` range)
//! with the dedicated `E-PARSE-INT` diagnostic below. `float(s)` and
//! `parse_float(s)` relate the same way (`E-RUNTIME` vs `E-PARSE-FLOAT`).

use std::io::BufRead;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use crate::diagnostics::Diagnostic;

/// Over-long offending text is redacted past this many characters so a
/// hostile multi-megabyte line never lands verbatim in a diagnostic.
pub const MAX_SHOWN_INPUT_CHARS: usize = 40;

/// Set while the MCP server owns process stdin (`main::run_mcp_mode`).
/// Programs executed via `mcp::tool_run` then see EOF (`""`) instead of
/// stealing JSON-RPC bytes off the server's stdin stream.
static MCP_OWNS_STDIN: AtomicBool = AtomicBool::new(false);

/// Serializes whole-line reads so concurrent spawned tasks each receive
/// exactly one complete line.
static STDIN_LOCK: Mutex<()> = Mutex::new(());

/// Mark process stdin as owned by the MCP server. After this call,
/// [`read_line`] returns `""` without touching stdin.
pub fn reserve_stdin_for_mcp() {
    MCP_OWNS_STDIN.store(true, Ordering::SeqCst);
}

/// True when [`reserve_stdin_for_mcp`] has been called (test hook for the
/// MCP-protection guarantee).
pub fn stdin_reserved_for_mcp() -> bool {
    MCP_OWNS_STDIN.load(Ordering::SeqCst)
}

/// Strip one trailing line ending: a trailing `\n`, plus a preceding
/// `\r` for CRLF input. `"a\r\n"` → `"a"`, `"a\n"` → `"a"`, `"\n"` → `""`.
pub fn strip_line(line: &str) -> String {
    let line = line.strip_suffix('\n').unwrap_or(line);
    line.strip_suffix('\r').unwrap_or(line).to_string()
}

/// Render offending input for diagnostics, redacting past
/// [`MAX_SHOWN_INPUT_CHARS`] characters.
pub fn redact_input(text: &str) -> String {
    if text.chars().count() <= MAX_SHOWN_INPUT_CHARS {
        return format!("\"{text}\"");
    }
    let head: String = text.chars().take(MAX_SHOWN_INPUT_CHARS).collect();
    let rest = text.chars().count() - MAX_SHOWN_INPUT_CHARS;
    format!("\"{head}...\" [{rest} chars redacted]")
}

/// `E-PARSE-INT`: `text` is not strict decimal or is outside `i32` range.
fn parse_int_err(text: &str, why: &str) -> Diagnostic {
    let shown = redact_input(text);
    Diagnostic::error(
        "E-PARSE-INT",
        &format!("parse_int({shown}) failed: {why}"),
        "runtime",
        0,
        0,
        "parse_int needs an optional leading `-` followed by decimal digits within i32 range",
        &["check the input line before parsing"],
        "input/parse-int",
    )
}

/// `E-PARSE-FLOAT`: `text` is not a finite `f64` decimal.
fn parse_float_err(text: &str) -> Diagnostic {
    let shown = redact_input(text);
    Diagnostic::error(
        "E-PARSE-FLOAT",
        &format!("parse_float({shown}) failed: not a finite decimal number"),
        "runtime",
        0,
        0,
        "parse_float needs a finite decimal number",
        &["check the input line before parsing"],
        "input/parse-float",
    )
}

/// Read one line from process stdin, without the trailing newline.
/// Returns `""` at EOF (indistinguishable from an empty line, by design).
/// When stdin is reserved for the MCP server, returns `""` immediately.
pub fn read_line() -> String {
    if stdin_reserved_for_mcp() {
        return String::new();
    }
    let _guard = STDIN_LOCK.lock().unwrap();
    let stdin = std::io::stdin();
    let mut line = String::new();
    match stdin.lock().read_line(&mut line) {
        Ok(0) => String::new(),
        Ok(_) => strip_line(&line),
        Err(_) => String::new(),
    }
}

/// Strict decimal integer: optional single leading `-`, then one or more
/// ASCII digits, within `i32` range. No whitespace, no `+`, no prefixes.
pub fn parse_int_str(text: &str) -> Result<i64, Diagnostic> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let ok = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    if !ok {
        return Err(parse_int_err(text, "not a decimal integer with optional leading `-`"));
    }
    match text.parse::<i32>() {
        Ok(v) => Ok(i64::from(v)),
        Err(_) => Err(parse_int_err(text, "out of i32 range")),
    }
}

/// Finite `f64` decimal (`f64` grammar: fraction and scientific notation
/// accepted). No surrounding whitespace; infinities and NaN are errors.
pub fn parse_float_str(text: &str) -> Result<f64, Diagnostic> {
    if text.is_empty() || text.bytes().any(|b| b.is_ascii_whitespace()) {
        return Err(parse_float_err(text));
    }
    match text.parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => Err(parse_float_err(text)),
    }
}

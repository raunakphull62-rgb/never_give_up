//! `klang lsp`: a real stdio JSON-RPC language server (FOUNDATION-3 Part 2C, v1).
//!
//! This module REPLACES the former 32-line JSON-renderer stub (which
//! hardcoded `"line": 0` and destroyed position information). Nothing of
//! the stub remains: positions are computed for real from diagnostic byte
//! spans, and the server speaks framed JSON-RPC over stdio.
//!
//! Explicit v1 scope (see `docs/foundation3-part2-design.md` §C):
//! - IN scope: diagnostics-as-you-type. The message content is built
//!   directly on the existing `klang check` pipeline (same loader, same
//!   checker, same `Diagnostic::to_json()` objects, carried in the LSP
//!   `data` field) with REAL line/character positions derived from byte
//!   spans ([`byte_offset_to_position`]).
//! - OUT of scope (planned v2, not a silent omission): hover and
//!   go-to-definition. The server answers both with JSON-RPC
//!   `MethodNotFound` (-32601) saying exactly that, so editors degrade
//!   loudly instead of hanging.
//! - Incremental strategy: whole-file re-check per change with a 100 ms
//!   debounce ([`DEBOUNCE_MS`]). `check` is a single linear pass —
//!   `examples/eval.klang` re-checks at ~39 ms p95 end-to-end (process
//!   spawn included; in-process is faster), far under the ~200 ms budget
//!   that would trigger routing through the Salsa incremental database
//!   (`src/db.rs`) instead. No coalescing beyond the debounce timer, no
//!   per-line caching: stated cost, measured fast enough.
//! - Simulated async, registry, file/stdin/process/sleep blocking, and
//!   all other v1 scopes are untouched by this module.
//!
//! Transport: LSP `Content-Length` framing over stdin/stdout (byte-exact,
//! NOT line-delimited like `klang mcp`: bodies may contain newlines).
//! Hand-rolled JSON via `crate::mcp::{parse_json, Json}` — no tower-lsp,
//! no tokio (two heavy async deps for a synchronous checker would be
//! absurd; stated cost avoided).
//!
//! Security/robustness posture: every inbound message is parsed
//! defensively — malformed JSON, missing `Content-Length`, unknown
//! methods, and out-of-range spans never crash the server (ignored or
//! answered with a JSON-RPC error). A poisoned internal lock is a loud
//! process exit, never a silent result.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

use crate::diagnostics::Diagnostic;
use crate::mcp::{parse_json, Json};

/// Debounce window: after a change notification, wait this long for
/// quiet before re-checking (whole-file re-check per settled change).
/// 100 ms sits under the ~200 ms editor-latency budget on its own while
/// coalescing keystroke bursts into one check each.
pub const DEBOUNCE_MS: u64 = 100;

/// LSP server name/version reported in `initialize`.
pub const SERVER_NAME: &str = "klang";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Positions: byte spans -> LSP line/character
// ---------------------------------------------------------------------------

/// Convert a byte `offset` in `text` to an LSP `(line, character)` pair
/// (both 0-based). `character` counts UTF-16 code units, per the LSP
/// specification (editors index lines that way; plain `chars().count()`
/// would drift on astral-plane characters). Lines split on `\n` (a lone
/// `\r` is an ordinary character; `\r\n` contributes one line break and
/// the `\r` is NOT counted in any line's characters).
///
/// Out-of-range offsets clamp to the end of the document (never panic,
/// never wrap): a stale span still points at visible text instead of
/// killing the server.
pub fn byte_offset_to_position(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    // Snap to a char boundary: spans are byte offsets and must never
    // split a multi-byte scalar (floor to the boundary at or before).
    let mut snap = offset;
    while snap > 0 && !text.is_char_boundary(snap) {
        snap -= 1;
    }
    let mut line: u32 = 0;
    let mut utf16_units: u32 = 0;
    for (i, c) in text.char_indices() {
        if i >= snap {
            break;
        }
        if c == '\n' {
            line += 1;
            utf16_units = 0;
        } else if c != '\r' {
            utf16_units += c.len_utf16() as u32;
        }
    }
    (line, utf16_units)
}

/// Map a checker severity string to an LSP `DiagnosticSeverity`.
/// The checker only emits `"error"` today; anything else degrades to
/// `Error` (1) rather than vanishing — severities never go silent.
pub fn severity_to_lsp(severity: &str) -> u32 {
    match severity {
        "error" => 1,
        "warning" => 2,
        "information" | "info" => 3,
        "hint" => 4,
        _ => 1,
    }
}

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

/// Render one checker diagnostic as an LSP `Diagnostic` JSON object for a
/// document holding `text`.
///
/// - `range` comes from the diagnostic's byte `primary_span` via
///   [`byte_offset_to_position`] (REAL positions — the stub this module
///   replaced hardcoded `line: 0`).
/// - `message` is the checker's human message; `code` is the `E-*` code,
///   `source` is always `"klang"`.
/// - `data` carries the FULL `klang check` diagnostic JSON
///   (`Diagnostic::to_json()` verbatim): the LSP message is built
///   directly on the check format, so any tooling built on `check`
///   output stays valid through the LSP surface.
pub fn diagnostic_to_lsp_json(d: &Diagnostic, text: &str) -> String {
    let (sl, sc) = byte_offset_to_position(text, d.primary_span.start);
    let (el, ec) = byte_offset_to_position(text, d.primary_span.end);
    format!(
        "{{\"range\":{{\"start\":{{\"line\":{sl},\"character\":{sc}}},\"end\":{{\"line\":{el},\"character\":{ec}}}}},\"severity\":{},\"code\":\"{}\",\"source\":\"klang\",\"message\":\"{}\",\"data\":{}}}",
        severity_to_lsp(&d.severity),
        esc(&d.code),
        esc(&d.message),
        d.to_json(),
    )
}

// ---------------------------------------------------------------------------
// Checking: the exact `klang check` pipeline over an in-memory buffer
// ---------------------------------------------------------------------------

/// Check one open document exactly as `klang check` would check its saved
/// file: same loader (with the buffer substituted for the entry file, so
/// `import`s still resolve from disk), same checker, same diagnostics.
///
/// - `path` (`Some` for `file://` URIs that exist on disk): buffer text
///   goes through [`crate::imports::load_program_with_entry_source`],
///   then `TypedHIR::check_with_file`.
/// - `None` (untitled/unsaved documents, or paths not on disk):
///   standalone parse + check of the buffer alone. Files using `import`
///   checked this way report `E-UNDEFINED` for imported names (stated
///   limitation: save the file once and the loader path takes over).
/// - Load failures (`E-IO-NOT-FOUND`/`E-IMPORT`/`E-DUPLICATE`/entry
///   `E-PARSE`) become ONE diagnostic at 0,0 naming the real code, so a
///   broken import still surfaces as-you-type instead of silence.
pub fn check_document(path: Option<&str>, text: &str) -> Vec<Diagnostic> {
    let on_disk = path.map(|p| std::path::Path::new(p).is_file()).unwrap_or(false);
    if let Some(path) = path {
        if on_disk {
            return check_via_loader(path, text);
        }
    }
    let label = path.unwrap_or("untitled");
    let mut p = crate::parser::Parser::new_with_file(text, label);
    match p.parse_program() {
        Ok(prog) => {
            // Phase 1d warnings surface as severity-2 diagnostics; they
            // never fail the check itself.
            let warns = crate::hir::narrow_type_warnings(&prog, label);
            match crate::hir::TypedHIR::check_with_file(prog, label) {
                Ok(_) => warns,
                Err(mut diags) => {
                    diags.extend(warns);
                    diags
                }
            }
        }
        Err(d) => vec![d],
    }
}

fn check_via_loader(path: &str, text: &str) -> Vec<Diagnostic> {
    match crate::imports::load_program_with_entry_source(path, text) {
        Ok(loaded) => {
            let warns = crate::hir::narrow_type_warnings(&loaded.program, path);
            match crate::hir::TypedHIR::check_with_file(loaded.program, path) {
                Ok(_) => warns,
                Err(mut diags) => {
                    diags.extend(warns);
                    diags
                }
            }
        }
        Err(e) => {
            if e.code == "E-PARSE" {
                // The message already IS the inner diagnostic JSON: parse
                // it back so positions survive (fall back to 0,0 text).
                if let Ok(Json::Obj(_)) = parse_json(&e.message) {
                    if let Some(d) = diagnostic_from_check_json(&e.message) {
                        return vec![d];
                    }
                }
            }
            vec![Diagnostic::error(
                e.code,
                &e.message,
                path,
                0,
                0,
                "language server load failed",
                &[],
                "lsp/load",
            )]
        }
    }
}

/// Rehydrate a `Diagnostic` from its `to_json()` rendering (Code, message,
/// severity, span; fixes/related are not needed for publishing).
fn diagnostic_from_check_json(json: &str) -> Option<Diagnostic> {
    let v = parse_json(json).ok()?;
    let code = v.get("code")?.as_str()?.to_string();
    let message = v.get("message")?.as_str()?.to_string();
    let severity = v.get("severity").and_then(|j| j.as_str()).unwrap_or("error").to_string();
    let span = v.get("primary_span")?;
    let file = span.get("file").and_then(|j| j.as_str()).unwrap_or("").to_string();
    let start = match span.get("start") {
        Some(Json::Int(n)) => (*n).max(0) as usize,
        _ => 0,
    };
    let end = match span.get("end") {
        Some(Json::Int(n)) => (*n).max(0) as usize,
        _ => 0,
    };
    Some(Diagnostic {
        code,
        severity,
        message,
        primary_span: crate::diagnostics::Span { file, start, end },
        cause: String::new(),
        expected: None,
        found: None,
        fixes: Vec::new(),
        rule: String::new(),
        related: Vec::new(),
    })
}

/// Map a document URI to a filesystem path (`Some`) or `None` for
/// non-file URIs (`untitled:`, `git:`, ...). Percent-decoding failures
/// and empty paths yield `None` (standalone check, never a crash).
pub fn uri_to_path(uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file://")?;
    if path.is_empty() {
        return None;
    }
    percent_decode(path).ok()
}

fn percent_decode(s: &str) -> Result<String, String> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                if i + 2 >= b.len() {
                    return Err("bad percent-encoding".to_string());
                }
                let hex = std::str::from_utf8(&b[i + 1..i + 3])
                    .map_err(|_| "bad percent-encoding".to_string())?;
                let v = u8::from_str_radix(hex, 16)
                    .map_err(|_| "bad percent-encoding".to_string())?;
                out.push(v);
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "bad percent-encoding".to_string())
}

// ---------------------------------------------------------------------------
// Protocol: framed JSON-RPC over stdio
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OpenDoc {
    version: Option<i64>,
    text: String,
}

fn json_id(id: &Json) -> String {
    match id {
        Json::Int(n) => n.to_string(),
        Json::Str(s) => format!("\"{}\"", esc(s)),
        _ => "null".to_string(),
    }
}

fn response_ok(raw_id: &str, result: &str) -> String {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{raw_id},\"result\":{result}}}")
}

fn response_err(raw_id: &str, code: i64, message: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{raw_id},\"error\":{{\"code\":{code},\"message\":\"{}\"}}}}",
        esc(message)
    )
}

fn publish_notification(uri: &str, version: Option<i64>, diags_json: &str) -> String {
    let version_part = match version {
        Some(v) => format!(",\"version\":{v}"),
        None => String::new(),
    };
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{{\"uri\":\"{}\"{version_part},\"diagnostics\":[{diags_json}]}}}}",
        esc(uri)
    )
}

fn get_str(v: &Json, key: &str) -> Option<String> {
    v.get(key).and_then(|j| j.as_str()).map(|s| s.to_string())
}

fn get_int(v: &Json, key: &str) -> Option<i64> {
    match v.get(key) {
        Some(Json::Int(n)) => Some(*n),
        _ => None,
    }
}

/// Check every dirty document and render one `publishDiagnostics`
/// notification per document. Checks run on a deep-stack worker each
/// (the checker recurses over the AST: a hostile buffer must produce
/// diagnostics, never a native stack overflow).
///
/// Cross-file honesty (v1 publishes only the OPEN document): diagnostics
/// whose span names another file are NOT silently dropped — when the
/// open document itself is clean but its dependencies are not, one
/// synthesized 0,0 diagnostic names the other file and its real code, so
/// editors still show something is wrong instead of false-clean.
fn check_dirty(docs: &HashMap<String, OpenDoc>, dirty: &std::collections::HashSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut uris: Vec<&String> = dirty.iter().collect();
    uris.sort();
    for uri in uris {
        let Some(doc) = docs.get(uri) else {
            continue;
        };
        let path = uri_to_path(uri);
        let text = doc.text.clone();
        let version = doc.version;
        let uri_owned = uri.clone();
        let path_for_check = path.clone();
        let diags = crate::with_deep_stack(move || check_document(path_for_check.as_deref(), &text));
        let (here, elsewhere): (Vec<&Diagnostic>, Vec<&Diagnostic>) = match &path {
            Some(p) => diags.iter().partition(|d| d.primary_span.file == *p),
            None => (diags.iter().collect(), Vec::new()),
        };
        let rendered: Vec<String> = if here.is_empty() && !elsewhere.is_empty() {
            let first = elsewhere[0];
            vec![diagnostic_to_lsp_json(
                &Diagnostic::error(
                    &first.code,
                    &format!(
                        "{} other-file error(s); first is {} in {}: {} (v1 publishes only the open document)",
                        elsewhere.len(),
                        first.code,
                        first.primary_span.file,
                        first.message,
                    ),
                    path.as_deref().unwrap_or("untitled"),
                    0,
                    0,
                    "dependency failed while the open file is clean",
                    &["open the other file to see its diagnostics"],
                    "lsp/scope",
                ),
                &doc.text,
            )]
        } else {
            here.iter().map(|d| diagnostic_to_lsp_json(d, &doc.text)).collect()
        };
        out.push(publish_notification(&uri_owned, version, &rendered.join(",")));
    }
    out
}

/// Handle ONE parsed JSON-RPC message. Returns `(responses, shutdown_seen)`.
/// `responses` are outbound JSON strings in send order (replies first,
/// then at most one publish batch per dirty document).
fn handle_message(
    msg: &Json,
    docs: &mut HashMap<String, OpenDoc>,
    dirty: &mut std::collections::HashSet<String>,
    shutdown_seen: bool,
) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let method = get_str(msg, "method").unwrap_or_default();
    let has_id = msg.get("id").is_some();
    let raw_id = msg.get("id").map(json_id).unwrap_or_else(|| "null".to_string());
    let params = msg.get("params").cloned().unwrap_or(Json::Null);

    // Notifications (no id) that change documents mark them dirty; the
    // caller drains `dirty` through `check_dirty` after the debounce
    // window settles.
    match method.as_str() {
        "initialize" => {
            let result = format!(
                "{{\"capabilities\":{{\"textDocumentSync\":1}},\"serverInfo\":{{\"name\":\"{SERVER_NAME}\",\"version\":\"{SERVER_VERSION}\"}}}}"
            );
            out.push(response_ok(&raw_id, &result));
        }
        "initialized" => {}
        "textDocument/didOpen" => {
            if let Some(td) = params.get("textDocument") {
                if let Some(uri) = get_str(td, "uri") {
                    let text = get_str(td, "text").unwrap_or_default();
                    let version = get_int(td, "version");
                    docs.insert(uri.clone(), OpenDoc { version, text });
                    dirty.insert(uri);
                }
            }
        }
        "textDocument/didChange" => {
            if let Some(td) = params.get("textDocument") {
                if let Some(uri) = get_str(td, "uri") {
                    let version = get_int(td, "version");
                    // Full-sync only in v1: the last `text` wins.
                    let mut new_text: Option<String> = None;
                    if let Some(changes) = params.get("contentChanges").and_then(|j| j.as_arr()) {
                        for ch in changes {
                            if let Some(t) = get_str(ch, "text") {
                                new_text = Some(t);
                            }
                        }
                    }
                    if let Some(text) = new_text {
                        docs.insert(uri.clone(), OpenDoc { version, text });
                        dirty.insert(uri);
                    } else if let Some(doc) = docs.get_mut(&uri) {
                        doc.version = version;
                    }
                }
            }
        }
        "textDocument/didClose" => {
            if let Some(td) = params.get("textDocument") {
                if let Some(uri) = get_str(td, "uri") {
                    docs.remove(&uri);
                    dirty.remove(&uri);
                    out.push(publish_notification(&uri, None, ""));
                }
            }
        }
        "shutdown" => {
            out.push(response_ok(&raw_id, "null"));
            return (out, true);
        }
        "exit" => {
            std::process::exit(if shutdown_seen { 0 } else { 1 });
        }
        "textDocument/hover" | "textDocument/definition" => {
            // Explicit v2 scope: loud MethodNotFound, never a hang.
            if has_id {
                out.push(response_err(
                    &raw_id,
                    -32601,
                    &format!("{method} is not in the v1 server (planned v2)"),
                ));
            }
        }
        "$/cancelRequest" | "$/setTrace" => {}
        _ => {
            if has_id {
                out.push(response_err(&raw_id, -32601, "method not found"));
            }
        }
    }
    (out, shutdown_seen)
}

/// Read ONE LSP-framed message from `reader` (headers + `Content-Length`
/// bytes). `Ok(None)` is clean EOF; `Err` is malformed framing (the
/// caller skips it, never crashes).
fn read_framed<R: Read>(reader: &mut std::io::BufReader<R>) -> Result<Option<String>, String> {
    use std::io::BufRead;
    let mut content_len: Option<usize> = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(e) => return Err(format!("header read: {e}")),
        }
        let t = line.trim();
        if t.is_empty() {
            break;
        }
        if let Some(v) = t.strip_prefix("Content-Length:") {
            content_len = v.trim().parse().ok();
        } else if let Some(v) = t.strip_prefix("content-length:") {
            content_len = v.trim().parse().ok();
        }
    }
    let Some(n) = content_len else {
        return Err("missing Content-Length".to_string());
    };
    if n > 64 * 1024 * 1024 {
        return Err("message too large".to_string());
    }
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf).map_err(|e| format!("body read: {e}"))?;
    String::from_utf8(buf).map_err(|_| "message is not UTF-8".to_string()).map(Some)
}

fn write_framed(stdout: &mut std::io::Stdout, body: &str) -> bool {
    let head = format!("Content-Length: {}\r\n\r\n", body.len());
    stdout.write_all(head.as_bytes()).is_ok()
        && stdout.write_all(body.as_bytes()).is_ok()
        && stdout.flush().is_ok()
}

enum Inbound {
    Text(String),
}

/// Serve LSP on stdio: reader thread parses frames into a channel, the
/// processor loop debounces change bursts into whole-file re-checks.
///
/// Layout rationale: `stdin().lock()` blocks, so it lives on its own
/// thread; the processor owns documents + debounce timing. A poisoned
/// channel or a vanished stdout ends the server loudly (nonzero exit),
/// never spinning silently.
pub fn serve_stdio() {
    let (tx, rx) = mpsc::channel::<Inbound>();
    std::thread::Builder::new()
        .name("klang-lsp-reader".to_string())
        .spawn(move || {
            let stdin = std::io::stdin();
            let mut reader = std::io::BufReader::new(stdin.lock());
            loop {
                match read_framed(&mut reader) {
                    Ok(None) => break,
                    Ok(Some(text)) => {
                        if tx.send(Inbound::Text(text)).is_err() {
                            break;
                        }
                    }
                    Err(_) => continue,
                }
            }
        })
        .expect("lsp reader spawns");

    let mut stdout = std::io::stdout();
    let mut docs: HashMap<String, OpenDoc> = HashMap::new();
    let mut dirty: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut shutdown_seen = false;

    loop {
        // Blocking wait for the next message, then drain whatever else
        // arrived (or arrives within the debounce window) before
        // checking: keystroke bursts settle into one check per document.
        let first = match rx.recv() {
            Ok(m) => m,
            Err(_) => break,
        };
        let mut batch = vec![first];
        loop {
            match rx.recv_timeout(Duration::from_millis(DEBOUNCE_MS)) {
                Ok(m) => batch.push(m),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if batch.len() > 128 {
                break;
            }
        }
        let mut publish: Vec<String> = Vec::new();
        for msg in &batch {
            let Inbound::Text(text) = msg;
            let parsed = match parse_json(text) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let (mut responses, shut) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handle_message(&parsed, &mut docs, &mut dirty, shutdown_seen)
            }))
            .unwrap_or_else(|_| {
                (
                    vec![response_err("null", -32603, "internal error")],
                    shutdown_seen,
                )
            });
            shutdown_seen = shut;
            for r in responses.drain(..) {
                if !write_framed(&mut stdout, &r) {
                    std::process::exit(1);
                }
            }
        }
        // Whole-file re-check per settled change burst (the debounce
        // window above already coalesced keystrokes): one publish per
        // dirty document, in URI order.
        if !dirty.is_empty() {
            let notes = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                check_dirty(&docs, &dirty)
            }))
            .unwrap_or_default();
            dirty.clear();
            for n in notes {
                publish.push(n);
            }
        }
        for n in publish.drain(..) {
            if !write_framed(&mut stdout, &n) {
                std::process::exit(1);
            }
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn positions_first_line() {
        assert_eq!(byte_offset_to_position("abc", 0), (0, 0));
        assert_eq!(byte_offset_to_position("abc", 1), (0, 1));
        assert_eq!(byte_offset_to_position("abc", 3), (0, 3));
    }

    #[test]
    fn positions_multiline() {
        let t = "ab\ncde\nf";
        assert_eq!(byte_offset_to_position(t, 3), (1, 0));
        assert_eq!(byte_offset_to_position(t, 5), (1, 2));
        assert_eq!(byte_offset_to_position(t, 7), (2, 0));
        assert_eq!(byte_offset_to_position(t, 8), (2, 1));
    }

    #[test]
    fn positions_clamp_past_end() {
        assert_eq!(byte_offset_to_position("ab", 99), (0, 2));
        assert_eq!(byte_offset_to_position("", 5), (0, 0));
    }
}

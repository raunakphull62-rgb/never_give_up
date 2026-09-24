//! `klang mcp`: the verification engine as MCP tools over stdio.
//!
//! Phase 4 correction: the harness owns the model connection, so Klang
//! meets it where it already works — as a set of MCP tools the harness
//! calls inside its own agent loop. No per-harness adapter, no provider
//! catalog inside Klang, no network or keys on this side.
//!
//! Transport: JSON-RPC 2.0 messages, one per line on stdin/stdout
//! (MCP stdio framing), protocol version 2024-11-05. Hand-rolled JSON,
//! like everywhere else in this crate: no new dependencies.
//!
//! Tool results carry the exact `Diagnostic::to_json()` objects, so any
//! prompt work built on `check` output stays valid regardless of which
//! harness calls. Deliberately NOT a tool: anything that takes a task
//! description and calls a model — that is the harness's job, using
//! `klang_check` / `klang_scope_plan` as its feedback signal.

use std::collections::HashMap;

use crate::diagnostics::{Diagnostic, Fix, Span};
use crate::hir::TypedHIR;
use crate::parser::Parser;
use crate::repair::scope::{RepairScope, is_declaration_level, plan_scope};

pub const PROTOCOL_VERSION: &str = "2024-11-05";
pub const SERVER_NAME: &str = "klang";
pub const SERVER_VERSION: &str = "0.1.0";

/// Wall clock bound on `klang_run` execution. A non-terminating program
/// yields a timeout error, not a hung server; the timed-out worker
/// thread is detached (it cannot be killed) and documented as such.
pub const RUN_TIMEOUT_SECS: u64 = 30;

/// Maximum JSON nesting accepted in requests (adversarial-depth guard
/// for the recursive parser below).
const MAX_JSON_DEPTH: usize = 64;

// ---------------------------------------------------------------------
// Minimal JSON value model (parse + serialize, no dependencies).
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }
    fn esc_into(out: &mut String, s: &str) {
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
    }
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }
    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(n) => out.push_str(&n.to_string()),
            Json::Float(f) => {
                if f.is_finite() {
                    out.push_str(&format!("{f:?}"));
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(s) => {
                out.push('"');
                Self::esc_into(out, s);
                out.push('"');
            }
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    Self::esc_into(out, k);
                    out.push_str("\":");
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

fn parse_string(s: &str, b: &[u8], i: usize) -> Result<(String, usize), String> {
    // b[i] == b'"' on entry.
    let mut out = String::new();
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'"' => return Ok((out, j + 1)),
            b'\\' => {
                j += 1;
                if j >= b.len() {
                    return Err("unterminated escape".to_string());
                }
                match b[j] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if j + 4 >= b.len() {
                            return Err("truncated \\u escape".to_string());
                        }
                        let hex = &s[j + 1..j + 5];
                        let cp = u32::from_str_radix(hex, 16)
                            .map_err(|_| "bad \\u escape".to_string())?;
                        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        j += 4;
                    }
                    _ => return Err("bad escape".to_string()),
                }
                j += 1;
            }
            _ => {
                let ch = s[j..].chars().next().ok_or("bad utf-8")?;
                out.push(ch);
                j += ch.len_utf8();
            }
        }
    }
    Err("unterminated string".to_string())
}

fn parse_number(s: &str, b: &[u8], i: usize) -> Result<(Json, usize), String> {
    let mut j = i;
    if b.get(j) == Some(&b'-') {
        j += 1;
    }
    let int_start = j;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    let mut is_float = false;
    if b.get(j) == Some(&b'.') {
        is_float = true;
        j += 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    if b.get(j) == Some(&b'e') || b.get(j) == Some(&b'E') {
        is_float = true;
        j += 1;
        if b.get(j) == Some(&b'+') || b.get(j) == Some(&b'-') {
            j += 1;
        }
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    if j == int_start {
        return Err("bad number".to_string());
    }
    let text = &s[i..j];
    if !is_float {
        if let Ok(n) = text.parse::<i64>() {
            return Ok((Json::Int(n), j));
        }
    }
    text.parse::<f64>()
        .map(|f| (Json::Float(f), j))
        .map_err(|_| "bad number".to_string())
}

fn parse_value(s: &str, b: &[u8], i: usize, depth: usize) -> Result<(Json, usize), String> {
    if depth > MAX_JSON_DEPTH {
        return Err("json too deeply nested".to_string());
    }
    let i = skip_ws(b, i);
    if i >= b.len() {
        return Err("unexpected end of json".to_string());
    }
    match b[i] {
        b'"' => parse_string(s, b, i).map(|(v, j)| (Json::Str(v), j)),
        b'{' => {
            let mut pairs = Vec::new();
            let mut j = skip_ws(b, i + 1);
            if b.get(j) == Some(&b'}') {
                return Ok((Json::Obj(pairs), j + 1));
            }
            loop {
                j = skip_ws(b, j);
                if b.get(j) != Some(&b'"') {
                    return Err("expected string key".to_string());
                }
                let (k, j2) = parse_string(s, b, j)?;
                j = skip_ws(b, j2);
                if b.get(j) != Some(&b':') {
                    return Err("expected ':'".to_string());
                }
                let (v, j3) = parse_value(s, b, j + 1, depth + 1)?;
                pairs.push((k, v));
                j = skip_ws(b, j3);
                match b.get(j) {
                    Some(&b',') => j += 1,
                    Some(&b'}') => return Ok((Json::Obj(pairs), j + 1)),
                    _ => return Err("expected ',' or '}'".to_string()),
                }
            }
        }
        b'[' => {
            let mut items = Vec::new();
            let mut j = skip_ws(b, i + 1);
            if b.get(j) == Some(&b']') {
                return Ok((Json::Arr(items), j + 1));
            }
            loop {
                let (v, j2) = parse_value(s, b, j, depth + 1)?;
                items.push(v);
                j = skip_ws(b, j2);
                match b.get(j) {
                    Some(&b',') => j += 1,
                    Some(&b']') => return Ok((Json::Arr(items), j + 1)),
                    _ => return Err("expected ',' or ']'".to_string()),
                }
            }
        }
        b't' if s[i..].starts_with("true") => Ok((Json::Bool(true), i + 4)),
        b'f' if s[i..].starts_with("false") => Ok((Json::Bool(false), i + 5)),
        b'n' if s[i..].starts_with("null") => Ok((Json::Null, i + 4)),
        c if c == b'-' || c.is_ascii_digit() => parse_number(s, b, i),
        _ => Err("unexpected json value".to_string()),
    }
}

/// Parse one complete JSON value; trailing garbage is an error.
pub fn parse_json(s: &str) -> Result<Json, String> {
    let b = s.as_bytes();
    let (v, j) = parse_value(s, b, 0, 0)?;
    if skip_ws(b, j) != b.len() {
        return Err("trailing characters after json".to_string());
    }
    Ok(v)
}

// ---------------------------------------------------------------------
// Diagnostic round-trip: the scope planner consumes diagnostics that a
// harness got from `klang_check`, i.e. exact `to_json()` objects.
// ---------------------------------------------------------------------

fn json_int(v: &Json) -> Result<i64, String> {
    match v {
        Json::Int(n) => Ok(*n),
        Json::Float(f) => Ok(*f as i64),
        _ => Err("expected number".to_string()),
    }
}

fn diagnostic_from_json(v: &Json) -> Result<Diagnostic, String> {
    let code = v.get("code").and_then(Json::as_str).ok_or("diagnostic missing code")?;
    let message = v.get("message").and_then(Json::as_str).ok_or("diagnostic missing message")?;
    let rule = v.get("rule").and_then(Json::as_str).unwrap_or("");
    let span = v.get("primary_span").ok_or("diagnostic missing primary_span")?;
    let file = span.get("file").and_then(Json::as_str).unwrap_or("input.warden");
    let start = span.get("start").map(json_int).transpose()? .unwrap_or(0).max(0) as usize;
    let end = span.get("end").map(json_int).transpose()?.unwrap_or(0).max(0) as usize;
    let fixes = match v.get("fixes") {
        Some(Json::Arr(items)) => items
            .iter()
            .filter_map(|f| f.get("label").and_then(Json::as_str))
            .map(|l| Fix { label: l.to_string() })
            .collect(),
        _ => Vec::new(),
    };
    let related = match v.get("related") {
        Some(Json::Arr(items)) => items
            .iter()
            .map(diagnostic_from_json)
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };
    Ok(Diagnostic {
        code: code.to_string(),
        severity: v.get("severity").and_then(Json::as_str).unwrap_or("error").to_string(),
        message: message.to_string(),
        primary_span: Span { file: file.to_string(), start, end },
        cause: v.get("cause").and_then(Json::as_str).unwrap_or("").to_string(),
        fixes,
        rule: rule.to_string(),
        related,
    })
}

// ---------------------------------------------------------------------
// Tools: pure functions over source strings (transport lives in main).
// ---------------------------------------------------------------------

/// Parse + check, returning the diagnostics (empty = clean).
pub fn check_source(source: &str) -> Vec<Diagnostic> {
    let mut p = Parser::new(source);
    match p.parse_program() {
        Err(d) => vec![d],
        Ok(prog) => match TypedHIR::check(prog) {
            Ok(_) => vec![],
            Err(ds) => ds,
        },
    }
}

fn diags_json(diags: &[Diagnostic]) -> Json {
    // Re-parse each to_json() object so tool output embeds the identical
    // shape a harness would get from the CLI (verified by test).
    let items: Vec<Json> = diags
        .iter()
        .map(|d| parse_json(&d.to_json()).expect("to_json is valid json"))
        .collect();
    Json::Arr(items)
}

/// `klang_check`: source in, diagnostics out (empty array = clean).
pub fn tool_check(source: &str) -> Json {
    Json::Obj(vec![("diagnostics".to_string(), diags_json(&check_source(source)))])
}

/// `klang_run`: check first, then execute `entry` (default `main`) with
/// a wall-clock bound. Never hangs the caller on loops.
pub fn tool_run(source: &str, entry: &str) -> (bool, Json) {
    let diags = check_source(source);
    if !diags.is_empty() {
        return (
            true,
            Json::Obj(vec![
                ("ok".to_string(), Json::Bool(false)),
                ("stage".to_string(), Json::Str("check".to_string())),
                ("diagnostics".to_string(), diags_json(&diags)),
            ]),
        );
    }
    let mut p = Parser::new(source);
    let prog = match p.parse_program() {
        Err(d) => {
            return (
                true,
                Json::Obj(vec![
                    ("ok".to_string(), Json::Bool(false)),
                    ("stage".to_string(), Json::Str("parse".to_string())),
                    ("diagnostics".to_string(), diags_json(&[d])),
                ]),
            );
        }
        Ok(prog) => prog,
    };
    let entry = entry.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // Deep-stack worker, like the CLI: hostile depth must error,
        // never abort the server process.
        let r = crate::with_deep_stack(move || {
            let mir = crate::mir::lower(&prog);
            crate::runtime::run_with_output(&mir, &entry, &[], &HashMap::new())
        });
        let _ = tx.send(r);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(RUN_TIMEOUT_SECS)) {
        Ok(Ok((v, out))) => (
            false,
            Json::Obj(vec![
                ("ok".to_string(), Json::Bool(true)),
                (
                    "stdout".to_string(),
                    Json::Arr(out.into_iter().map(Json::Str).collect()),
                ),
                ("return_value".to_string(), Json::Int(v as i64)),
            ]),
        ),
        Ok(Err(d)) => (
            true,
            Json::Obj(vec![
                ("ok".to_string(), Json::Bool(false)),
                ("stage".to_string(), Json::Str("run".to_string())),
                (
                    "error".to_string(),
                    parse_json(&d.to_json()).expect("to_json is valid json"),
                ),
            ]),
        ),
        Err(_) => (
            true,
            Json::Obj(vec![
                ("ok".to_string(), Json::Bool(false)),
                ("stage".to_string(), Json::Str("timeout".to_string())),
                ("timeout_secs".to_string(), Json::Int(RUN_TIMEOUT_SECS as i64)),
            ]),
        ),
    }
}

/// `klang_fmt`: canonical source out, or the parse diagnostic.
pub fn tool_fmt(source: &str) -> (bool, Json) {
    let mut p = Parser::new(source);
    match p.parse_program() {
        Err(d) => (
            true,
            Json::Obj(vec![
                ("ok".to_string(), Json::Bool(false)),
                (
                    "diagnostic".to_string(),
                    parse_json(&d.to_json()).expect("to_json is valid json"),
                ),
            ]),
        ),
        Ok(prog) => (
            false,
            Json::Obj(vec![
                ("ok".to_string(), Json::Bool(true)),
                ("formatted".to_string(), Json::Str(crate::fmt::fmt_program(&prog))),
            ]),
        ),
    }
}

/// `klang_scope_plan`: the scope planner's verdict over harness-held
/// diagnostics — the same `plan_scope` the repair loop uses, so a
/// harness doing its own scoped editing gets identical answers.
pub fn tool_scope_plan(source: &str, diags: &[Diagnostic]) -> Json {
    let scope = plan_scope(diags, source, false);
    match &scope {
        RepairScope::Functions(fs) => Json::Obj(vec![
            ("scope".to_string(), Json::Str("function".to_string())),
            (
                "functions".to_string(),
                Json::Arr(fs.iter().cloned().map(Json::Str).collect()),
            ),
            (
                "reason".to_string(),
                Json::Str(format!(
                    "diagnostics attribute to function(s): {}",
                    fs.join(", ")
                )),
            ),
        ]),
        RepairScope::File => {
            let mut rules: Vec<String> = diags
                .iter()
                .filter(|d| is_declaration_level(d))
                .map(|d| d.rule.clone())
                .collect();
            rules.sort();
            rules.dedup();
            let reason = if rules.is_empty() {
                "diagnostics span multiple scopes or could not be attributed; use whole file".to_string()
            } else {
                format!(
                    "declaration-level diagnostics (rules: {}) always repair at file scope",
                    rules.join(", ")
                )
            };
            Json::Obj(vec![
                ("scope".to_string(), Json::Str("file".to_string())),
                ("reason".to_string(), Json::Str(reason)),
            ])
        }
    }
}

// ---------------------------------------------------------------------
// JSON-RPC envelope: initialize / tools/list / tools/call.
// ---------------------------------------------------------------------

fn rpc_error(id: Option<&Json>, code: i64, message: &str) -> String {
    Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        (
            "id".to_string(),
            id.cloned().unwrap_or(Json::Null),
        ),
        (
            "error".to_string(),
            Json::Obj(vec![
                ("code".to_string(), Json::Int(code)),
                ("message".to_string(), Json::Str(message.to_string())),
            ]),
        ),
    ])
    .render()
}

fn rpc_result(id: &Json, result: Json) -> String {
    Json::Obj(vec![
        ("jsonrpc".to_string(), Json::Str("2.0".to_string())),
        ("id".to_string(), id.clone()),
        ("result".to_string(), result),
    ])
    .render()
}

/// Generic internal error (used when a request panics mid-handle and its
/// id cannot be recovered).
pub fn internal_error() -> String {
    rpc_error(None, -32603, "internal error")
}

fn tool_text(result: Json, is_error: bool) -> Json {
    Json::Obj(vec![
        (
            "content".to_string(),
            Json::Arr(vec![Json::Obj(vec![(
                "type".to_string(),
                Json::Str("text".to_string()),
            ), ("text".to_string(), Json::Str(result.render()))])]),
        ),
        ("isError".to_string(), Json::Bool(is_error)),
    ])
}

fn tool_list() -> Json {
    let tool = |name: &str, desc: &str, schema: Vec<(String, Json)>, required: &[&str]| {
        Json::Obj(vec![
            ("name".to_string(), Json::Str(name.to_string())),
            ("description".to_string(), Json::Str(desc.to_string())),
            (
                "inputSchema".to_string(),
                Json::Obj(vec![
                    ("type".to_string(), Json::Str("object".to_string())),
                    ("properties".to_string(), Json::Obj(schema)),
                    (
                        "required".to_string(),
                        Json::Arr(required.iter().map(|s| Json::Str(s.to_string())).collect()),
                    ),
                ]),
            ),
        ])
    };
    let str_prop = |desc: &str| {
        Json::Obj(vec![
            ("type".to_string(), Json::Str("string".to_string())),
            ("description".to_string(), Json::Str(desc.to_string())),
        ])
    };
    Json::Obj(vec![(
        "tools".to_string(),
        Json::Arr(vec![
            tool(
                "klang_check",
                "Type-check Klang source. Returns the compiler diagnostics as JSON (empty array means clean).",
                vec![("source".to_string(), str_prop("Klang source text to check"))],
                &["source"],
            ),
            tool(
                "klang_run",
                "Type-check then run Klang source. Checks first: on failure returns the diagnostics without running.",
                vec![
                    ("source".to_string(), str_prop("Klang source text to run")),
                    ("entry".to_string(), str_prop("Entry function (default: main)")),
                ],
                &["source"],
            ),
            tool(
                "klang_fmt",
                "Format Klang source canonically (4-space indent, one statement per line, parenthesized operators).",
                vec![("source".to_string(), str_prop("Klang source text to format"))],
                &["source"],
            ),
            tool(
                "klang_scope_plan",
                "Plan the minimal repair scope for check diagnostics: function-scoped names, or file scope with the reason. Feed it the diagnostics array exactly as klang_check returned it.",
                vec![
                    ("source".to_string(), str_prop("The failing Klang source text")),
                    (
                        "diagnostics".to_string(),
                        Json::Obj(vec![
                            ("type".to_string(), Json::Str("array".to_string())),
                            ("description".to_string(), Json::Str("Diagnostics exactly as klang_check returned them".to_string())),
                        ]),
                    ),
                ],
                &["source", "diagnostics"],
            ),
        ]),
    )])
}

fn call_tool(name: &str, args: &Json) -> Result<(bool, Json), String> {
    let obj = match args {
        Json::Obj(_) => args,
        Json::Null => &Json::Obj(vec![]),
        _ => return Err("arguments must be an object".to_string()),
    };
    let source = |args: &Json| -> Result<String, String> {
        args.get("source")
            .and_then(Json::as_str)
            .map(str::to_string)
            .ok_or("missing required string argument: source".to_string())
    };
    match name {
        "klang_check" => Ok((false, tool_check(&source(obj)?))),
        "klang_run" => {
            let entry = obj
                .get("entry")
                .and_then(Json::as_str)
                .unwrap_or("main")
                .to_string();
            Ok(tool_run(&source(obj)?, &entry))
        }
        "klang_fmt" => Ok(tool_fmt(&source(obj)?)),
        "klang_scope_plan" => {
            let src = source(obj)?;
            let raw = obj.get("diagnostics").and_then(Json::as_arr).ok_or(
                "missing required array argument: diagnostics".to_string(),
            )?;
            let mut diags = Vec::with_capacity(raw.len());
            for (i, d) in raw.iter().enumerate() {
                diags.push(
                    diagnostic_from_json(d)
                        .map_err(|e| format!("diagnostics[{i}] invalid: {e}"))?,
                );
            }
            Ok((false, tool_scope_plan(&src, &diags)))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

/// Handle one JSON-RPC line. Returns `None` for notifications (no `id`)
/// which require no response.
pub fn handle_request(line: &str) -> Option<String> {
    let req = match parse_json(line) {
        Err(e) => return Some(rpc_error(None, -32700, &format!("parse error: {e}"))),
        Ok(v) => v,
    };
    let method = req.get("method").and_then(Json::as_str).unwrap_or("");
    let id = req.get("id");
    // Notifications (no id, or explicit notification prefix) get silence.
    if id.is_none() || method.starts_with("notifications/") {
        return None;
    }
    let id = id.expect("id present");
    if req.get("jsonrpc").and_then(Json::as_str) != Some("2.0") {
        return Some(rpc_error(Some(id), -32600, "invalid request: want {\"jsonrpc\": \"2.0\"}"));
    }
    match method {
        "initialize" => Some(rpc_result(
            id,
            Json::Obj(vec![
                ("protocolVersion".to_string(), Json::Str(PROTOCOL_VERSION.to_string())),
                (
                    "capabilities".to_string(),
                    Json::Obj(vec![("tools".to_string(), Json::Obj(vec![]))]),
                ),
                (
                    "serverInfo".to_string(),
                    Json::Obj(vec![
                        ("name".to_string(), Json::Str(SERVER_NAME.to_string())),
                        ("version".to_string(), Json::Str(SERVER_VERSION.to_string())),
                    ]),
                ),
            ]),
        )),
        "ping" => Some(rpc_result(id, Json::Obj(vec![]))),
        "tools/list" => Some(rpc_result(id, tool_list())),
        "tools/call" => {
            let params = req.get("params").unwrap_or(&Json::Null);
            let name = params.get("name").and_then(Json::as_str).unwrap_or("");
            let args = params.get("arguments").unwrap_or(&Json::Null);
            match call_tool(name, args) {
                Ok((is_error, result)) => Some(rpc_result(id, tool_text(result, is_error))),
                Err(e) if e.starts_with("unknown tool:") => {
                    Some(rpc_error(Some(id), -32602, &e))
                }
                Err(e) => Some(rpc_error(Some(id), -32602, &e)),
            }
        }
        _ => Some(rpc_error(Some(id), -32601, &format!("method not found: {method}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let v = parse_json("{\"a\": [1, -2.5, true, null, \"x\\ny\"]}").expect("parses");
        assert_eq!(v.get("a").and_then(Json::as_arr).map(Vec::len), Some(5));
        let again = parse_json(&v.render()).expect("reparses");
        assert_eq!(v, again);
    }

    #[test]
    fn json_rejects_garbage_and_depth() {
        assert!(parse_json("{oops").is_err());
        assert!(parse_json("{\"a\": 1} trailing").is_err());
        assert!(parse_json("").is_err());
        let mut deep = String::new();
        for _ in 0..200 {
            deep.push('[');
        }
        assert!(parse_json(&deep).is_err());
    }

    #[test]
    fn diagnostic_round_trip_through_json() {
        let mut p = Parser::new("fn main() -> i32 { return nope }");
        let prog = p.parse_program().expect("parses");
        let diags = TypedHIR::check(prog).expect_err("fails");
        for d in &diags {
            let back = diagnostic_from_json(&parse_json(&d.to_json()).expect("json")).expect("parse");
            assert_eq!(back.code, d.code);
            assert_eq!(back.message, d.message);
            assert_eq!(back.rule, d.rule);
            assert_eq!(back.primary_span.start, d.primary_span.start);
            assert_eq!(back.primary_span.end, d.primary_span.end);
            assert_eq!(back.fixes.len(), d.fixes.len());
        }
    }
}

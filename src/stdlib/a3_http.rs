//! A3 HTTP server and fuller HTTP client (Wave 2 Stage A3).
//!
//! Flat builtins (matching `read_file` / `tcp_listen` convention):
//! `http_serve`, `server_port`, `http_stop`, `http_request`,
//! `http_json`, `http_text`, `http_redirect`, `route_match`,
//! `json_decode`, `json_stringify`.
//!
//! Reuse rule (no second networking path):
//! - Server sockets go through `a1_net` (`tcp_listen`,
//!   `tcp_accept_timeout`, `conn_read_line`, `conn_read`, `conn_write`,
//!   `conn_set_timeout`, `conn_peer`, `conn_close`). No raw
//!   `TcpListener`/`TcpStream` here except via those handles.
//! - Worker pool threads use the same OS-thread shape as `a2_threads`
//!   (8 MiB stack, `Arc` module sharing, per-thread Klang ids) and the
//!   job queue is an `a2_threads` channel carrying `Value::Conn`.
//! - Client TLS reuses the existing `ureq` rustls + WebPki bundle from
//!   `stdlib::http` (no second TLS stack, no native-tls).
//!
//! Null mapping (hosting-platform requirement): `Value::Null`
//! (`is_null`) is JSON `null` everywhere this stage produces or parses
//! JSON. `json_decode("null")` is `Null`, `json_stringify(Null)` is
//! `"null"`, `http_json(status, Null)` has body `"null"`. Booleans have
//! no distinct runtime value (`Bool` erases to `Int` 0/1), so JSON
//! `true`/`false` decode to `1`/`0` and every `Int` encodes as a number
//! (bools do not round-trip; the legacy `json` package markers keep
//! type honesty for package users).
//!
//! Naming deviation (W-SHADOW): the pure-Klang `json` package already
//! defines `fn json_parse(text) -> {ok, value, error}` with `$json`
//! markers. A same-named native would trip `W-SHADOW` and break the
//! package parity self-test (like `hex_encode` in A1 / B1 complements).
//! So the fast native parser is named `json_decode(s) -> value`
//! (throws `E-JSON-PARSE` on invalid input) while `json_stringify`
//! has no package collision and keeps the queue name. The package keeps
//! working unchanged; server handlers should use the natives for the
//! 1 MB-in-1s path.
//!
//! All functions deny with `E-SANDBOX-NETWORK` under
//! `klang run --sandbox` except the pure constructors
//! (`http_json`, `http_text`, `http_redirect`, `route_match`,
//! `json_decode`, `json_stringify`), which are allowed like `is_null`.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use crate::diagnostics::Diagnostic;
use crate::runtime::Value;

// ---------------------------------------------------------------------------
// Limits.
// ---------------------------------------------------------------------------

/// Default server workers.
pub const DEFAULT_WORKERS: usize = 4;
/// Max server workers (options cap).
pub const MAX_WORKERS: usize = 32;
/// Default max request body (1 MB).
pub const DEFAULT_MAX_BODY: usize = 1024 * 1024;
/// Max request body ever accepted (10 MB, like A1 reads).
pub const MAX_BODY_CAP: usize = 10 * 1024 * 1024;
/// Default read timeout for one request (10 s).
pub const DEFAULT_READ_TIMEOUT_MS: u64 = 10_000;
/// Max read timeout (60 s; 0 means blocking).
pub const MAX_READ_TIMEOUT_MS: u64 = 60_000;
/// Default max header block (64 KB).
pub const DEFAULT_MAX_HEADER: usize = 64 * 1024;
/// Max header block ever accepted (1 MB).
pub const MAX_HEADER_CAP: usize = 1024 * 1024;
/// Max JSON output from `json_stringify` (10 MB).
pub const MAX_JSON_OUT: usize = 10 * 1024 * 1024;
/// Max client timeout (120 s; 0 means default 30 s).
pub const MAX_CLIENT_TIMEOUT_MS: i64 = 120_000;
/// Default client timeout when `timeout_ms == 0` (30 s).
pub const DEFAULT_CLIENT_TIMEOUT_MS: u64 = 30_000;
/// Max idle keep-alive timeout (60 s; 0 = never close an idle conn).
pub const MAX_IDLE_TIMEOUT_MS: u64 = 60_000;
/// Default idle keep-alive timeout (5 s).
pub const DEFAULT_IDLE_TIMEOUT_MS: u64 = 5_000;
/// Default cap on requests served over one keep-alive conn (0 = unlimited).
pub const DEFAULT_MAX_REQUESTS_PER_CONN: u64 = 1_000;
/// Max requests-per-conn cap.
pub const MAX_REQUESTS_PER_CONN: u64 = 1_000_000;
/// Accept poll interval (lets `http_stop` land quickly).
pub const ACCEPT_POLL_MS: i64 = 100;

// ---------------------------------------------------------------------------
// Error constructors (every message names the offending value).
// ---------------------------------------------------------------------------

fn http_err(code: &str, message: String, cause: String, rule: &str) -> Diagnostic {
    Diagnostic::error(code, &message, "runtime", 0, 0, &cause, &["check the method, URL, headers, body and timeout, then retry"], rule)
}

fn url_err(op: &str, url: &str, detail: &str) -> Diagnostic {
    let shown = if url.len() > 200 { format!("{}... [{} chars redacted]", &url[..200], url.len() - 200) } else { url.to_string() };
    http_err("E-HTTP-URL", format!("{op}({shown:?}) failed: invalid URL: {detail}"), detail.to_string(), "http/url")
}

fn method_err(op: &str, method: &str) -> Diagnostic {
    http_err("E-HTTP-METHOD", format!("{op}({method:?}) failed: invalid method (want GET, POST, PUT, DELETE, PATCH, HEAD or OPTIONS)"), format!("bad method {method:?}"), "http/method")
}

fn header_err(op: &str, detail: &str) -> Diagnostic {
    http_err("E-HTTP-HEADER", format!("{op} failed: invalid header: {detail}"), detail.to_string(), "http/header")
}

fn timeout_err(op: &str, target: &str, detail: &str) -> Diagnostic {
    http_err("E-HTTP-TIMEOUT", format!("{op}({target}) failed: timed out: {detail}"), detail.to_string(), "http/timeout")
}

fn connect_err(op: &str, target: &str, detail: &str) -> Diagnostic {
    http_err("E-HTTP-CONNECT", format!("{op}({target}) failed: connection failed: {detail}"), detail.to_string(), "http/connect")
}

fn redirect_err(op: &str, url: &str, detail: &str) -> Diagnostic {
    http_err("E-HTTP-REDIRECT", format!("{op}({url:?}) failed: {detail}"), detail.to_string(), "http/redirect")
}

fn options_err(op: &str, detail: &str) -> Diagnostic {
    http_err("E-HTTP-OPTIONS", format!("{op} failed: invalid options: {detail}"), detail.to_string(), "http/options")
}

fn handler_err(op: &str, name: &str, detail: &str) -> Diagnostic {
    http_err("E-HTTP-HANDLER", format!("{op}({name:?}) failed: {detail}"), detail.to_string(), "http/handler")
}

fn closed_err(op: &str) -> Diagnostic {
    http_err("E-HTTP-CLOSED", format!("{op} failed: server is already stopped"), "server handle was stopped".to_string(), "http/closed")
}

fn status_err(op: &str, got: i64) -> Diagnostic {
    http_err("E-HTTP-STATUS", format!("{op}({got}) failed: invalid status {got} (want 100..599)"), format!("status {got} out of range"), "http/status")
}

fn route_err(op: &str, detail: &str) -> Diagnostic {
    http_err("E-HTTP-ROUTE", format!("{op} failed: {detail}"), detail.to_string(), "http/route")
}

fn json_parse_err(op: &str, detail: &str) -> Diagnostic {
    http_err("E-JSON-PARSE", format!("{op} failed: invalid JSON: {detail}"), detail.to_string(), "json/parse")
}

fn json_stringify_err(op: &str, detail: &str) -> Diagnostic {
    http_err("E-JSON-STRINGIFY", format!("{op} failed: {detail}"), detail.to_string(), "json/stringify")
}

fn redact_body(s: &str) -> String {
    if s.len() <= 200 { format!("{s:?}") } else { format!("{:?}... [{} chars redacted]", &s[..200], s.len() - 200) }
}

// ---------------------------------------------------------------------------
// JSON: Json <-> Value with Null <-> null.
// ---------------------------------------------------------------------------

/// Decode one JSON text to a Klang `Value` (`null` -> `Null`,
/// `true`/`false` -> `1`/`0`, numbers -> `Int`/`Float`, strings ->
/// `Str`, arrays -> `Array`, objects -> `Map`). Trailing garbage and
/// over-deep nesting are `E-JSON-PARSE`, never a panic.
pub fn json_decode(s: &str) -> Result<Value, Diagnostic> {
    const OP: &str = "json_decode";
    let j = crate::json::parse_json(s).map_err(|e| json_parse_err(OP, &e))?;
    Ok(json_to_value(&j))
}

fn json_to_value(j: &crate::json::Json) -> Value {
    match j {
        crate::json::Json::Null => Value::Null,
        crate::json::Json::Bool(b) => Value::Int(i64::from(*b)),
        crate::json::Json::Int(n) => Value::Int(*n),
        crate::json::Json::Float(f) => Value::Float(*f),
        crate::json::Json::Str(s) => Value::Str(s.clone()),
        crate::json::Json::Arr(items) => Value::Array(items.iter().map(json_to_value).collect()),
        crate::json::Json::Obj(pairs) => Value::Map(pairs.iter().map(|(k, v)| (k.clone(), json_to_value(v))).collect()),
    }
}

/// Stringify a Klang `Value` to JSON (`Null` -> `null`, `Int`/`Float`
/// -> numbers, `Str` -> quoted, `Array` -> `[...]`, `Map`/`Struct` ->
/// `{...}`). `pretty != 0` indents with two spaces. Closures and
/// handles are `E-JSON-STRINGIFY`, never silent. Output past
/// 10 MB is `E-JSON-STRINGIFY`.
pub fn json_stringify(v: &Value, pretty: i64) -> Result<String, Diagnostic> {
    const OP: &str = "json_stringify";
    let mut out = String::new();
    write_json(v, pretty != 0, 0, &mut out).map_err(|e| e)?;
    if out.len() > MAX_JSON_OUT {
        return Err(json_stringify_err(OP, &format!("output {} bytes exceeds limit {MAX_JSON_OUT}", out.len())));
    }
    let _ = OP;
    Ok(out)
}

fn write_json(v: &Value, pretty: bool, depth: usize, out: &mut String) -> Result<(), Diagnostic> {
    const OP: &str = "json_stringify";
    if depth > 64 {
        return Err(json_stringify_err(OP, "value too deeply nested (limit 64)"));
    }
    match v {
        Value::Null => out.push_str("null"),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Float(f) => {
            if !f.is_finite() {
                return Err(json_stringify_err(OP, &format!("non-finite float {f} has no JSON form")));
            }
            out.push_str(&format!("{f:?}"));
        }
        Value::Str(s) => {
            out.push('"');
            escape_into(out, s);
            out.push('"');
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return Ok(());
            }
            if pretty {
                out.push_str("[\n");
                for (i, it) in items.iter().enumerate() {
                    indent_into(out, depth + 1);
                    write_json(it, pretty, depth + 1, out)?;
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent_into(out, depth);
                out.push(']');
            } else {
                out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json(it, pretty, depth + 1, out)?;
                }
                out.push(']');
            }
        }
        Value::Map(pairs) => {
            if pairs.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            if pretty {
                out.push_str("{\n");
                for (i, (k, val)) in pairs.iter().enumerate() {
                    indent_into(out, depth + 1);
                    out.push('"');
                    escape_into(out, k);
                    out.push_str("\": ");
                    write_json(val, pretty, depth + 1, out)?;
                    if i + 1 < pairs.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent_into(out, depth);
                out.push('}');
            } else {
                out.push('{');
                for (i, (k, val)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    escape_into(out, k);
                    out.push_str("\":");
                    write_json(val, pretty, depth + 1, out)?;
                }
                out.push('}');
            }
        }
        Value::Struct { fields, .. } => {
            // Structs encode like maps (field names -> values).
            let as_map = Value::Map(fields.clone());
            write_json(&as_map, pretty, depth, out)?;
        }
        other => {
            return Err(json_stringify_err(OP, &format!("value of type {} has no JSON form", crate::runtime::value_type_name_pub(other))));
        }
    }
    if out.len() > MAX_JSON_OUT + 1024 {
        return Err(json_stringify_err(OP, &format!("output exceeds limit {MAX_JSON_OUT}")));
    }
    Ok(())
}

fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

fn indent_into(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

// ---------------------------------------------------------------------------
// Routing helper.
// ---------------------------------------------------------------------------

/// `route_match(pattern, path) -> params map or null`.
///
/// `pattern` segments split on `/`; `:name` captures one segment into
/// the map; exact segments must match. Trailing slashes are ignored
/// (`/notes/` matches `/notes`). No match is `Null` (never an error).
/// Empty pattern/path or empty `:name` is `E-HTTP-ROUTE`.
pub fn route_match(pattern: &str, path: &str) -> Result<Value, Diagnostic> {
    const OP: &str = "route_match";
    if pattern.is_empty() {
        return Err(route_err(OP, "pattern is empty"));
    }
    if path.is_empty() {
        return Err(route_err(OP, "path is empty"));
    }
    if pattern.contains('\0') || path.contains('\0') {
        return Err(route_err(OP, "pattern/path contains NUL"));
    }
    let pat_segs: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // Root ("/") matches only root.
    if pat_segs.is_empty() && path_segs.is_empty() {
        return Ok(Value::Map(Vec::new()));
    }
    if pat_segs.len() != path_segs.len() {
        return Ok(Value::Null);
    }
    let mut params = Vec::new();
    for (pat, got) in pat_segs.iter().zip(path_segs.iter()) {
        if let Some(name) = pat.strip_prefix(':') {
            if name.is_empty() {
                return Err(route_err(OP, &format!("empty param name in pattern {pattern:?}")));
            }
            // Percent-decode the captured segment (best-effort: raw on failure).
            params.push((name.to_string(), Value::Str(percent_decode(got))));
        } else if *pat == *got {
            // exact
        } else {
            return Ok(Value::Null);
        }
    }
    Ok(Value::Map(params))
}

// ---------------------------------------------------------------------------
// Response constructors (pure, sandbox-allowed).
// ---------------------------------------------------------------------------

fn check_status(op: &str, status: i64) -> Result<i64, Diagnostic> {
    if !(100..=599).contains(&status) {
        return Err(status_err(op, status));
    }
    Ok(status)
}

/// `http_json(status, value) -> response map` with
/// `content-type: application/json` and a compact JSON body
/// (`Null` -> `null`). Bad statuses are `E-HTTP-STATUS`.
pub fn http_json(status: i64, v: &Value) -> Result<Value, Diagnostic> {
    const OP: &str = "http_json";
    check_status(OP, status)?;
    let body = json_stringify(v, 0)?;
    Ok(Value::Map(vec![
        ("status".to_string(), Value::Int(status)),
        ("headers".to_string(), Value::Map(vec![("content-type".to_string(), Value::Str("application/json".to_string()))])),
        ("body".to_string(), Value::Str(body)),
    ]))
}

/// `http_text(status, s) -> response map` with
/// `content-type: text/plain; charset=utf-8`.
pub fn http_text(status: i64, s: &str) -> Result<Value, Diagnostic> {
    const OP: &str = "http_text";
    check_status(OP, status)?;
    Ok(Value::Map(vec![
        ("status".to_string(), Value::Int(status)),
        ("headers".to_string(), Value::Map(vec![("content-type".to_string(), Value::Str("text/plain; charset=utf-8".to_string()))])),
        ("body".to_string(), Value::Str(s.to_string())),
    ]))
}

/// `http_redirect(url) -> 302 response` with a `location` header.
pub fn http_redirect(url: &str) -> Result<Value, Diagnostic> {
    const OP: &str = "http_redirect";
    if url.is_empty() || url.contains('\0') {
        return Err(url_err(OP, url, "redirect URL is empty or contains NUL"));
    }
    Ok(Value::Map(vec![
        ("status".to_string(), Value::Int(302)),
        ("headers".to_string(), Value::Map(vec![("location".to_string(), Value::Str(url.to_string()))])),
        ("body".to_string(), Value::Str(String::new())),
    ]))
}

// ---------------------------------------------------------------------------
// Client: http_request (reuses ureq rustls + WebPki, no second TLS stack).
// ---------------------------------------------------------------------------

const VALID_METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

fn check_method(op: &str, method: &str) -> Result<(), Diagnostic> {
    if VALID_METHODS.contains(&method) {
        return Ok(());
    }
    Err(method_err(op, method))
}

fn check_client_timeout(op: &str, ms: i64) -> Result<u64, Diagnostic> {
    if ms < 0 || ms > MAX_CLIENT_TIMEOUT_MS {
        return Err(timeout_err(op, &format!("{ms}ms"), &format!("timeout {ms}ms out of range 0..{MAX_CLIENT_TIMEOUT_MS}")));
    }
    if ms == 0 {
        return Ok(DEFAULT_CLIENT_TIMEOUT_MS);
    }
    Ok(ms as u64)
}

fn agent_with(follow: bool, max_redirs: u32, timeout_ms: u64) -> ureq::Agent {
    let mut b = ureq::config::Config::builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_millis(timeout_ms)));
    if follow {
        b = b.max_redirects(max_redirs).max_redirects_will_error(true);
    } else {
        b = b.max_redirects(0);
    }
    ureq::Agent::new_with_config(b.build())
}

fn map_ureq_error(op: &str, url: &str, err: &ureq::Error) -> Diagnostic {
    match err {
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => url_err(op, url, &err.to_string()),
        ureq::Error::Timeout(_) => timeout_err(op, url, &err.to_string()),
        ureq::Error::TooManyRedirects | ureq::Error::RedirectFailed => redirect_err(op, url, &err.to_string()),
        _ => {
            let s = err.to_string();
            let low = s.to_lowercase();
            if low.contains("timed out") || low.contains("timeout") {
                timeout_err(op, url, &s)
            } else if low.contains("redirect") {
                redirect_err(op, url, &s)
            } else {
                connect_err(op, url, &s)
            }
        }
    }
}

fn response_to_value(status: u16, headers: Vec<(String, String)>, body: String) -> Value {
    Value::Map(vec![
        ("status".to_string(), Value::Int(i64::from(status))),
        ("body".to_string(), Value::Str(body)),
        ("headers".to_string(), Value::Map(headers.into_iter().map(|(k, v)| (k, Value::Str(v))).collect())),
    ])
}

fn drain_response(op: &str, url: &str, res: ureq::http::Response<ureq::Body>) -> Result<Value, Diagnostic> {
    let status = res.status().as_u16();
    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in res.headers().iter() {
        let n = name.as_str().to_lowercase();
        let v = value.to_str().map(str::to_string).unwrap_or_else(|_| String::from_utf8_lossy(value.as_bytes()).into_owned());
        match headers.iter_mut().find(|(k, _)| *k == n) {
            Some((_, old)) => {
                old.push_str(", ");
                old.push_str(&v);
            }
            None => headers.push((n, v)),
        }
    }
    let bytes = res.into_body().read_to_vec().map_err(|e| map_ureq_error(op, url, &e))?;
    Ok(response_to_value(status, headers, String::from_utf8_lossy(&bytes).into_owned()))
}

/// `http_request(method, url, headers_map, body, timeout_ms[, options])`.
/// `headers_map` is a `Map` of `str -> str` (keys case-insensitive on
/// the wire, lower-cased in the response). `body` is a `Str`.
/// Redirects follow up to 5 by default; pass `{"follow_redirects": 0}`
/// as the 6th arg to disable. Errors are `E-HTTP-*`.
pub fn http_request(
    method: &str,
    url: &str,
    headers_val: &Value,
    body: &str,
    timeout_ms: i64,
    options_val: Option<&Value>,
) -> Result<Value, Diagnostic> {
    const OP: &str = "http_request";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    check_method(OP, method)?;
    if url.is_empty() || url.contains('\0') {
        return Err(url_err(OP, url, "URL is empty or contains NUL"));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(url_err(OP, url, "want an absolute http(s) URL"));
    }
    let headers: Vec<(String, String)> = match headers_val {
        Value::Map(pairs) => {
            let mut out = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                let vs = match v {
                    Value::Str(s) => s.clone(),
                    other => return Err(header_err(OP, &format!("header {k:?} value is {}, want str", crate::runtime::value_type_name_pub(other)))),
                };
                if k.is_empty() || k.contains('\0') || vs.contains('\0') {
                    return Err(header_err(OP, &format!("header {k:?} is empty or contains NUL")));
                }
                // Validate via the http crate (same rules as http_post).
                if k.parse::<ureq::http::header::HeaderName>().is_err() {
                    return Err(header_err(OP, &format!("bad header name {k:?}")));
                }
                if vs.parse::<ureq::http::header::HeaderValue>().is_err() {
                    return Err(header_err(OP, &format!("bad value for header {k:?}")));
                }
                out.push((k.clone(), vs));
            }
            out
        }
        other => return Err(crate::runtime::type_err_pub(OP, "a headers map", other)),
    };
    let wait = check_client_timeout(OP, timeout_ms)?;
    // Redirect options (6th arg, optional).
    let mut follow = true;
    let mut max_redirs: u32 = 5;
    if let Some(opts) = options_val {
        match opts {
            Value::Map(pairs) => {
                for (k, v) in pairs {
                    match k.as_str() {
                        "follow_redirects" => {
                            let n = match v {
                                Value::Int(n) => *n,
                                other => return Err(options_err(OP, &format!("follow_redirects is {}, want 0/1", crate::runtime::value_type_name_pub(other)))),
                            };
                            if n == 0 {
                                follow = false;
                            } else if n == 1 {
                                follow = true;
                            } else {
                                return Err(options_err(OP, &format!("follow_redirects {n} (want 0 or 1)")));
                            }
                        }
                        "max_redirects" => {
                            let n = match v {
                                Value::Int(n) => *n,
                                other => return Err(options_err(OP, &format!("max_redirects is {}, want int", crate::runtime::value_type_name_pub(other)))),
                            };
                            if !(1..=10).contains(&n) {
                                return Err(options_err(OP, &format!("max_redirects {n} (want 1..10)")));
                            }
                            max_redirs = n as u32;
                        }
                        other => return Err(options_err(OP, &format!("unknown option {other:?} (want follow_redirects, max_redirects)"))),
                    }
                }
            }
            other => return Err(crate::runtime::type_err_pub(OP, "an options map", other)),
        }
    }
    let agent = agent_with(follow, max_redirs, wait);
    // Build the request by method. GET/HEAD carry no body (ignored when
    // non-empty, documented); others send `body` as-is (lossy UTF-8
    // already, since Klang strings are UTF-8). `WithoutBody` vs
    // `WithBody` builders are distinct types, so each method builds and
    // sends in its own arm (no shared variable).
    let res = match method {
        "GET" => {
            let mut req = agent.get(url);
            for (k, v) in &headers {
                req = req.header(k.clone(), v.clone());
            }
            req.call().map_err(|e| map_ureq_error(OP, url, &e))?
        }
        "HEAD" => {
            let mut req = agent.head(url);
            for (k, v) in &headers {
                req = req.header(k.clone(), v.clone());
            }
            req.call().map_err(|e| map_ureq_error(OP, url, &e))?
        }
        "POST" => {
            let mut req = agent.post(url);
            for (k, v) in &headers {
                req = req.header(k.clone(), v.clone());
            }
            req.send(body).map_err(|e| map_ureq_error(OP, url, &e))?
        }
        "PUT" => {
            let mut req = agent.put(url);
            for (k, v) in &headers {
                req = req.header(k.clone(), v.clone());
            }
            req.send(body).map_err(|e| map_ureq_error(OP, url, &e))?
        }
        "DELETE" => {
            // `WithoutBody`: `body` is ignored when non-empty (documented).
            let mut req = agent.delete(url);
            for (k, v) in &headers {
                req = req.header(k.clone(), v.clone());
            }
            req.call().map_err(|e| map_ureq_error(OP, url, &e))?
        }
        "PATCH" => {
            let mut req = agent.patch(url);
            for (k, v) in &headers {
                req = req.header(k.clone(), v.clone());
            }
            req.send(body).map_err(|e| map_ureq_error(OP, url, &e))?
        }
        "OPTIONS" => {
            // `WithoutBody`: `body` is ignored when non-empty (documented).
            let mut req = agent.options(url);
            for (k, v) in &headers {
                req = req.header(k.clone(), v.clone());
            }
            req.call().map_err(|e| map_ureq_error(OP, url, &e))?
        }
        _ => unreachable!("checked"),
    };
    drain_response(OP, url, res)
}

// ---------------------------------------------------------------------------
// Server options + HTTP parsing helpers (pure, tested directly).
// ---------------------------------------------------------------------------

/// Parsed `http_serve` options.
#[derive(Debug, Clone, Copy)]
pub struct ServeOptions {
    pub workers: usize,
    pub max_body_bytes: usize,
    pub read_timeout_ms: u64,
    pub max_header_bytes: usize,
    /// Idle keep-alive timeout: a conn with no next request for this long
    /// is quietly closed (0 = never expire). Applied only between requests.
    pub idle_timeout_ms: u64,
    /// Cap on requests served over one keep-alive conn (0 = unlimited).
    pub max_requests_per_conn: u64,
}

impl Default for ServeOptions {
    fn default() -> Self {
        Self {
            workers: DEFAULT_WORKERS,
            max_body_bytes: DEFAULT_MAX_BODY,
            read_timeout_ms: DEFAULT_READ_TIMEOUT_MS,
            max_header_bytes: DEFAULT_MAX_HEADER,
            idle_timeout_ms: DEFAULT_IDLE_TIMEOUT_MS,
            max_requests_per_conn: DEFAULT_MAX_REQUESTS_PER_CONN,
        }
    }
}

/// Parse the `options` map for `http_serve`. Unknown keys are
/// `E-HTTP-OPTIONS`; out-of-range values are `E-HTTP-OPTIONS`.
pub fn parse_serve_options(v: &Value) -> Result<ServeOptions, Diagnostic> {
    const OP: &str = "http_serve";
    let mut out = ServeOptions::default();
    let pairs = match v {
        Value::Map(pairs) => pairs,
        other => return Err(crate::runtime::type_err_pub(OP, "an options map", other)),
    };
    for (k, val) in pairs {
        let n = match val {
            Value::Int(n) => *n,
            other => return Err(options_err(OP, &format!("option {k:?} is {}, want int", crate::runtime::value_type_name_pub(other)))),
        };
        match k.as_str() {
            "workers" => {
                if !(1..=(MAX_WORKERS as i64)).contains(&n) {
                    return Err(options_err(OP, &format!("workers {n} (want 1..{MAX_WORKERS})")));
                }
                out.workers = n as usize;
            }
            "max_body_bytes" => {
                if n < 0 || (n as usize) > MAX_BODY_CAP {
                    return Err(options_err(OP, &format!("max_body_bytes {n} (want 0..{MAX_BODY_CAP})")));
                }
                out.max_body_bytes = n as usize;
            }
            "read_timeout_ms" => {
                if n < 0 || (n as u64) > MAX_READ_TIMEOUT_MS {
                    return Err(options_err(OP, &format!("read_timeout_ms {n} (want 0..{MAX_READ_TIMEOUT_MS})")));
                }
                out.read_timeout_ms = n as u64;
            }
            "max_header_bytes" => {
                if n < 1024 || (n as usize) > MAX_HEADER_CAP {
                    return Err(options_err(OP, &format!("max_header_bytes {n} (want 1024..{MAX_HEADER_CAP})")));
                }
                out.max_header_bytes = n as usize;
            }
            "idle_timeout_ms" => {
                if n < 0 || (n as u64) > MAX_IDLE_TIMEOUT_MS {
                    return Err(options_err(OP, &format!("idle_timeout_ms {n} (want 0..{MAX_IDLE_TIMEOUT_MS})")));
                }
                out.idle_timeout_ms = n as u64;
            }
            "max_requests_per_conn" => {
                if n < 0 || (n as u64) > MAX_REQUESTS_PER_CONN {
                    return Err(options_err(OP, &format!("max_requests_per_conn {n} (want 0..{MAX_REQUESTS_PER_CONN})")));
                }
                out.max_requests_per_conn = n as u64;
            }
            other => return Err(options_err(OP, &format!("unknown option {other:?} (want workers, max_body_bytes, read_timeout_ms, max_header_bytes, idle_timeout_ms, max_requests_per_conn)"))),
        }
    }
    Ok(out)
}

/// Percent-decode `%XX` and `+` (query form). Invalid escapes stay raw
/// (never a panic, never an error — the map holds the raw text).
pub fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 1 {
            if i + 2 < b.len() {
                let hi = hex_val(b[i + 1]);
                let lo = hex_val(b[i + 2]);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push(((h << 4) | l) as char);
                    i += 3;
                    continue;
                }
            }
            out.push('%');
            i += 1;
        } else if b[i] == b'+' {
            out.push(' ');
            i += 1;
        } else {
            // ASCII fast path; multibyte UTF-8 copies verbatim.
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Parse `a=1&b=2` into a map (first value wins on repeats, `+` is
/// space, `%XX` decoded). Empty query is an empty map.
pub fn parse_query(q: &str) -> Vec<(String, Value)> {
    if q.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(String, Value)> = Vec::new();
    for part in q.split('&') {
        if part.is_empty() {
            continue;
        }
        let (k, v) = match part.split_once('=') {
            Some((k, v)) => (k, v),
            None => (part, ""),
        };
        let key = percent_decode(k);
        if out.iter().any(|(ek, _)| *ek == key) {
            continue;
        }
        out.push((key, Value::Str(percent_decode(v))));
    }
    out
}

/// Split `GET /path?q=1 HTTP/1.1` into `(method, target, version)`.
/// Anything else is `None` (caller sends 400).
pub fn split_request_line(line: &str) -> Option<(String, String, String)> {
    let mut parts = line.split_whitespace();
    let m = parts.next()?;
    let t = parts.next()?;
    let v = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if m.is_empty() || t.is_empty() || v.is_empty() {
        return None;
    }
    if !v.starts_with("HTTP/") {
        return None;
    }
    Some((m.to_string(), t.to_string(), v.to_string()))
}

/// Split `/path?query` into `(path, query)`. Empty path becomes `/`.
pub fn split_target(target: &str) -> (String, String) {
    match target.split_once('?') {
        Some((p, q)) => {
            let path = if p.is_empty() { "/".to_string() } else { p.to_string() };
            (path, q.to_string())
        }
        None => {
            let path = if target.is_empty() { "/".to_string() } else { target.to_string() };
            (path, String::new())
        }
    }
}

/// Reason phrase for a status code.
pub fn reason_for(status: i64) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        413 => "Content Too Large",
        500 => "Internal Server Error",
        _ if (100..200).contains(&status) => "Informational",
        _ if (200..300).contains(&status) => "OK",
        _ if (300..400).contains(&status) => "Redirect",
        _ if (400..500).contains(&status) => "Bad Request",
        _ => "Error",
    }
}

/// Format one HTTP response (`status`, `headers`, `body`) into bytes.
/// Always sets `content-length`; preserves caller headers (lower-case
/// lookup for content-type/connection). `keep_alive` controls the
/// `connection` header value.
pub fn format_response(status: i64, headers: &[(String, String)], body: &str, keep_alive: bool) -> String {
    let mut out = format!("HTTP/1.1 {status} {}\r\n", reason_for(status));
    let mut has_len = false;
    let mut has_conn = false;
    for (k, v) in headers {
        let lk = k.to_lowercase();
        if lk == "content-length" {
            has_len = true;
        }
        if lk == "connection" {
            has_conn = true;
        }
        // Header values are already validated on the Klang side (no
        // CR/LF allowed — checked when the response map is read).
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    if !has_len {
        out.push_str(&format!("content-length: {}\r\n", body.len()));
    }
    if !has_conn {
        if keep_alive {
            out.push_str("connection: keep-alive\r\n");
        } else {
            out.push_str("connection: close\r\n");
        }
    }
    out.push_str("\r\n");
    out.push_str(body);
    out
}

/// Validate response-map headers (no CR/LF, non-empty name). Returns
/// the header list on success.
pub fn check_response_headers(v: &Value) -> Result<Vec<(String, String)>, String> {
    let pairs = match v {
        Value::Map(pairs) => pairs,
        _ => return Err("headers is not a map".to_string()),
    };
    let mut out = Vec::with_capacity(pairs.len());
    for (k, val) in pairs {
        let vs = match val {
            Value::Str(s) => s.clone(),
            _ => return Err(format!("header {k:?} value is not a string")),
        };
        if k.is_empty() || k.contains(['\r', '\n', '\0']) || vs.contains(['\r', '\n', '\0']) {
            return Err(format!("bad header {k:?}"));
        }
        out.push((k.clone(), vs));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Server handles.
// ---------------------------------------------------------------------------

/// Opaque HTTP server. `running` flips false on `http_stop` (or at
/// program exit); the accept loop blocks on `listener_accept_blocking`
/// and is woken by a self-connect (`listener_wake`). Job queue is an A2
/// channel of `Value::Conn`; idle keep-alive conns park in `idle` (one
/// poller thread per server).
pub struct ServerState {
    pub listener: crate::stdlib::a1_net::ListenerHandle,
    pub port: i64,
    pub handler: String,
    pub workers: usize,
    pub max_body_bytes: usize,
    pub read_timeout_ms: u64,
    pub max_header_bytes: usize,
    pub running: Arc<AtomicBool>,
    pub jobs: Value,
    /// Parked idle keep-alive conns (the poller peeks + hands them back).
    pub idle: std::sync::Arc<IdlePool>,
    /// Accept + worker + poller join handles (taken on `http_stop`).
    pub accept: Option<std::thread::JoinHandle<()>>,
    pub worker_handles: Vec<std::thread::JoinHandle<()>>,
    pub poller: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerState")
            .field("port", &self.port)
            .field("handler", &self.handler)
            .field("workers", &self.workers)
            .field("live", &self.running.load(Ordering::SeqCst))
            .finish()
    }
}

pub type ServerHandle = Arc<Mutex<ServerState>>;

thread_local! {
    static IS_HTTP_WORKER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True on a server worker thread (so `http_stop` from a handler only
/// signals instead of joining itself).
pub fn is_http_worker() -> bool {
    IS_HTTP_WORKER.with(|c| c.get())
}

/// Mark the current thread as a server worker (called once per worker).
pub fn mark_http_worker() {
    IS_HTTP_WORKER.with(|c| c.set(true));
}

// ---------------------------------------------------------------------------
// Idle keep-alive pool (fix for worker starvation).
// ---------------------------------------------------------------------------

/// Parked idle keep-alive connections, owned by one poller thread per
/// server. A worker that finishes a keep-alive response hands the conn
/// here when no next request is already readable; the poller peeks each
/// parked socket (never consuming bytes) and hands readable conns back to
/// the worker pool. Workers therefore never block on idle sockets, and an
/// idle conn is never closed just because other work is queued.
pub struct IdlePool {
    inner: std::sync::Mutex<IdleInner>,
    cv: std::sync::Condvar,
}

#[derive(Default)]
struct IdleInner {
    /// Parked conns with the time of their last served request.
    conns: Vec<(Value, std::time::Instant)>,
    stopped: bool,
}

impl IdlePool {
    pub fn new() -> Self {
        Self { inner: std::sync::Mutex::new(IdleInner::default()), cv: std::sync::Condvar::new() }
    }

    /// Hand a keep-alive conn to the poller (worker → pool ownership).
    /// False when the pool is stopped: the caller must close the conn.
    pub fn park(&self, conn: Value) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if g.stopped {
            return false;
        }
        g.conns.push((conn, std::time::Instant::now()));
        self.cv.notify_all();
        true
    }

    /// Shutdown: the poller closes every parked conn and exits.
    pub fn stop(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.stopped = true;
        self.cv.notify_all();
    }
}

/// The idle poller: round-robin peeks parked conns and hands readable
/// ones back to the job queue. Per-conn probe timeout shrinks with the
/// pool size so one detection cycle stays ≤ ~50 ms regardless of how many
/// conns are parked; `idle_timeout_ms` (0 = never) closes conns idle past
/// the budget quietly.
pub fn idle_poller_loop(pool: std::sync::Arc<IdlePool>, jobs: Value, running: Arc<AtomicBool>, idle_timeout_ms: u64) {
    let drain_close = |conns: &mut Vec<(Value, std::time::Instant)>| {
        for (c, _) in conns.drain(..) {
            let _ = crate::stdlib::a1_net::conn_close(&c);
        }
    };
    loop {
        let (batch, probe) = {
            let mut g = pool.inner.lock().unwrap_or_else(|e| e.into_inner());
            if g.stopped {
                drain_close(&mut g.conns);
                return;
            }
            if g.conns.is_empty() {
                let (g2, _t) = pool
                    .cv
                    .wait_timeout(g, std::time::Duration::from_millis(200))
                    .unwrap_or_else(|e| e.into_inner());
                g = g2;
                if g.stopped {
                    drain_close(&mut g.conns);
                    return;
                }
                if g.conns.is_empty() {
                    continue;
                }
            }
            let n = g.conns.len();
            let probe = (50 / n as u64).max(1);
            (std::mem::take(&mut g.conns), probe)
        };
        let mut still_idle: Vec<(Value, std::time::Instant)> = Vec::new();
        for (c, since) in batch {
            if !running.load(Ordering::SeqCst) {
                // Shutting down: the drain rule still serves a request
                // that is already readable on an accepted conn. Anything
                // else (still idle, EOF, error) closes quietly.
                match crate::stdlib::a1_net::conn_poll_readable(&c, probe as i64) {
                    Ok(crate::stdlib::a1_net::ConnPoll::Data) => {
                        if crate::stdlib::a2_threads::send(&jobs, &c).is_err() {
                            let _ = crate::stdlib::a1_net::conn_close(&c);
                        }
                    }
                    _ => {
                        let _ = crate::stdlib::a1_net::conn_close(&c);
                    }
                }
                continue;
            }
            if idle_timeout_ms != 0 && since.elapsed() >= std::time::Duration::from_millis(idle_timeout_ms) {
                // Idle budget spent: quiet close (no 408).
                let _ = crate::stdlib::a1_net::conn_close(&c);
                continue;
            }
            match crate::stdlib::a1_net::conn_poll_readable(&c, probe as i64) {
                Ok(crate::stdlib::a1_net::ConnPoll::Data) => {
                    // A request (complete or partial) arrived: the next
                    // worker reads and serves it (running=false still
                    // serves one response — the drain rule, handled above).
                    if crate::stdlib::a2_threads::send(&jobs, &c).is_err() {
                        let _ = crate::stdlib::a1_net::conn_close(&c);
                    }
                }
                Ok(crate::stdlib::a1_net::ConnPoll::Eof) => {
                    let _ = crate::stdlib::a1_net::conn_close(&c);
                }
                Ok(crate::stdlib::a1_net::ConnPoll::Timeout) => still_idle.push((c, since)),
                Err(_) => {
                    let _ = crate::stdlib::a1_net::conn_close(&c);
                }
            }
        }
        if !still_idle.is_empty() {
            let mut g = pool.inner.lock().unwrap_or_else(|e| e.into_inner());
            if g.stopped {
                drain_close(&mut still_idle);
                drain_close(&mut g.conns);
                return;
            }
            g.conns.extend(still_idle);
        }
    }
}

fn as_server(op: &str, v: &Value) -> Result<ServerHandle, Diagnostic> {
    match v {
        Value::Server(h) => Ok(h.clone()),
        other => Err(crate::runtime::type_err_pub(op, "a Server", other)),
    }
}

/// `server_port(s) -> Int`. Stopped servers are `E-HTTP-CLOSED`
/// (like `listener_port` on a closed listener); `http_stop` itself stays
/// idempotent.
pub fn server_port(v: &Value) -> Result<i64, Diagnostic> {
    const OP: &str = "server_port";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_server(OP, v)?;
    let st = h.lock().unwrap_or_else(|e| e.into_inner());
    if !st.running.load(Ordering::SeqCst) {
        return Err(closed_err(OP));
    }
    Ok(st.port)
}

/// `http_stop(s) -> 1` (graceful: stop accepting new work, drain the
/// listen backlog, finish every accepted connection, then close the
/// listener last). Signals stop, wakes the acceptor with a self-connect
/// (a `close()` does not wake a blocked accept on Linux), joins the
/// accept + worker threads unless called from a worker (then a detached
/// finisher thread does the join). Idempotent.
pub fn http_stop(v: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "http_stop";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_server(OP, v)?;
    // Signal + wake under the lock, take handles out, join outside.
    let (listener, jobs, accept, workers, poller, idle) = {
        let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
        if st.accept.is_none() {
            return Ok(()); // already stopped / already finishing (idempotent)
        }
        st.running.store(false, Ordering::SeqCst);
        let listener = st.listener.clone();
        let jobs = st.jobs.clone();
        let accept = st.accept.take();
        let workers = std::mem::take(&mut st.worker_handles);
        let poller = st.poller.take();
        let idle = st.idle.clone();
        (listener, jobs, accept, workers, poller, idle)
    };
    // Wake the blocking accept: the wake socket is dropped, so the
    // accepted end reads instant EOF (it can never stall a worker).
    let _ = crate::stdlib::a1_net::listener_wake(&Value::Listener(listener.clone()));
    if is_http_worker() {
        // Self-join would deadlock; a detached finisher (not marked as a
        // worker) does the join + drain. Its handle replaces `accept`, so a
        // later stop/reap joins the finisher instead.
        let fin = std::thread::Builder::new()
            .name("klang-http-finisher".to_string())
            .spawn(move || finish_shutdown(listener, jobs, accept, workers, poller, idle))
            .ok();
        if let Some(fin) = fin {
            let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
            st.accept = Some(fin);
        }
        return Ok(());
    }
    finish_shutdown(listener, jobs, accept, workers, poller, idle);
    Ok(())
}

/// Join the acceptor, drain the listen backlog into the job queue, stop
/// the idle poller (closing parked conns), close the queue, join the
/// workers, and close the listener last. Every connection accepted (or
/// already handshake-completed in the backlog) at the time of the stop
/// request is served exactly once.
fn finish_shutdown(
    listener: crate::stdlib::a1_net::ListenerHandle,
    jobs: Value,
    accept: Option<std::thread::JoinHandle<()>>,
    workers: Vec<std::thread::JoinHandle<()>>,
    poller: Option<std::thread::JoinHandle<()>>,
    idle: std::sync::Arc<IdlePool>,
) {
    if let Some(jh) = accept {
        let _ = jh.join();
    }
    // Drain: take every connection still queued in the kernel backlog
    // (handshake complete, never accepted) and hand it to the workers.
    loop {
        match crate::stdlib::a1_net::listener_accept_nonblocking(&Value::Listener(listener.clone())) {
            Ok(v @ Value::Conn(_)) => {
                // Blocking send: workers are still draining the queue, so
                // this frees up (never deadlocks).
                let _ = crate::stdlib::a2_threads::send(&jobs, &v);
            }
            _ => break, // backlog empty (WouldBlock) or listener gone
        }
    }
    // Stop the idle poller BEFORE closing the job queue: it may still be
    // handing readable conns over. stop() makes it close every parked conn
    // and exit; join so no further sends race the queue close.
    idle.stop();
    if let Some(jh) = poller {
        let _ = jh.join();
    }
    // Close the job queue so idle workers wake; blocked recvs drain the
    // remaining connections first, then see null.
    let _ = crate::stdlib::a2_threads::close(&jobs);
    for jh in workers {
        let _ = jh.join();
    }
    // Close the listener last: draining needs it open.
    let _ = crate::stdlib::a1_net::conn_close(&Value::Listener(listener));
}

/// Is this server still running?
pub fn is_running(h: &ServerHandle) -> bool {
    h.lock().unwrap_or_else(|e| e.into_inner()).running.load(Ordering::SeqCst)
}

/// Redacted helper for diagnostics.
pub fn _redacted(s: &str) -> String {
    redact_body(s)
}

//! Klang OS-interop HTTP boundary (STDLIB-NET-1).
//!
//! `http::get` / `http::post` in the PRD map onto Klang's flat builtins
//! `http_get(url)` / `http_post(url, body, headers)` (see
//! `hir::is_builtin` and `runtime::exec_builtin`). Flat names are
//! deliberate: Klang's `::` call syntax only resolves through declared
//! `mod` blocks (`modules::rewrite_ctor` converts `m::f(args)` into a
//! `Call` only when `m` is a program module), so a namespaced
//! `http::get(...)` spelling would parse as an enum constructor and
//! fail type checking. Consistency with the existing builtins matters
//! more than the PRD's illustrative `::` shape.
//!
//! This is a real synchronous HTTP client over [`ureq`] (default
//! features: rustls TLS, gzip), not a subprocess or a mock. It lives
//! here — NOT in `net.rs`, whose `MockTransport` is intentionally a
//! test double for the v2 echo system and stays untouched.
//!
//! Design decisions (PRD §2/§3 questions):
//! - HTTP error statuses (4xx/5xx) are normal results populating
//!   `status` in the returned map, NOT diagnostics — parallel to
//!   Phase 2's non-zero-exit decision and Phase 3's non-match
//!   decision. Only transport failures raise. (`http_status_as_error`
//!   is explicitly disabled.)
//! - Request headers are an array of `"Key: Value"` strings, the same
//!   shape as `run_process`'s argv array (array-of-strings precedent)
//!   rather than a map. Response headers are a map of lowercased
//!   name → value (repeated headers joined with `", "`), so
//!   `resp["headers"]["content-type"]` works exactly like
//!   `regex_find`'s nested `m["named"]["user"]`.
//! - Bodies are lossy-UTF-8 strings on the way out (same precedent as
//!   `process::run`'s `String::from_utf8_lossy` stdout capture): a
//!   binary body becomes replacement characters, never an error.
//! - Timeouts: 10s connect, 60s whole-call backstop, so a stalled
//!   server can fail loudly but never hang the interpreter forever.
//! - One `ureq::Agent` per call (no connection reuse): each call is
//!   independent, correct for v1.
//!
//! Security note (PRD §5): `http_get`/`http_post` fetch/send to
//! whatever URL the program passes — same trust model as
//! `process::run` taking a real argv array. Real and unrestricted,
//! no allowlist; credentials are just headers the caller provides
//! (no secrets layer).
//!
//! All I/O goes through the free functions here so both the
//! interpreter (`runtime::exec_builtin`) and unit tests share one
//! error mapping.

use crate::diagnostics::Diagnostic;
use std::time::Duration;

/// Per-attempt TCP/TLS connect timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-call backstop (connect + send + receive).
const GLOBAL_TIMEOUT: Duration = Duration::from_secs(60);

/// `E-NET-UNREACHABLE`: DNS failure, connection refused, timeout,
/// TLS failure, broken server exchange — anything where the URL was
/// fine but the network exchange could not complete. The underlying
/// `ureq` error renders as the cause verbatim (never a shared generic
/// message, cf. AUDIT F14).
fn net_unreachable(op: &str, url: &str, err: &ureq::Error) -> Diagnostic {
    Diagnostic::error(
        "E-NET-UNREACHABLE",
        &format!("{op}({url}) failed: {err}"),
        "runtime",
        0,
        0,
        &err.to_string(),
        &["check the host is reachable and DNS resolves for it"],
        "net/unreachable",
    )
}

/// `E-NET-INVALID-URL`: the URL itself is malformed (missing scheme
/// or host, unparseable). Carries `ureq`'s real parse complaint.
fn net_invalid_url(op: &str, url: &str, err: &ureq::Error) -> Diagnostic {
    Diagnostic::error(
        "E-NET-INVALID-URL",
        &format!("{op}({url}) failed: invalid URL: {err}"),
        "runtime",
        0,
        0,
        &err.to_string(),
        &["pass an absolute http(s) URL such as https://example.com/"],
        "net/invalid-url",
    )
}

/// `E-NET-INVALID-HEADER`: a request-header entry is not
/// `"Name: Value"` (missing colon, empty name) or violates HTTP
/// token rules. Deliberate addition beyond the PRD's two codes: the
/// header-array shape is ours (not the caller's URL, not the
/// network), so its failures get their own code rather than being
/// folded into URL/unreachable errors.
fn net_invalid_header(op: &str, what: &str, detail: &str) -> Diagnostic {
    Diagnostic::error(
        "E-NET-INVALID-HEADER",
        &format!("{op}({what}) failed: invalid header: {detail}"),
        "runtime",
        0,
        0,
        detail,
        &["pass headers as \"Name: Value\" strings, e.g. \"X-Token: abc\""],
        "net/invalid-header",
    )
}

/// Map a `ureq` failure to the specific `E-NET-*` diagnostic for `op`
/// on `url`. `BadUri` (missing scheme/host) and `Http` (illegal URI
/// characters — verified live: `"not a url at all"` surfaces as
/// `Http("http: invalid uri character")`, not `BadUri`) are the two
/// URL-shape failures, so both are `E-NET-INVALID-URL`. The `Http`
/// arm is sound despite also covering header-builder errors: every
/// header we set passes through [`parse_header`]'s own `http`-crate
/// validation first, so by the time `ureq` runs no header-caused
/// `Http` failure remains possible (`http_get` sets no headers at
/// all). Everything else means the exchange itself failed and is
/// `E-NET-UNREACHABLE`.
///
/// `ureq::Error` is `#[non_exhaustive]`, so the trailing arm is
/// structural, not lazy: any future ureq variant defaults to
/// UNREACHABLE (a failed exchange) unless it is specifically a
/// URL problem, in which case this mapping must be extended
/// deliberately.
pub fn map_error(op: &str, url: &str, err: &ureq::Error) -> Diagnostic {
    match err {
        ureq::Error::BadUri(_) | ureq::Error::Http(_) => net_invalid_url(op, url, err),
        _ => net_unreachable(op, url, err),
    }
}

/// Parse one `"Name: Value"` request-header entry for `op`, enforcing
/// the real HTTP token rules up front (via the `http` crate's own
/// parsers) so `ureq` can never fail later on input we accepted.
fn parse_header(op: &str, entry: &str) -> Result<(String, String), Diagnostic> {
    let (name, value) = entry.split_once(':').ok_or_else(|| {
        net_invalid_header(
            op,
            entry,
            "expected \"Name: Value\" with a colon separator",
        )
    })?;
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() {
        return Err(net_invalid_header(op, entry, "header name is empty"));
    }
    if let Err(e) = name.parse::<ureq::http::header::HeaderName>() {
        return Err(net_invalid_header(op, entry, &e.to_string()));
    }
    if let Err(e) = value.parse::<ureq::http::header::HeaderValue>() {
        return Err(net_invalid_header(op, entry, &e.to_string()));
    }
    Ok((name.to_string(), value.to_string()))
}

/// A completed HTTP exchange: transport succeeded, whatever the
/// status code (4xx/5xx included — statuses are data, not errors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Numeric status code (200, 404, …).
    pub status: u16,
    /// Response body as (lossy) UTF-8.
    pub body: String,
    /// Lowercased header names in first-seen order; repeats joined
    /// with `", "`.
    pub headers: Vec<(String, String)>,
}

/// One independent client per call (no shared pool, no reuse).
fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .http_status_as_error(false)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(GLOBAL_TIMEOUT))
            .build(),
    )
}

/// Drain one `ureq` response into an [`HttpResponse`]. A mid-body
/// transport failure maps to `E-NET-UNREACHABLE` with the real cause.
fn to_response(
    op: &str,
    url: &str,
    res: ureq::http::Response<ureq::Body>,
) -> Result<HttpResponse, Diagnostic> {
    let status = res.status().as_u16();
    let mut headers: Vec<(String, String)> = Vec::new();
    for (name, value) in res.headers().iter() {
        let n = name.as_str().to_lowercase();
        let v = value
            .to_str()
            .map(str::to_string)
            .unwrap_or_else(|_| {
                String::from_utf8_lossy(value.as_bytes()).into_owned()
            });
        match headers.iter_mut().find(|(k, _)| *k == n) {
            Some((_, old)) => {
                old.push_str(", ");
                old.push_str(&v);
            }
            None => headers.push((n, v)),
        }
    }
    let bytes = res
        .into_body()
        .read_to_vec()
        .map_err(|e| map_error(op, url, &e))?;
    Ok(HttpResponse {
        status,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        headers,
    })
}

/// `GET url` → [`HttpResponse`]. Transport failures raise `E-NET-*`;
/// HTTP error statuses do not (they populate `status`).
pub fn get(url: &str) -> Result<HttpResponse, Diagnostic> {
    let res = agent()
        .get(url)
        .call()
        .map_err(|e| map_error("http_get", url, &e))?;
    to_response("http_get", url, res)
}

/// `POST url` with string `body` and `"Name: Value"` `headers` →
/// [`HttpResponse`]. Same status/transport contract as [`get`].
pub fn post(url: &str, body: &str, headers: &[String]) -> Result<HttpResponse, Diagnostic> {
    let mut req = agent().post(url);
    for h in headers {
        let (k, v) = parse_header("http_post", h)?;
        req = req.header(k, v);
    }
    let res = req
        .send(body)
        .map_err(|e| map_error("http_post", url, &e))?;
    to_response("http_post", url, res)
}

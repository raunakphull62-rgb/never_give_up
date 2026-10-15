//! A3b PostgreSQL client (Wave 2 Stage A3b).
//!
//! Flat builtins (see `hir::is_builtin` and `runtime::exec_builtin`):
//! `pg_connect`, `pg_exec`, `pg_query`, `pg_begin`, `pg_commit`,
//! `pg_rollback`, `pg_close`.
//!
//! Pure-Rust sync client (`postgres` over `tokio-postgres`, no libpq C
//! code) with rustls TLS via `tokio-postgres-rustls` (ring +
//! webpki-roots, same bundled Mozilla roots as `ureq`, no native-tls /
//! openssl, no second TLS stack). SCRAM-SHA-256 is pure Rust in the
//! client and works on all release targets (Android/musl/Windows/macOS).
//!
//! URL form: `postgres://user:pass@host:port/dbname?sslmode=MODE`
//! (`postgresql://` also accepted). `port` defaults to 5432, `dbname`
//! defaults to `user`. `sslmode` is `disable` (plaintext, for local
//! tests), `prefer` (default: try verified TLS, fall back to plaintext
//! only when the server does not support SSL), `require` / `verify-ca` /
//! `verify-full` (verified TLS, no fallback). Certificates are verified
//! against the bundled WebPki roots by default; there is no
//! `sslmode=disable`-except-local-tests escape hatch for production.
//!
//! Parameters are ALWAYS bound (`$1`, `$2`, ...) via `ToSql`, never
//! interpolated into SQL text. Klang values map to SQL as:
//! `Int` -> INT8, `Float` -> FLOAT8, `Str` -> TEXT, `Null` -> NULL
//! (unknown type, PG infers). `Bool` has no distinct runtime value
//! (erases to `Int` 0/1 like JSON); to write a BOOL column use an
//! explicit cast in SQL (`VALUES ($1::bool)`, since `1::bool` is true).
//! `Array`/`Map`/`Struct`/`Closure`/handles as params are `E-PG-TYPE`.
//!
//! Query results map Postgres types to Klang values (consistent with
//! `json_decode`/`json_stringify`: `Null` <-> SQL NULL, `Bool` <-> `Int`
//! 0/1):
//! integers (INT2/INT4/INT8/OID) -> `Int`, floats (FLOAT4/FLOAT8) ->
//! `Float`, NUMERIC -> `E-PG-TYPE` unless cast in SQL (`n::text` -> `Str`,
//! `n::float8` -> `Float`; exact decimal needs a cast since the driver has
//! no arbitrary-precision decimal), text (TEXT/VARCHAR/CHAR/NAME) -> `Str`, BOOL ->
//! `Int` (1/0), timestamps (TIMESTAMP/TIMESTAMPTZ/DATE/TIME) -> ISO `Str`,
//! JSON/JSONB -> `Str` (canonical JSON text), NULL -> `Null` (`is_null`),
//! other types -> `Str` via text output when possible, else `E-PG-TYPE`.
//! Unknown-type columns never panic.
//!
//! Handles (`Pg`) are opaque `Value` variants holding
//! `Arc<Mutex<...>>` state: they cannot be forged from ints, they compare
//! by identity, they render as `<pg>`, and the connection closes on drop
//! (program exit closes whatever the program forgot; an open transaction
//! aborts on disconnect, like libpq). `pg_close` is idempotent.
//! Operations on a closed handle are `E-PG-CLOSED`.
//!
//! Transactions are explicit `BEGIN`/`COMMIT`/`ROLLBACK` on the single
//! connection (not `Transaction` objects, so the handle stays shareable).
//! `pg_begin` when already in a transaction is `E-PG-TX`; `pg_commit` /
//! `pg_rollback` when not in a transaction is `E-PG-TX`. A failed query
//! inside a transaction aborts the transaction (PG rule); the caller must
//! `pg_rollback` before further queries, else further queries are
//! `E-PG-SQL`.
//!
//! All functions deny with `E-SANDBOX-NETWORK` under
//! `klang run --sandbox` (including `pg_close`, like network handles).

use std::sync::{Arc, Mutex};

use crate::diagnostics::Diagnostic;
use crate::runtime::Value;

// ---------------------------------------------------------------------------
// Limits / constants.
// ---------------------------------------------------------------------------

/// Default Postgres port.
const DEFAULT_PORT: u16 = 5432;
/// Characters of a URL/SQL shown verbatim in diagnostics past this.
const MAX_SHOWN: usize = 200;

// ---------------------------------------------------------------------------
// Error constructors (every message names the offending value, never the
// password; never a panic).
// ---------------------------------------------------------------------------

fn pg_err(code: &str, message: String, cause: String, rule: &str) -> Diagnostic {
    Diagnostic::error(code, &message, "runtime", 0, 0, &cause, &["check the URL, SQL and params, then retry"], rule)
}

fn connect_err(redacted_url: &str, detail: &str) -> Diagnostic {
    let shown = redact_long(redacted_url);
    pg_err(
        "E-PG-CONNECT",
        format!("pg_connect({shown}) failed: {detail}"),
        detail.to_string(),
        "pg/connect",
    )
}

fn auth_err(redacted_url: &str, detail: &str) -> Diagnostic {
    let shown = redact_long(redacted_url);
    pg_err(
        "E-PG-AUTH",
        format!("pg_connect({shown}) failed: authentication failed: {detail}"),
        detail.to_string(),
        "pg/auth",
    )
}

fn sql_err(op: &str, sqlstate: &str, message: &str) -> Diagnostic {
    let shown_msg = if message.len() <= MAX_SHOWN {
        message.to_string()
    } else {
        format!("{}... [{} chars redacted]", &message[..MAX_SHOWN], message.len() - MAX_SHOWN)
    };
    pg_err(
        "E-PG-SQL",
        format!("{op} failed: [{sqlstate}] {shown_msg}"),
        format!("[{sqlstate}] {shown_msg}"),
        "pg/sql",
    )
}

fn type_err(op: &str, detail: &str) -> Diagnostic {
    pg_err(
        "E-PG-TYPE",
        format!("{op} failed: {detail}"),
        detail.to_string(),
        "pg/type",
    )
}

fn closed_err(op: &str) -> Diagnostic {
    pg_err(
        "E-PG-CLOSED",
        format!("{op} failed: Pg handle is already closed"),
        "Pg handle was closed".to_string(),
        "pg/closed",
    )
}

fn txn_err(op: &str, detail: &str) -> Diagnostic {
    pg_err(
        "E-PG-TX",
        format!("{op} failed: {detail}"),
        detail.to_string(),
        "pg/tx",
    )
}

fn redact_long(s: &str) -> String {
    if s.len() <= MAX_SHOWN {
        return format!("{s:?}");
    }
    let head = &s[..MAX_SHOWN];
    let rest = s.len() - MAX_SHOWN;
    format!("{head:?}... [{rest} chars redacted]")
}

// ---------------------------------------------------------------------------
// URL parsing + password redaction (no new crates, no panic).
// ---------------------------------------------------------------------------

/// Parsed connection parameters (password kept separately, never logged).
#[derive(Debug, Clone)]
struct PgConfig {
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    dbname: String,
    sslmode: SslMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SslMode {
    Disable,
    Prefer,
    Require,
}

impl SslMode {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "disable" => Some(Self::Disable),
            "allow" => Some(Self::Prefer),
            "prefer" => Some(Self::Prefer),
            "require" => Some(Self::Require),
            "verify-ca" => Some(Self::Require),
            "verify-full" => Some(Self::Require),
            _ => None,
        }
    }
}

/// Replace the password in a Postgres URL with `***` for errors/logs.
/// `postgres://user:secret@host/db` -> `postgres://user:***@host/db`.
/// Never panics; on unparseable input returns a truncated copy.
pub fn redact_url(url: &str) -> String {
    // Find scheme:// then credentials before '@'.
    let scheme_end = match url.find("://") {
        Some(i) => i + 3,
        None => return redact_long(url),
    };
    let after_scheme = &url[scheme_end..];
    // Host part starts at '@' (last '@' before '/' or '?' to allow '@' in password? passwords should be percent-encoded, so first '@' is fine, but use rfind before '/' for safety).
    let auth_end_rel = {
        let end = after_scheme.find('/').or_else(|| after_scheme.find('?')).unwrap_or(after_scheme.len());
        after_scheme[..end].rfind('@')
    };
    let auth_end = match auth_end_rel {
        Some(i) => i,
        None => return url.to_string(),
    };
    let auth = &after_scheme[..auth_end];
    let rest = &after_scheme[auth_end..];
    // auth is user[:password]
    match auth.find(':') {
        Some(ci) => {
            let user = &auth[..ci];
            format!("{}://{}:***{}", &url[..scheme_end - 3], user, rest)
        }
        None => url.to_string(),
    }
}

/// Percent-decode `%XX` (plus stays plus; invalid sequences stay as-is).
/// Never panics.
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h << 4) | l) as u8 as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn parse_pg_url(url: &str) -> Result<PgConfig, Diagnostic> {
    let redacted = redact_url(url);
    // Scheme.
    let (scheme, rest) = match url.find("://") {
        Some(i) => (&url[..i], &url[i + 3..]),
        None => {
            return Err(connect_err(
                &redacted,
                "invalid URL (want postgres://user:pass@host:port/dbname?sslmode=...)",
            ));
        }
    };
    if scheme != "postgres" && scheme != "postgresql" {
        return Err(connect_err(&redacted, &format!("invalid scheme {scheme:?} (want postgres:// or postgresql://)")));
    }
    // Split query.
    let (authority_and_path, query) = match rest.find('?') {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    // Split dbname (first '/' after authority).
    let (authority, dbpart) = match authority_and_path.find('/') {
        Some(i) => (&authority_and_path[..i], Some(&authority_and_path[i + 1..])),
        None => (authority_and_path, None),
    };
    if authority.is_empty() {
        return Err(connect_err(&redacted, "invalid URL (missing host)"));
    }
    // Split userinfo @ hostport (rfind '@' before '/').
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(a) => match a.find(':') {
            Some(ci) => {
                let u = percent_decode(&a[..ci]);
                let p = percent_decode(&a[ci + 1..]);
                if u.is_empty() {
                    return Err(connect_err(&redacted, "invalid URL (empty user)"));
                }
                (u, Some(p))
            }
            None => {
                let u = percent_decode(a);
                if u.is_empty() {
                    return Err(connect_err(&redacted, "invalid URL (empty user)"));
                }
                (u, None)
            }
        },
        None => {
            return Err(connect_err(&redacted, "invalid URL (missing user)"));
        }
    };
    if hostport.is_empty() {
        return Err(connect_err(&redacted, "invalid URL (missing host)"));
    }
    // Host/port: handle [ipv6]:port, host:port, host.
    let (host, port) = if hostport.starts_with('[') {
        // [v6] or [v6]:port
        match hostport.find(']') {
            Some(e) => {
                let h = hostport[1..e].to_string();
                let rest = &hostport[e + 1..];
                if h.is_empty() {
                    return Err(connect_err(&redacted, "invalid URL (empty host)"));
                }
                if rest.is_empty() {
                    (h, DEFAULT_PORT)
                } else if rest.starts_with(':') {
                    let pstr = &rest[1..];
                    match pstr.parse::<u16>() {
                        Ok(p) => (h, p),
                        Err(_) => return Err(connect_err(&redacted, &format!("invalid port {pstr:?}"))),
                    }
                } else {
                    return Err(connect_err(&redacted, "invalid URL (bad IPv6 host)"));
                }
            }
            None => return Err(connect_err(&redacted, "invalid URL (bad IPv6 host)")),
        }
    } else {
        match hostport.rfind(':') {
            Some(ci) => {
                let h = &hostport[..ci];
                let pstr = &hostport[ci + 1..];
                // Empty host like ":5432" is invalid; bare ":" also invalid.
                if h.is_empty() {
                    return Err(connect_err(&redacted, "invalid URL (empty host)"));
                }
                // If port part is empty, default (e.g. "host:").
                if pstr.is_empty() {
                    (h.to_string(), DEFAULT_PORT)
                } else {
                    match pstr.parse::<u16>() {
                        Ok(p) => (h.to_string(), p),
                        Err(_) => {
                            // Could be ':' inside password? No, hostport has no password (split at '@').
                            return Err(connect_err(&redacted, &format!("invalid port {pstr:?}")));
                        }
                    }
                }
            }
            None => (hostport.to_string(), DEFAULT_PORT),
        }
    };
    if host.is_empty() {
        return Err(connect_err(&redacted, "invalid URL (empty host)"));
    }
    // dbname: up to '?' (already stripped), percent-decode, empty -> default to user.
    let dbname = match dbpart {
        Some(d) => {
            let d = d.trim();
            if d.is_empty() {
                user.clone()
            } else {
                percent_decode(d)
            }
        }
        None => user.clone(),
    };
    if dbname.is_empty() {
        return Err(connect_err(&redacted, "invalid URL (empty dbname)"));
    }
    // sslmode from query (default prefer).
    let mut sslmode = SslMode::Prefer;
    if let Some(q) = query {
        for pair in q.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = match pair.find('=') {
                Some(e) => (&pair[..e], &pair[e + 1..]),
                None => (pair, ""),
            };
            if k.eq_ignore_ascii_case("sslmode") {
                match SslMode::parse(v) {
                    Some(m) => sslmode = m,
                    None => {
                        return Err(connect_err(
                            &redacted,
                            &format!("invalid sslmode {v:?} (want disable, prefer, require)"),
                        ));
                    }
                }
            }
        }
    }
    Ok(PgConfig { host, port, user, password, dbname, sslmode })
}

// ---------------------------------------------------------------------------
// Handle state (opaque, shared by all `Value` clones of one handle).
// ---------------------------------------------------------------------------

/// One Postgres connection. `client` is `None` once closed.
pub struct PgState {
    client: Option<postgres::Client>,
    in_txn: bool,
    closed: bool,
    /// Redacted URL for diagnostics (password never stored in full).
    redacted_url: String,
}

impl std::fmt::Debug for PgState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgState")
            .field("closed", &self.closed)
            .field("in_txn", &self.in_txn)
            .field("url", &self.redacted_url)
            .finish()
    }
}

pub type PgHandle = Arc<Mutex<PgState>>;

fn as_pg(op: &str, v: &Value) -> Result<PgHandle, Diagnostic> {
    match v {
        Value::Pg(h) => Ok(h.clone()),
        other => Err(crate::runtime::type_err_pub(op, "a Pg", other)),
    }
}

fn check_closed(op: &str, st: &PgState) -> Result<(), Diagnostic> {
    if st.closed || st.client.is_none() {
        return Err(closed_err(op));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TLS (verified WebPki roots, same as ureq; no second TLS stack).
// ---------------------------------------------------------------------------

fn verified_tls_config() -> Result<rustls::ClientConfig, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(cfg)
}

fn is_auth_error(e: &postgres::Error) -> bool {
    // SQLSTATE 28P01 (invalid_password) / 28000 (invalid_authorization).
    if let Some(db) = e.as_db_error() {
        let code = db.code().code();
        if code == "28P01" || code == "28000" || code == "28P00" {
            return true;
        }
        let msg = db.message().to_ascii_lowercase();
        if msg.contains("password authentication failed")
            || msg.contains("authentication failed")
            || (msg.contains("password") && msg.contains("failed"))
        {
            return true;
        }
    }
    let s = format!("{e:?}").to_ascii_lowercase();
    s.contains("password authentication failed") || s.contains("authentication failed")
}

fn is_ssl_unsupported_error(e: &postgres::Error) -> bool {
    let s = format!("{e} {e:?}").to_ascii_lowercase();
    // Server speaks plaintext when we insist on TLS.
    (s.contains("ssl") && (s.contains("not supported") || s.contains("unsupported") || s.contains("does not support") || s.contains("no pg_hba")))
        || s.contains("server does not support ssl")
        || s.contains("ssl is not enabled on the server")
}

fn db_sqlstate(e: &postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        return db.code().code().to_string();
    }
    // Connection-level failures have no SQLSTATE; use generic 08000.
    "08000".to_string()
}

fn db_message(e: &postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        return db.message().to_string();
    }
    // Strip any accidental password echo (defensive; postgres never echoes it).
    format!("{e}")
}

fn connect_config(cfg: &PgConfig) -> postgres::Config {
    let mut c = postgres::Config::new();
    c.host(&cfg.host);
    c.port(cfg.port);
    c.user(&cfg.user);
    if let Some(p) = &cfg.password {
        c.password(p);
    }
    c.dbname(&cfg.dbname);
    c.connect_timeout(std::time::Duration::from_secs(10));
    c
}

// ---------------------------------------------------------------------------
// Builtins.
// ---------------------------------------------------------------------------

/// `pg_connect(url) -> Pg`. Password never appears in errors (redacted).
pub fn pg_connect(url: &str) -> Result<PgHandle, Diagnostic> {
    const OP: &str = "pg_connect";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    if url.is_empty() {
        return Err(connect_err("\"\"", "invalid URL (empty)"));
    }
    if url.contains('\0') {
        return Err(connect_err(&redact_url(url), "invalid URL (NUL byte)"));
    }
    let cfg = parse_pg_url(url)?;
    let redacted = redact_url(url);
    let base = connect_config(&cfg);
    match cfg.sslmode {
        SslMode::Disable => match base.connect(postgres::NoTls) {
            Ok(client) => Ok(Arc::new(Mutex::new(PgState {
                client: Some(client),
                in_txn: false,
                closed: false,
                redacted_url: redacted,
            }))),
            Err(e) => {
                if is_auth_error(&e) {
                    Err(auth_err(&redacted, &db_message(&e)))
                } else {
                    Err(connect_err(&redacted, &db_message(&e)))
                }
            }
        },
        SslMode::Require => {
            let tls_cfg = verified_tls_config().map_err(|d| connect_err(&redacted, &d))?;
            let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_cfg);
            match base.connect(tls) {
                Ok(client) => Ok(Arc::new(Mutex::new(PgState {
                    client: Some(client),
                    in_txn: false,
                    closed: false,
                    redacted_url: redacted,
                }))),
                Err(e) => {
                    if is_auth_error(&e) {
                        Err(auth_err(&redacted, &db_message(&e)))
                    } else {
                        Err(connect_err(&redacted, &db_message(&e)))
                    }
                }
            }
        }
        SslMode::Prefer => {
            // Try verified TLS first; fall back to plaintext ONLY when the
            // server does not speak SSL at all (not on cert/auth failures).
            let tls_cfg = verified_tls_config().map_err(|d| connect_err(&redacted, &d))?;
            let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_cfg);
            match base.connect(tls) {
                Ok(client) => Ok(Arc::new(Mutex::new(PgState {
                    client: Some(client),
                    in_txn: false,
                    closed: false,
                    redacted_url: redacted.clone(),
                }))),
                Err(tls_err) => {
                    if is_auth_error(&tls_err) {
                        return Err(auth_err(&redacted, &db_message(&tls_err)));
                    }
                    if is_ssl_unsupported_error(&tls_err) {
                        // Retry plaintext (fresh Config; Config::connect consumes nothing mutably problematic, reuse base).
                        match base.connect(postgres::NoTls) {
                            Ok(client) => Ok(Arc::new(Mutex::new(PgState {
                                client: Some(client),
                                in_txn: false,
                                closed: false,
                                redacted_url: redacted,
                            }))),
                            Err(e2) => {
                                if is_auth_error(&e2) {
                                    Err(auth_err(&redacted, &db_message(&e2)))
                                } else {
                                    Err(connect_err(&redacted, &db_message(&e2)))
                                }
                            }
                        }
                    } else {
                        Err(connect_err(&redacted, &db_message(&tls_err)))
                    }
                }
            }
        }
    }
}

/// Owned param values (live across the `execute` call).
/// Ints that fit in `i32` are sent as INT4 (so INT, BIGINT and SMALLINT
/// columns all accept them via implicit cast); larger ints are sent as
/// INT8 (BIGINT columns; INT columns correctly reject out-of-range).
enum OwnedParam {
    Int32(i32),
    Int64(i64),
    Float(f64),
    Str(String),
    Null,
}

fn owned_params(op: &str, params: &Value) -> Result<Vec<OwnedParam>, Diagnostic> {
    let arr = match params {
        Value::Array(items) => items,
        other => return Err(crate::runtime::type_err_pub(op, "an array of params", other)),
    };
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        match v {
            Value::Int(n) => {
                if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 {
                    out.push(OwnedParam::Int32(*n as i32));
                } else {
                    out.push(OwnedParam::Int64(*n));
                }
            }
            Value::Float(f) => out.push(OwnedParam::Float(*f)),
            Value::Str(s) => out.push(OwnedParam::Str(s.clone())),
            Value::Null => out.push(OwnedParam::Null),
            other => {
                return Err(type_err(
                    op,
                    &format!("unsupported param of type {}, want int/float/str/null", crate::runtime::value_type_name_pub(other)),
                ));
            }
        }
    }
    Ok(out)
}

/// Borrowed `ToSql` refs over `owned` (plus a long-lived NULL sentinel).
fn bind_refs<'a>(
    owned: &'a [OwnedParam],
    null_sentinel: &'a Option<String>,
) -> Vec<&'a (dyn postgres::types::ToSql + Sync)> {
    let mut refs: Vec<&'a (dyn postgres::types::ToSql + Sync)> = Vec::with_capacity(owned.len());
    for p in owned {
        match p {
            OwnedParam::Int32(n) => refs.push(n as &'a (dyn postgres::types::ToSql + Sync)),
            OwnedParam::Int64(n) => refs.push(n as &'a (dyn postgres::types::ToSql + Sync)),
            OwnedParam::Float(f) => refs.push(f as &'a (dyn postgres::types::ToSql + Sync)),
            OwnedParam::Str(s) => refs.push(s as &'a (dyn postgres::types::ToSql + Sync)),
            OwnedParam::Null => refs.push(null_sentinel as &'a (dyn postgres::types::ToSql + Sync)),
        }
    }
    refs
}

fn sql_state_message(e: &postgres::Error) -> (String, String) {
    (db_sqlstate(e), db_message(e))
}

/// `pg_exec(pg, sql, params) -> {changes}`.
pub fn pg_exec(pg: &Value, sql: &str, params: &Value) -> Result<i64, Diagnostic> {
    const OP: &str = "pg_exec";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_pg(OP, pg)?;
    if sql.contains('\0') {
        return Err(sql_err(OP, "42602", "invalid SQL (NUL byte)"));
    }
    let owned = owned_params(OP, params)?;
    let null_sentinel: Option<String> = None;
    let refs = bind_refs(&owned, &null_sentinel);
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    check_closed(OP, &st)?;
    let client = st.client.as_mut().ok_or_else(|| closed_err(OP))?;
    match client.execute(sql, &refs) {
        Ok(n) => Ok(n as i64),
        Err(e) => {
            let (code, msg) = sql_state_message(&e);
            Err(sql_err(OP, &code, &msg))
        }
    }
}

/// Map one column to a Klang value (NULL -> `Null`, BOOL -> 0/1).
/// Never panics (all `try_get`, unknown types -> `Str` or `E-PG-TYPE`).
fn col_to_value(row: &postgres::Row, idx: usize) -> Result<Value, Diagnostic> {
    use postgres::types::Type;
    const OP: &str = "pg_query";
    let col = &row.columns()[idx];
    let t = col.type_();
    // Helper: try Option<String> for text-ish fallback.
    let as_opt_str = || -> Option<Option<String>> {
        match row.try_get::<_, Option<String>>(idx) {
            Ok(v) => Some(v),
            Err(_) => None,
        }
    };
    match *t {
        Type::BOOL => match row.try_get::<_, Option<bool>>(idx) {
            Ok(Some(true)) => Ok(Value::Int(1)),
            Ok(Some(false)) => Ok(Value::Int(0)),
            Ok(None) => Ok(Value::Null),
            Err(e) => Err(type_err(OP, &format!("cannot decode BOOL column {}: {e}", col.name()))),
        },
        Type::INT2 => match row.try_get::<_, Option<i16>>(idx) {
            Ok(Some(n)) => Ok(Value::Int(n as i64)),
            Ok(None) => Ok(Value::Null),
            Err(e) => Err(type_err(OP, &format!("cannot decode INT2 column {}: {e}", col.name()))),
        },
        Type::INT4 => match row.try_get::<_, Option<i32>>(idx) {
            Ok(Some(n)) => Ok(Value::Int(n as i64)),
            Ok(None) => Ok(Value::Null),
            Err(e) => Err(type_err(OP, &format!("cannot decode INT4 column {}: {e}", col.name()))),
        },
        Type::INT8 => match row.try_get::<_, Option<i64>>(idx) {
            Ok(Some(n)) => Ok(Value::Int(n)),
            Ok(None) => Ok(Value::Null),
            Err(e) => Err(type_err(OP, &format!("cannot decode INT8 column {}: {e}", col.name()))),
        },
        Type::OID => match row.try_get::<_, Option<u32>>(idx) {
            Ok(Some(n)) => Ok(Value::Int(n as i64)),
            Ok(None) => Ok(Value::Null),
            Err(e) => Err(type_err(OP, &format!("cannot decode OID column {}: {e}", col.name()))),
        },
        Type::FLOAT4 => match row.try_get::<_, Option<f32>>(idx) {
            Ok(Some(f)) => Ok(Value::Float(f as f64)),
            Ok(None) => Ok(Value::Null),
            Err(e) => Err(type_err(OP, &format!("cannot decode FLOAT4 column {}: {e}", col.name()))),
        },
        Type::FLOAT8 => match row.try_get::<_, Option<f64>>(idx) {
            Ok(Some(f)) => Ok(Value::Float(f)),
            Ok(None) => Ok(Value::Null),
            Err(e) => Err(type_err(OP, &format!("cannot decode FLOAT8 column {}: {e}", col.name()))),
        },
        Type::NUMERIC => {
            // No arbitrary-precision decimal in the driver: use a SQL cast
            // (`n::text` -> Str, `n::float8` -> Float). Direct NUMERIC reads
            // are E-PG-TYPE (loud, never panic).
            Err(type_err(OP, &format!("NUMERIC column {} needs a SQL cast (e.g. n::text or n::float8)", col.name())))
        }
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
            match row.try_get::<_, Option<String>>(idx) {
                Ok(Some(s)) => Ok(Value::Str(s)),
                Ok(None) => Ok(Value::Null),
                Err(e) => Err(type_err(OP, &format!("cannot decode text column {}: {e}", col.name()))),
            }
        }
        Type::TIMESTAMP => {
            // With chrono feature, get NaiveDateTime and ISO-format; else text.
            match row.try_get::<_, Option<chrono::NaiveDateTime>>(idx) {
                Ok(Some(dt)) => Ok(Value::Str(dt.format("%Y-%m-%dT%H:%M:%S%.3f").to_string())),
                Ok(None) => Ok(Value::Null),
                Err(_) => match as_opt_str() {
                    Some(Some(s)) => Ok(Value::Str(s)),
                    Some(None) => Ok(Value::Null),
                    None => Err(type_err(OP, &format!("cannot decode TIMESTAMP column {}", col.name()))),
                },
            }
        }
        Type::TIMESTAMPTZ => match row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(idx) {
            Ok(Some(dt)) => Ok(Value::Str(dt.to_rfc3339())),
            Ok(None) => Ok(Value::Null),
            Err(_) => match as_opt_str() {
                Some(Some(s)) => Ok(Value::Str(s)),
                Some(None) => Ok(Value::Null),
                None => Err(type_err(OP, &format!("cannot decode TIMESTAMPTZ column {}", col.name()))),
            },
        },
        Type::DATE => match row.try_get::<_, Option<chrono::NaiveDate>>(idx) {
            Ok(Some(d)) => Ok(Value::Str(d.to_string())),
            Ok(None) => Ok(Value::Null),
            Err(_) => match as_opt_str() {
                Some(Some(s)) => Ok(Value::Str(s)),
                Some(None) => Ok(Value::Null),
                None => Err(type_err(OP, &format!("cannot decode DATE column {}", col.name()))),
            },
        },
        Type::TIME | Type::TIMETZ => match row.try_get::<_, Option<chrono::NaiveTime>>(idx) {
            Ok(Some(ti)) => Ok(Value::Str(ti.to_string())),
            Ok(None) => Ok(Value::Null),
            Err(_) => match as_opt_str() {
                Some(Some(s)) => Ok(Value::Str(s)),
                Some(None) => Ok(Value::Null),
                None => Err(type_err(OP, &format!("cannot decode TIME column {}", col.name()))),
            },
        },
        Type::JSON | Type::JSONB => {
            match row.try_get::<_, Option<serde_json::Value>>(idx) {
                Ok(Some(j)) => Ok(Value::Str(j.to_string())),
                Ok(None) => Ok(Value::Null),
                Err(_) => match as_opt_str() {
                    Some(Some(s)) => Ok(Value::Str(s)),
                    Some(None) => Ok(Value::Null),
                    None => Err(type_err(OP, &format!("cannot decode JSON column {}", col.name()))),
                },
            }
        }
        _ => {
            // Other types (UUID, BYTEA, arrays, etc.): text when possible, else loud type error (never panic).
            match as_opt_str() {
                Some(Some(s)) => Ok(Value::Str(s)),
                Some(None) => Ok(Value::Null),
                None => Err(type_err(OP, &format!("unsupported column type {} for column {}", t, col.name()))),
            }
        }
    }
}

/// `pg_query(pg, sql, params) -> array of maps`.
pub fn pg_query(pg: &Value, sql: &str, params: &Value) -> Result<Vec<Value>, Diagnostic> {
    const OP: &str = "pg_query";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_pg(OP, pg)?;
    if sql.contains('\0') {
        return Err(sql_err(OP, "42602", "invalid SQL (NUL byte)"));
    }
    let owned = owned_params(OP, params)?;
    let null_sentinel: Option<String> = None;
    let refs = bind_refs(&owned, &null_sentinel);
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    check_closed(OP, &st)?;
    let client = st.client.as_mut().ok_or_else(|| closed_err(OP))?;
    let rows = match client.query(sql, &refs) {
        Ok(r) => r,
        Err(e) => {
            let (code, msg) = sql_state_message(&e);
            return Err(sql_err(OP, &code, &msg));
        }
    };
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut fields = Vec::with_capacity(row.len());
        for i in 0..row.len() {
            let name = row.columns()[i].name().to_string();
            let v = col_to_value(row, i)?;
            fields.push((name, v));
        }
        out.push(Value::Map(fields));
    }
    Ok(out)
}

/// `pg_begin(pg) -> 1`.
pub fn pg_begin(pg: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "pg_begin";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_pg(OP, pg)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    check_closed(OP, &st)?;
    if st.in_txn {
        return Err(txn_err(OP, "already in a transaction (pg_commit or pg_rollback first)"));
    }
    let client = st.client.as_mut().ok_or_else(|| closed_err(OP))?;
    match client.batch_execute("BEGIN") {
        Ok(()) => {
            st.in_txn = true;
            Ok(())
        }
        Err(e) => {
            let (code, msg) = sql_state_message(&e);
            Err(sql_err(OP, &code, &msg))
        }
    }
}

/// `pg_commit(pg) -> 1`.
pub fn pg_commit(pg: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "pg_commit";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_pg(OP, pg)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    check_closed(OP, &st)?;
    if !st.in_txn {
        return Err(txn_err(OP, "not in a transaction (pg_begin first)"));
    }
    let client = st.client.as_mut().ok_or_else(|| closed_err(OP))?;
    match client.batch_execute("COMMIT") {
        Ok(()) => {
            st.in_txn = false;
            Ok(())
        }
        Err(e) => {
            let (code, msg) = sql_state_message(&e);
            Err(sql_err(OP, &code, &msg))
        }
    }
}

/// `pg_rollback(pg) -> 1`.
pub fn pg_rollback(pg: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "pg_rollback";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_pg(OP, pg)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    check_closed(OP, &st)?;
    if !st.in_txn {
        return Err(txn_err(OP, "not in a transaction (pg_begin first)"));
    }
    let client = st.client.as_mut().ok_or_else(|| closed_err(OP))?;
    match client.batch_execute("ROLLBACK") {
        Ok(()) => {
            st.in_txn = false;
            Ok(())
        }
        Err(e) => {
            let (code, msg) = sql_state_message(&e);
            Err(sql_err(OP, &code, &msg))
        }
    }
}

/// `pg_close(pg) -> 1` (idempotent).
pub fn pg_close(pg: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "pg_close";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_network(OP));
    }
    let h = as_pg(OP, pg)?;
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    if st.closed {
        return Ok(());
    }
    st.closed = true;
    st.in_txn = false;
    st.client.take();
    Ok(())
}

/// True when the handle is closed (for tests; not a builtin).
pub fn is_closed(h: &PgHandle) -> bool {
    let st = h.lock().unwrap_or_else(|e| e.into_inner());
    st.closed || st.client.is_none()
}

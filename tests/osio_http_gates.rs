//! STDLIB-NET Phase 2 — HTTP client gates.
//!
//! Covers `http_get(url)` / `http_post(url, body, headers)` per the
//! PRD: real requests with real status/body/header assertions (not
//! just "didn't crash"), HTTP error statuses as normal results,
//! `E-NET-UNREACHABLE` on refused connections, `E-NET-INVALID-URL`
//! on malformed URLs, `E-NET-INVALID-HEADER` on bad header entries,
//! code specificity, and builtin arity/type checking.
//!
//! Two network tiers:
//! - Hermetic loopback server (std `TcpListener`, ephemeral port):
//!   GET status/body/headers, POST body+header round-trip, 404 as a
//!   result, refused-connection unreachable. No external dependency,
//!   real TCP + HTTP/1.1 wire I/O through the real client.
//! - Public endpoint (https://httpbin.org, the canonical httpbin-style
//!   service — reachable in 0.3s from this sandbox, stable for over a
//!   decade, and the service ureq's own docs use in examples): GET
//!   with a real body-content assertion, POST with body + custom
//!   header echoed back and asserted. THESE TESTS NEED OUTBOUND
//!   INTERNET: anyone running `cargo test` (dev machines, any future
//!   test workflow) must have it. Note the repo currently has no
//!   test-running CI job (`release.yml` only builds binaries on tags),
//!   so this changes no existing CI behavior — but the dependency is
//!   real and stated here, not assumed.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new())
}

// ---- minimal loopback HTTP/1.1 server (test helper, std only) ----

/// Serve exactly `expect` requests on 127.0.0.1, then exit. The 20s
/// deadline only ever triggers when the client under test fails to
/// connect — tests fail, never hang.
fn start_server(expect: usize) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    listener.set_nonblocking(true).expect("nonblocking");
    let handle = thread::spawn(move || {
        let mut served = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while served < expect && std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((s, _)) => {
                    handle_conn(s);
                    served += 1;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    (format!("http://127.0.0.1:{port}"), handle)
}

/// A port that was just bound and released: connecting to it is a
/// guaranteed ECONNREFUSED with no external dependency.
fn refused_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let p = l.local_addr().expect("addr").port();
    drop(l);
    p
}

fn respond(mut s: std::net::TcpStream, status: u16, reason: &str, extra: &[(&str, &str)], body: &[u8]) {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
    let _ = s.flush();
}

fn handle_conn(s: std::net::TcpStream) {
    let mut reader = BufReader::new(s.try_clone().expect("clone stream"));
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        if line.trim_end().is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_lowercase(), v.trim().to_string()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    // Answer 100-continue before reading the body when asked.
    let writer = s.try_clone().expect("clone writer");
    if header("expect") == "100-continue" {
        let _ = (&writer).write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    let len: usize = header("content-length").parse().unwrap_or(0);
    let mut body = vec![0u8; len.min(1 << 20)];
    if len > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }
    match (method.as_str(), path.as_str()) {
        ("GET", "/ok") => respond(
            writer,
            200,
            "OK",
            &[("Content-Type", "text/plain"), ("X-Echo-Test", "yes")],
            b"hello http",
        ),
        ("POST", "/echo") => {
            let custom = header("x-custom");
            let colon = header("x-colon");
            respond(
                writer,
                200,
                "OK",
                &[
                    ("Content-Type", "text/plain"),
                    ("X-Got-Custom", &custom),
                    ("X-Got-Colon", &colon),
                ],
                &body,
            )
        }
        _ => respond(writer, 404, "Not Found", &[("Content-Type", "text/plain")], b"nope"),
    }
}

// ---- hermetic loopback tests (real client, real wire, no internet) ----

#[test]
fn osio_http_local_get_status_body_headers() {
    let (base, handle) = start_server(1);
    let src = format!(
        "fn main() -> i32 {{ let r = http_get(\"{base}/ok\") print(r[\"status\"]) print(r[\"body\"]) print(r[\"headers\"][\"x-echo-test\"]) print(r[\"headers\"][\"content-type\"]) if r[\"status\"] != 200 {{ return 1 }} if r[\"body\"] != \"hello http\" {{ return 2 }} if r[\"headers\"][\"x-echo-test\"] != \"yes\" {{ return 3 }} return 42 }}"
    );
    let (v, out) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 42, "GET assertions, prints: {out:?}");
    assert_eq!(out, vec!["200", "hello http", "yes", "text/plain"]);
}

#[test]
fn osio_http_local_post_echoes_body_and_headers() {
    // The server mirrors the POST body and the two custom headers, so
    // every assertion below pins bytes that crossed the wire twice.
    // `X-Colon: a:b:c` also proves values may contain colons (split
    // is on the FIRST colon only).
    let (base, handle) = start_server(1);
    let src = format!(
        "fn main() -> i32 {{ let r = http_post(\"{base}/echo\", \"ping-body-789\", [\"X-Custom: abc-123\", \"X-Colon: a:b:c\", \"Content-Type: text/plain\"]) print(r[\"status\"]) print(r[\"body\"]) print(r[\"headers\"][\"x-got-custom\"]) print(r[\"headers\"][\"x-got-colon\"]) if r[\"status\"] != 200 {{ return 1 }} if r[\"body\"] != \"ping-body-789\" {{ return 2 }} if r[\"headers\"][\"x-got-custom\"] != \"abc-123\" {{ return 3 }} if r[\"headers\"][\"x-got-colon\"] != \"a:b:c\" {{ return 4 }} return 42 }}"
    );
    let (v, out) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 42, "POST assertions, prints: {out:?}");
    assert_eq!(out, vec!["200", "ping-body-789", "abc-123", "a:b:c"]);
}

#[test]
fn osio_http_status_404_is_result_not_error() {
    // Design decision (http_status_as_error=false): HTTP error
    // statuses populate the map, they do NOT raise — parallel to
    // Phase 2's non-zero-exit and Phase 3's non-match decisions.
    let (base, handle) = start_server(1);
    let src = format!(
        "fn main() -> i32 {{ let r = http_get(\"{base}/missing\") print(r[\"status\"]) print(r[\"body\"]) if r[\"status\"] != 404 {{ return 1 }} if r[\"body\"] != \"nope\" {{ return 2 }} return 42 }}"
    );
    let (v, out) = run_src(&src, "main").expect("404 must still be Ok");
    handle.join().expect("server thread");
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["404", "nope"]);
}

#[test]
fn osio_http_refused_is_unreachable() {
    let port = refused_port();
    let src = format!("fn main() -> i32 {{ let r = http_get(\"http://127.0.0.1:{port}/\") return r[\"status\"] }}");
    let err = run_src(&src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-NET-UNREACHABLE", "got: {}", err.to_json());
    assert!(
        err.to_json().contains("refus"),
        "real OS cause, not generic: {}",
        err.to_json()
    );
}

#[test]
fn osio_http_malformed_url_is_invalid_url() {
    let src = "fn main() -> i32 { let r = http_get(\"not a url at all\") return r[\"status\"] }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-NET-INVALID-URL", "got: {}", err.to_json());
    assert!(
        err.to_json().contains("invalid uri character"),
        "real parse complaint, not generic: {}",
        err.to_json()
    );
    let src = "fn main() -> i32 { let r = http_get(\"\") return r[\"status\"] }";
    let err = run_src(src, "main").expect_err("empty URL must fail");
    assert_eq!(err.code, "E-NET-INVALID-URL", "got: {}", err.to_json());
    assert!(
        err.to_json().contains("empty string"),
        "real parse complaint, not generic: {}",
        err.to_json()
    );
}

#[test]
fn osio_http_bad_header_is_invalid_header() {
    // Header validation happens before any connection, so these URLs
    // are never touched — the code proves the failure is in the
    // header, not the network.
    let src = "fn main() -> i32 { let r = http_post(\"http://127.0.0.1:9/echo\", \"x\", [\"no-colon-here\"]) return r[\"status\"] }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-NET-INVALID-HEADER", "got: {}", err.to_json());
    assert!(
        err.to_json().contains("colon"),
        "specific cause, not generic: {}",
        err.to_json()
    );
    let src = "fn main() -> i32 { let r = http_post(\"http://127.0.0.1:9/echo\", \"x\", [\": empty-name\"]) return r[\"status\"] }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-NET-INVALID-HEADER", "got: {}", err.to_json());
}

// ---- public endpoint tests (NEED OUTBOUND INTERNET, see header) ----

#[test]
fn osio_http_public_get_httpbin() {
    // https://httpbin.org/get answers 200 with a JSON body naming the
    // request URL — both asserted, not just "didn't crash".
    let src = "fn main() -> i32 { let r = http_get(\"https://httpbin.org/get\") print(r[\"status\"]) if r[\"status\"] != 200 { return 1 } if r[\"body\"].contains(\"httpbin.org\") { return 42 } return 2 }";
    let (v, out) = run_src(src, "main").expect("runs against httpbin");
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["200".to_string()]);
}

#[test]
fn osio_http_public_post_httpbin() {
    // https://httpbin.org/post echoes the raw body in "data" and the
    // request headers back — both asserted on real content.
    let src = "fn main() -> i32 { let r = http_post(\"https://httpbin.org/post\", \"klang-post-probe-456\", [\"Content-Type: text/plain\", \"X-Klang-Probe: header-val-789\"]) print(r[\"status\"]) if r[\"status\"] != 200 { return 1 } if r[\"body\"].contains(\"klang-post-probe-456\") { if r[\"body\"].contains(\"header-val-789\") { return 42 } return 2 } return 3 }";
    let (v, out) = run_src(src, "main").expect("runs against httpbin");
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["200".to_string()]);
}

#[test]
fn osio_http_error_codes_are_specific() {
    // End-to-end through the public mapping: each failure class gets
    // its own code with its own real cause (never one shared generic
    // string).
    let bad = klang::stdlib::http::get("not a url").expect_err("bad URL must fail");
    assert_eq!(bad.code, "E-NET-INVALID-URL");
    let port = refused_port();
    let down = klang::stdlib::http::get(&format!("http://127.0.0.1:{port}/"))
        .expect_err("refused must fail");
    assert_eq!(down.code, "E-NET-UNREACHABLE");
    assert!(
        down.to_json().contains("refus"),
        "real OS cause: {}",
        down.to_json()
    );
    let hdr = klang::stdlib::http::post("http://127.0.0.1:9/x", "b", &["oops".to_string()])
        .expect_err("bad header must fail");
    assert_eq!(hdr.code, "E-NET-INVALID-HEADER");
    // Distinct codes AND distinct causes across all three classes.
    assert_ne!(bad.code, down.code);
    assert_ne!(down.code, hdr.code);
    assert_ne!(bad.cause, down.cause);
    assert_ne!(down.cause, hdr.cause);
    // A healthy loopback round-trip is Ok with a real status number.
    let (base, handle) = start_server(1);
    let r = klang::stdlib::http::get(&format!("{base}/ok")).expect("loopback GET works");
    handle.join().expect("server thread");
    assert_eq!(r.status, 200);
    assert_eq!(r.body, "hello http");
    assert!(
        r.headers.iter().any(|(k, v)| k == "x-echo-test" && v == "yes"),
        "response headers mapped, got {:?}",
        r.headers
    );
}

#[test]
fn osio_http_arg_types_checked() {
    let mut p = Parser::new("fn main() -> i32 { let r = http_get(42) return r[\"status\"] }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { let r = http_get() return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
    let mut p = Parser::new("fn main() -> i32 { let r = http_post(\"u\", \"b\") return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-ARITY"));
    let mut p = Parser::new("fn main() -> i32 { let r = http_post(\"u\", \"b\", \"notarray\") return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
    let mut p = Parser::new("fn main() -> i32 { let r = http_post(\"u\", 1, []) return 0 }");
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE"));
}

#[test]
fn osio_http_header_errors_never_echo_values() {
    // SECAUDIT-1: diagnostics persist in harness logs by design, so a
    // secret-bearing header must never render into one. The name stays
    // (debuggable); the value goes.
    let src = "fn main() -> i32 { let r = http_post(\"http://127.0.0.1:9/echo\", \"x\", [\"Bearer s3cret-token-xyz\"]) return r[\"status\"] }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-NET-INVALID-HEADER", "got: {}", err.to_json());
    assert!(
        !err.to_json().contains("s3cret-token-xyz"),
        "secret must not render: {}",
        err.to_json()
    );
    // A malformed *named* header keeps the name but drops the value.
    let src = "fn main() -> i32 { let r = http_post(\"http://127.0.0.1:9/echo\", \"x\", [\"X-Bad: a\nb-s3cret\"]) return r[\"status\"] }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-NET-INVALID-HEADER", "got: {}", err.to_json());
    assert!(err.to_json().contains("X-Bad"), "name kept: {}", err.to_json());
    assert!(
        !err.to_json().contains("s3cret"),
        "value dropped: {}",
        err.to_json()
    );
}

#[test]
fn osio_http_url_userinfo_redacted() {
    // SECAUDIT-1: `user:pass@` in a failing URL renders as `***@`;
    // host and path stay for debugging.
    let src = "fn main() -> i32 { let r = http_get(\"https://user:s3cret-pass@not a host/\") return r[\"status\"] }";
    let err = run_src(src, "main").expect_err("must fail");
    assert_eq!(err.code, "E-NET-INVALID-URL", "got: {}", err.to_json());
    assert!(
        !err.to_json().contains("s3cret-pass"),
        "password must not render: {}",
        err.to_json()
    );
    assert!(err.to_json().contains("***@"), "redaction marked: {}", err.to_json());
    assert!(err.to_json().contains("not a host"), "host kept: {}", err.to_json());
}

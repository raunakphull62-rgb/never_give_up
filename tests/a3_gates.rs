//! A3 HTTP server + fuller client gates (Wave 2 Stage A3).
//!
//! Loopback only (`127.0.0.1`, port 0), no real internet. Deterministic,
//! no fixed `/tmp` names (unique temp dirs per test), no leaked threads,
//! sockets, servers or processes (every Klang program stops its server;
//! Rust handles drop on scope exit; program-exit reaps forgotten servers).
//! Process-global state (sandbox flag, signal flags, live-thread count)
//! serializes on `A3_LOCK`.

use std::collections::HashMap;
use std::process::Stdio;

use klang::parser::Parser;

static A3_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn parse_ok(src: &str) -> klang::ast::Program {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    prog
}

fn run_ok(src: &str) -> (i32, Vec<String>) {
    let _guard = A3_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    klang::stdlib::a2_threads::test_clear();
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(
        klang::stdlib::a2_threads::active_count(),
        0,
        "no leaked threads after run: {src}"
    );
    klang::sandbox::set_sandbox(false);
    klang::stdlib::a2_threads::test_clear();
    r
}

fn run_err(src: &str) -> klang::diagnostics::Diagnostic {
    let _guard = A3_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    klang::stdlib::a2_threads::test_clear();
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let e = klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new())
        .expect_err("must fail");
    assert_eq!(
        klang::stdlib::a2_threads::active_count(),
        0,
        "no leaked threads after failure: {src}"
    );
    klang::sandbox::set_sandbox(false);
    klang::stdlib::a2_threads::test_clear();
    e
}

fn check_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&want), "want {want} in {codes:?}: {src}");
}

fn run_sandboxed(src: &str) -> Result<(i32, Vec<String>), String> {
    let _guard = A3_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(true);
    klang::stdlib::a2_threads::test_clear();
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    klang::sandbox::set_sandbox(false);
    klang::stdlib::a2_threads::test_clear();
    match r {
        Ok(v) => Ok(v),
        Err(d) => Err(format!("{} {}", d.code, d.to_json())),
    }
}

fn mktemp_dir(prefix: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    p.push(format!(
        "a3_{}_{}_{}",
        prefix,
        std::process::id(),
        nanos % 1_000_000_000
    ));
    std::fs::create_dir_all(&p).expect("mktemp");
    p
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

fn refused_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let p = l.local_addr().expect("addr").port();
    drop(l);
    p
}

// ---- happy paths ----

#[test]
fn a3_serve_basic_get() {
    let src = "fn h(req: map) -> map { return http_text(200, \"hi\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) print(p > 0) let r = http_request(\"GET\", \"http://127.0.0.1:\" + str(p) + \"/\", {}, \"\", 5000) print(r[\"status\"]) print(r[\"body\"]) print(r[\"headers\"][\"content-type\"]) http_stop(s) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "200", "hi", "text/plain; charset=utf-8"]);
}

#[test]
fn a3_query_headers_remote_parsing() {
    let src = "fn h(req: map) -> map { print(req[\"method\"]) print(req[\"path\"]) print(req[\"query\"][\"a\"]) print(req[\"query\"][\"b\"]) print(req[\"headers\"][\"x-test\"]) print(req[\"headers\"][\"content-type\"]) print(req[\"remote\"].contains(\":\")) print(req[\"body\"]) return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let r = http_request(\"POST\", \"http://127.0.0.1:\" + str(p) + \"/hi?a=1&b=hello%20world\", {\"X-Test\": \"abc\", \"Content-Type\": \"text/plain\"}, \"pay\", 5000) print(r[\"status\"]) http_stop(s) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    // method, path, query a/b, x-test (lower-case), content-type, remote has colon, body, status.
    assert_eq!(out, vec!["POST", "/hi", "1", "hello world", "abc", "text/plain", "1", "pay", "200"]);
}

#[test]
fn a3_keepalive_reuse_raw() {
    // Two sequential GETs on one raw TCP conn (keep-alive then close).
    // Body reads loop until the exact 5 bytes: `conn_read` is one syscall
    // and may short-read under load (same A1 limit as above).
    let src = "fn h(req: map) -> map { return http_text(200, \"ok-\" + req[\"path\"]) } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let c = tcp_connect(\"127.0.0.1\", p, 5000) conn_set_timeout(c, 5000) conn_write(c, \"GET /a HTTP/1.1\\r\\nHost: x\\r\\nConnection: keep-alive\\r\\n\\r\\n\") print(conn_read_line(c)) let hl = conn_read_line(c) while hl != \"\" { hl = conn_read_line(c) } let b1 = \"\" while len(b1) < 5 { let pc = conn_read(c, 5 - len(b1)) if pc == \"\" { break } b1 = b1 + pc } print(b1) conn_write(c, \"GET /b HTTP/1.1\\r\\nHost: x\\r\\nConnection: close\\r\\n\\r\\n\") print(conn_read_line(c)) let h2 = conn_read_line(c) while h2 != \"\" { h2 = conn_read_line(c) } let b2 = \"\" while len(b2) < 5 { let pc = conn_read(c, 5 - len(b2)) if pc == \"\" { break } b2 = b2 + pc } print(b2) conn_close(c) http_stop(s) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert!(out[0].contains("200"), "first status: {out:?}");
    assert_eq!(out[1], "ok-/a");
    assert!(out[2].contains("200"), "second status: {out:?}");
    assert_eq!(out[3], "ok-/b");
}

#[test]
fn a3_concurrent_200_requests() {
    // 20 workers x 10 requests via thread pool (200 total), all correct.
    let src = "fn h(req: map) -> map { return http_text(200, \"ok\") } fn worker(base: unknown, n: unknown, out: unknown) -> i32 { let i = 0 while i < 10 { let r = http_request(\"GET\", base + \"/\", {}, \"\", 8000) if r[\"status\"] != 200 { send(out, 0) return 0 } if r[\"body\"] != \"ok\" { send(out, 0) return 0 } i = i + 1 } send(out, 1) return 1 } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {\"workers\": 8}) let p = server_port(s) let base = \"http://127.0.0.1:\" + str(p) let ch = chan(30) let i = 0 while i < 20 { let t = thread_spawn(\"worker\", [base, i, ch]) i = i + 1 } let ok = 0 let k = 0 while k < 20 { ok = ok + recv(ch) k = k + 1 } close(ch) http_stop(s) print(ok) if ok == 20 { return 42 } return 1 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["20"]);
}

#[test]
fn a3_handler_error_500_and_alive() {
    let src = "fn h(req: map) -> map { if req[\"path\"] == \"/boom\" { let x = 1 / 0 return http_text(200, \"unreached\") } return http_text(200, \"fine\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let base = \"http://127.0.0.1:\" + str(p) let bad = http_request(\"GET\", base + \"/boom\", {}, \"\", 5000) print(bad[\"status\"]) print(bad[\"body\"]) let good = http_request(\"GET\", base + \"/ok\", {}, \"\", 5000) print(good[\"status\"]) print(good[\"body\"]) http_stop(s) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out[0], "500");
    assert!(out[1].contains("internal server error"), "safe body: {out:?}");
    assert_eq!(out[2], "200");
    assert_eq!(out[3], "fine");
}

#[test]
fn a3_malformed_400_raw() {
    let src = "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let c = tcp_connect(\"127.0.0.1\", p, 5000) conn_set_timeout(c, 5000) conn_write(c, \"GARBAGE-NOT-HTTP\\r\\n\\r\\n\") print(conn_read_line(c)) conn_close(c) http_stop(s) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert!(out[0].contains("400"), "want 400, got {out:?}");
}

#[test]
fn a3_oversize_413() {
    let src = "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {\"max_body_bytes\": 100}) let p = server_port(s) let big = \"x\".repeat(500) let r = http_request(\"POST\", \"http://127.0.0.1:\" + str(p) + \"/\", {}, big, 5000) print(r[\"status\"]) http_stop(s) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["413"]);
}

#[test]
fn a3_slow_client_408_raw() {
    // Server read timeout 120ms; client sends partial then sleeps 400ms.
    let src = "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {\"read_timeout_ms\": 120}) let p = server_port(s) let c = tcp_connect(\"127.0.0.1\", p, 5000) conn_set_timeout(c, 3000) conn_write(c, \"GET / HTTP/1.1\\r\\nHost: x\\r\\n\") sleep_ms(400) print(conn_read_line(c)) conn_close(c) http_stop(s) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert!(out[0].contains("408"), "want 408, got {out:?}");
}

#[test]
fn a3_graceful_stop_finishes_inflight() {
    // Handler sleeps 200ms; main stops while request in flight (stop waits).
    let src = "fn h(req: map) -> map { sleep_ms(200) return http_text(200, \"done\") } fn fetch(base: unknown, ch: unknown) -> i32 { let r = http_request(\"GET\", base + \"/\", {}, \"\", 5000) send(ch, r[\"body\"]) return 1 } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let ch = chan(2) let t = thread_spawn(\"fetch\", [\"http://127.0.0.1:\" + str(p), ch]) sleep_ms(50) http_stop(s) let b = recv(ch) print(b) thread_join(t) close(ch) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["done"]);
}

#[test]
fn a3_json_round_trip_null() {
    let src = "fn main() -> i32 { print(is_null(json_decode(\"null\"))) print(json_stringify(json_decode(\"null\"), 0)) let v = json_decode(\"{\\\"a\\\": null, \\\"b\\\": [1, null, \\\"x\\\"]}\") print(is_null(v[\"a\"])) print(is_null(v[\"b\"][1])) print(json_stringify(v, 0)) print(json_stringify({\"n\": 1}, 1).contains(\"\\n\")) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out[0], "1");
    assert_eq!(out[1], "null");
    assert_eq!(out[2], "1");
    assert_eq!(out[3], "1");
    assert!(out[4].contains("null"), "body: {out:?}");
    assert_eq!(out[5], "1");
}

#[test]
fn a3_json_1mb_under_1s() {
    // Native parser must handle 1 MB in under 1 second (pure-Klang cannot).
    let _guard = A3_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let big = format!("[{}]", (0..200000).map(|i| (i % 10).to_string()).collect::<Vec<_>>().join(","));
    assert!(big.len() > 400000, "big len {}", big.len());
    // Pad to ~1MB with a large string field.
    let pad = "x".repeat(600000);
    let doc = format!("{{\"arr\": {big}, \"pad\": \"{pad}\"}}");
    assert!(doc.len() > 1000000, "doc len {}", doc.len());
    let start = std::time::Instant::now();
    let v = klang::stdlib::a3_http::json_decode(&doc).expect("1MB parses");
    let dt = start.elapsed();
    assert!(dt.as_secs_f64() < 1.0, "1MB parse took {dt:?} (must be <1s)");
    // Null inside still maps to Null.
    match v {
        klang::runtime::Value::Map(_) => {}
        other => panic!("want map, got {other:?}"),
    }
    let s2 = klang::stdlib::a3_http::json_stringify(&v, 0).expect("stringifies");
    assert!(s2.len() > 1000000, "round trip len {}", s2.len());
    klang::sandbox::set_sandbox(false);
}

#[test]
fn a3_http_json_text_redirect_ctors() {
    let src = "fn main() -> i32 { let j = http_json(200, {\"a\": 1}) print(j[\"status\"]) print(j[\"headers\"][\"content-type\"]) print(j[\"body\"]) let t = http_text(404, \"nope\") print(t[\"status\"]) print(t[\"body\"]) let r = http_redirect(\"/target\") print(r[\"status\"]) print(r[\"headers\"][\"location\"]) let n = http_json(200, json_decode(\"null\")) print(n[\"body\"]) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["200", "application/json", "{\"a\":1}", "404", "nope", "302", "/target", "null"]);
}

#[test]
fn a3_route_match_params() {
    let src = "fn main() -> i32 { let m = route_match(\"/notes/:id\", \"/notes/42\") print(m[\"id\"]) print(is_null(route_match(\"/a/:x\", \"/b/c\"))) print(is_null(route_match(\"/notes/:id\", \"/notes\"))) let r = route_match(\"/\", \"/\") print(r.contains(\"x\") == false) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["42", "1", "1", "1"]);
}

#[test]
fn a3_redirect_follow_and_disable() {
    let src = "fn h(req: map) -> map { if req[\"path\"] == \"/r\" { return http_redirect(\"/target\") } return http_text(200, \"target-ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let base = \"http://127.0.0.1:\" + str(p) let a = http_request(\"GET\", base + \"/r\", {}, \"\", 5000) print(a[\"status\"]) print(a[\"body\"]) let b = http_request(\"GET\", base + \"/r\", {}, \"\", 5000, {\"follow_redirects\": 0}) print(b[\"status\"]) print(b[\"headers\"][\"location\"]) http_stop(s) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["200", "target-ok", "302", "/target"]);
}

#[test]
fn a3_hello_api_example() {
    // Full Render-pattern example: PORT/PORT_FILE env, 0.0.0.0 bind,
    // /health, /time, /echo, SIGTERM graceful shutdown within 5s.
    let dir = mktemp_dir("hello");
    let port_file = dir.join("port.txt");
    let mut child = std::process::Command::new(klang_bin())
        .args(["run", "examples/apps/hello_api/main.klang"])
        .current_dir(repo_root())
        .env("PORT", "0")
        .env("PORT_FILE", &port_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hello_api spawns");
    // Wait up to 10s for the port file (write_file is immediate).
    let mut waited = 0;
    let port: u16 = loop {
        if let Ok(txt) = std::fs::read_to_string(&port_file) {
            if let Ok(p) = txt.trim().parse::<u16>() {
                if p != 0 {
                    break p;
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        waited += 1;
        if waited > 100 {
            child.kill().ok();
            child.wait().ok();
            panic!("hello_api did not write PORT_FILE within 10s");
        }
        if let Some(st) = child.try_wait().expect("try_wait") {
            panic!("hello_api exited early with {st}");
        }
    };
    // Hit the three routes via raw HTTP (no extra deps).
    let get = |path: &str| -> (u16, String) {
        let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
        use std::io::{Read, Write};
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).expect("write");
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).ok();
        let txt = String::from_utf8_lossy(&buf).into_owned();
        let status = txt.lines().next().unwrap_or("").to_string();
        let code = status.split_whitespace().nth(1).unwrap_or("0").parse::<u16>().unwrap_or(0);
        (code, txt)
    };
    let (st, body) = get("/health");
    assert_eq!(st, 200, "health body: {body}");
    assert!(body.contains("\"ok\""), "health: {body}");
    let (st, body) = get("/time");
    assert_eq!(st, 200, "time: {body}");
    assert!(body.contains("time"), "time: {body}");
    // POST /echo with JSON null (Null <-> null preserved).
    let echo_body = "{\"hi\": 1, \"n\": null}";
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect echo");
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    use std::io::{Read, Write};
    s.write_all(format!("POST /echo HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", echo_body.len(), echo_body).as_bytes()).expect("write echo");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok();
    let txt = String::from_utf8_lossy(&buf).into_owned();
    assert!(txt.contains("200"), "echo status: {txt}");
    assert!(txt.contains("\"n\":null") || txt.contains("\"n\": null"), "echo null preserved: {txt}");
    // fmt must stay clean for new .klang files.
    let fmt = std::process::Command::new(klang_bin())
        .args(["fmt", "--check", "examples/apps/hello_api/main.klang"])
        .current_dir(repo_root())
        .output()
        .expect("fmt runs");
    assert!(fmt.status.success(), "fmt --check must pass: {}", String::from_utf8_lossy(&fmt.stderr));
    // SIGTERM -> clean exit 0 within 5s (Render-style).
    let pid = child.id();
    std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output().expect("kill runs");
    let mut waited = 0;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(st) => break st,
            None => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                waited += 1;
                if waited > 100 {
                    child.kill().ok();
                    child.wait().ok();
                    panic!("hello_api did not exit within 5s after SIGTERM");
                }
            }
        }
    };
    assert!(status.code() == Some(0), "hello_api must exit 0 after SIGTERM, got {status}");
    std::fs::remove_dir_all(&dir).ok();
}

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ---- error codes ----

#[test]
fn a3_err_http_url() {
    for src in [
        "fn main() -> i32 { let r = http_request(\"GET\", \"not a url\", {}, \"\", 1000) return 0 }",
        "fn main() -> i32 { let r = http_request(\"GET\", \"\", {}, \"\", 1000) return 0 }",
        "fn main() -> i32 { let r = http_request(\"GET\", \"ftp://x/y\", {}, \"\", 1000) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-HTTP-URL", "{src}: {}", err.to_json());
    }
}

#[test]
fn a3_err_http_method() {
    for src in [
        "fn main() -> i32 { let r = http_request(\"\", \"http://127.0.0.1/\", {}, \"\", 1000) return 0 }",
        "fn main() -> i32 { let r = http_request(\"BAD METHOD\", \"http://127.0.0.1/\", {}, \"\", 1000) return 0 }",
        "fn main() -> i32 { let r = http_request(\"get\", \"http://127.0.0.1/\", {}, \"\", 1000) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-HTTP-METHOD", "{src}: {}", err.to_json());
    }
}

#[test]
fn a3_err_http_header() {
    // Bad header name/value in the map, and non-str value.
    let err = run_err("fn main() -> i32 { let r = http_request(\"GET\", \"http://127.0.0.1/\", {\"bad name\": \"v\"}, \"\", 1000) return 0 }");
    assert_eq!(err.code, "E-HTTP-HEADER", "got: {}", err.to_json());
    let err = run_err("fn main() -> i32 { let m = {\"k\": 1} let r = http_request(\"GET\", \"http://127.0.0.1/\", {\"x\": m[\"k\"]}, \"\", 1000) return 0 }");
    // Dynamic non-str value (int 1) is also E-HTTP-HEADER (not E-TYPE).
    assert_eq!(err.code, "E-HTTP-HEADER", "got: {}", err.to_json());
}

#[test]
fn a3_err_http_timeout_invalid_and_expiry() {
    for src in [
        "fn main() -> i32 { let r = http_request(\"GET\", \"http://127.0.0.1/\", {}, \"\", 0 - 1) return 0 }",
        "fn main() -> i32 { let r = http_request(\"GET\", \"http://127.0.0.1/\", {}, \"\", 99999999) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-HTTP-TIMEOUT", "{src}: {}", err.to_json());
    }
    // Real expiry: handler sleeps 400ms, client waits 60ms.
    let src = "fn h(req: map) -> map { sleep_ms(400) return http_text(200, \"slow\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let r = http_request(\"GET\", \"http://127.0.0.1:\" + str(p) + \"/\", {}, \"\", 60) http_stop(s) return 0 }";
    let err = run_err(src);
    assert_eq!(err.code, "E-HTTP-TIMEOUT", "expiry: {}", err.to_json());
}

#[test]
fn a3_err_http_connect_refused() {
    let port = refused_port();
    let src = format!("fn main() -> i32 {{ let r = http_request(\"GET\", \"http://127.0.0.1:{port}/\", {{}}, \"\", 1500) return 0 }}");
    let err = run_err(&src);
    assert_eq!(err.code, "E-HTTP-CONNECT", "got: {}", err.to_json());
}

#[test]
fn a3_err_http_redirect_loop() {
    // Server redirects to itself; client follows 5 then errors.
    let src = "fn h(req: map) -> map { return http_redirect(\"/loop\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let r = http_request(\"GET\", \"http://127.0.0.1:\" + str(p) + \"/loop\", {}, \"\", 5000) http_stop(s) return 0 }";
    let err = run_err(src);
    assert_eq!(err.code, "E-HTTP-REDIRECT", "got: {}", err.to_json());
}

#[test]
fn a3_err_http_options() {
    for src in [
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {\"workers\": 99}) return 0 }",
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {\"nope\": 1}) return 0 }",
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) let r = http_request(\"GET\", \"http://127.0.0.1:\" + str(p) + \"/\", {}, \"\", 1000, {\"follow_redirects\": 7}) http_stop(s) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-HTTP-OPTIONS", "{src}: {}", err.to_json());
    }
}

#[test]
fn a3_err_http_handler() {
    let err = run_err("fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"nope_fn_xyz_123\", {}) return 0 }");
    assert_eq!(err.code, "E-HTTP-HANDLER", "got: {}", err.to_json());
    assert!(err.message.contains("nope_fn_xyz_123"), "names handler: {}", err.message);
    let err = run_err("fn two(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"two\", {}) return 0 }");
    assert_eq!(err.code, "E-HTTP-HANDLER", "arity: {}", err.to_json());
}

#[test]
fn a3_err_http_closed_and_status_and_route() {
    let err = run_err("fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) http_stop(s) let p = server_port(s) return 0 }");
    assert_eq!(err.code, "E-HTTP-CLOSED", "got: {}", err.to_json());
    let err = run_err("fn main() -> i32 { let r = http_json(99, {\"a\": 1}) return 0 }");
    assert_eq!(err.code, "E-HTTP-STATUS", "got: {}", err.to_json());
    let err = run_err("fn main() -> i32 { let r = http_text(999, \"x\") return 0 }");
    assert_eq!(err.code, "E-HTTP-STATUS", "got: {}", err.to_json());
    let err = run_err("fn main() -> i32 { let m = route_match(\"\", \"/a\") return 0 }");
    assert_eq!(err.code, "E-HTTP-ROUTE", "got: {}", err.to_json());
    let err = run_err("fn main() -> i32 { let m = route_match(\"/a/:\", \"/a/b\") return 0 }");
    assert_eq!(err.code, "E-HTTP-ROUTE", "got: {}", err.to_json());
}

#[test]
fn a3_err_json_parse_and_stringify() {
    for src in [
        "fn main() -> i32 { let v = json_decode(\"{oops\") return 0 }",
        "fn main() -> i32 { let v = json_decode(\"\") return 0 }",
        "fn main() -> i32 { let v = json_decode(\"[1, 2\") return 0 }",
        "fn main() -> i32 { let v = json_decode(\"1 2\") return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-JSON-PARSE", "{src}: {}", err.to_json());
    }
    // Stringify a closure and a handle: loud, never silent.
    let err = run_err("fn main() -> i32 { let f = fn(x: i32) -> i32 { return x + 1 } print(json_stringify(f, 0)) return 0 }");
    assert_eq!(err.code, "E-JSON-STRINGIFY", "closure: {}", err.to_json());
    let err = run_err("fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) print(json_stringify(s, 0)) http_stop(s) return 0 }");
    assert_eq!(err.code, "E-JSON-STRINGIFY", "handle: {}", err.to_json());
}

#[test]
fn a3_err_wrong_handle_types_are_etype() {
    check_err_code("fn main() -> i32 { let p = server_port(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { http_stop(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let r = http_request(42, \"x\", {}, \"\", 1) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let m = route_match(42, \"/a\") return 0 }", "E-TYPE");
    let err = run_err("fn main() -> i32 { let m = {\"k\": 1} let p = server_port(m[\"k\"]) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
    assert!(err.message.contains("Server"), "names kind: {}", err.message);
}

// ---- hostile input never panics ----

#[test]
fn a3_hostile_no_panic() {
    let cases = [
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\\u0000x\", 0, \"h\", {}) return 0 }",
        "fn main() -> i32 { let r = http_request(\"GET\", \"http://127.0.0.1:1/\\u0000\", {}, \"\", 500) return 0 }",
        "fn main() -> i32 { print(json_decode(\"\\u0000\")) return 0 }",
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let c = tcp_connect(\"127.0.0.1\", server_port(s), 2000) conn_write(c, \"\\x00\\xff\\r\\n\\r\\n\") print(conn_read_line(c)) conn_close(c) http_stop(s) return 42 }",
        "fn main() -> i32 { print(json_stringify(\"hi\\u0000there\", 0)) return 42 }",
        "fn main() -> i32 { let m = {\"k\": 1} print(http_request(m[\"k\"], \"http://x/\", {}, \"\", 100)) return 0 }",
    ];
    for src in cases {
        let prog = parse_ok(src);
        let mir = klang::mir::lower(&prog);
        let _ = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    }
    // Double stop is idempotent; double port after stop is CLOSED (pinned above).
    let (v, _) = run_ok("fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) http_stop(s) http_stop(s) return 42 }");
    assert_eq!(v, 42);
}

// ---- arity / static checks ----

#[test]
fn a3_static_arity_and_types() {
    check_err_code("fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let p = server_port() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let r = http_request(\"GET\", \"http://x/\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let r = http_json(200) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let m = route_match(\"/a\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let v = json_decode() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let s = json_stringify(1) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let p = server_port(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let r = http_request(1, 2, 3, 4, 5) return 0 }", "E-TYPE");
}

// ---- sandbox ----

#[test]
fn a3_sandbox_denies_network() {
    for src in [
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) return 0 }",
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) let p = server_port(s) return 0 }",
        "fn main() -> i32 { let r = http_request(\"GET\", \"http://127.0.0.1/\", {}, \"\", 1000) return 0 }",
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) http_stop(s) return 0 }",
    ] {
        let err = run_sandboxed(src).expect_err("sandbox must deny");
        assert!(err.starts_with("E-SANDBOX-NETWORK"), "{src}: got {err}");
    }
    // Pure constructors stay allowed in sandbox (like `is_null`).
    let (v, out) = run_sandboxed("fn main() -> i32 { let j = http_json(200, {\"a\": 1}) print(j[\"status\"]) print(route_match(\"/a/:x\", \"/a/b\")[\"x\"]) print(json_stringify({\"a\": 1}, 0)) print(is_null(json_decode(\"null\"))) return 42 }").expect("pure allowed");
    assert_eq!(v, 42, "out: {out:?}");
    assert_eq!(out, vec!["200", "b", "{\"a\":1}", "1"]);
}

// ---- backend parity: JIT and v2 reject loudly ----

#[test]
fn a3_jit_rejects_new_builtins() {
    for src in [
        "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) return 0 }",
        "fn main() -> i32 { let r = http_request(\"GET\", \"http://x/\", {}, \"\", 1000) return 0 }",
        "fn main() -> i32 { print(json_decode(\"{}\")) return 0 }",
        "fn main() -> i32 { print(route_match(\"/a\", \"/a\")) return 0 }",
    ] {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok(), "{src}");
        let mir = klang::mir::lower(&prog);
        let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject");
        assert!(err.contains("unsupported"), "loud JIT-FAIL, got: {err}");
    }
}

#[test]
fn a3_v2_rejects_new_builtins() {
    let src = "fn main() -> i32 {\n    print(http_serve(\"127.0.0.1\", 0, \"h\", {}))\n    return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let err = klang::runtime::v2::run_v2_program(&prog, "main").expect_err("v2 must reject");
    assert_eq!(err.code, "E-UNDEFINED", "got: {}", err.to_json());
}

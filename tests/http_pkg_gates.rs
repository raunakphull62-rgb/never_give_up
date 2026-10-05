//! http package wrapper gates (Phase 1 item 3).
//!
//! Offline-safe: the pure helpers are pinned by `klang run http_test.klang`.
//! This file pins the LIVE wrappers (`http_fetch`, `http_post_text`,
//! `http_post_form`, `http_post_json`, async twins) against a hermetic
//! loopback server (std `TcpListener`, ephemeral port) — no internet.
//! Real-internet checks live in `scripts/live-tests.sh` (KLANG_LIVE_TESTS=1).
//!
//! The real package sources are embedded via `include_str!` (import lines
//! stripped; the import plumbing itself is covered by `klang run`).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

use klang::parser::Parser;

/// Package sources with `import` lines removed (the test harness feeds one
/// merged program; `klang run http_test.klang` covers the imports).
fn pkg_src() -> String {
    let mut out = String::new();
    for part in [
        include_str!("../stdlib-packages/io/src/files.klang"),
        include_str!("../stdlib-packages/string/src/text.klang"),
        include_str!("../stdlib-packages/http/src/client.klang"),
        include_str!("../stdlib-packages/http/src/transfer.klang"),
    ] {
        for line in part.lines() {
            if !line.trim_start().starts_with("import ") {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

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

/// Serve exactly `expect` requests on 127.0.0.1, then exit. The 20s deadline
/// only triggers when the client fails to connect — tests fail, never hang.
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

fn respond(mut s: std::net::TcpStream, body: &[u8], got_ct: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nX-Got-Ct: {got_ct}\r\nConnection: close\r\n\r\n",
        body.len()
    );
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
    let ct = headers
        .iter()
        .find(|(k, _)| k == "content-type")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len.min(1 << 20)];
    if len > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }
    // GET returns a fixed body; POST echoes the request body.
    let is_get = request_line.starts_with("GET ");
    let out = if is_get { b"hello http".to_vec() } else { body };
    respond(s, &out, &ct);
}

#[test]
fn http_pkg_fetch_wrappers() {
    let (base, handle) = start_server(2);
    let src = pkg_src()
        + &format!(
            "fn main() -> i32 {{ let r = http_fetch(\"{base}/ok\") if http_ok(r) == false {{ return 1 }} if http_status(r) != 200 {{ return 2 }} if http_body(r) != \"hello http\" {{ return 3 }} if http_fetch_body(\"{base}/ok\") != \"hello http\" {{ return 4 }} if r[\"headers\"][\"content-type\"] != \"text/plain\" {{ return 5 }} return 42 }}"
        );
    let (v, out) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 42, "fetch assertions, prints: {out:?}");
}

#[test]
fn http_pkg_post_wrappers() {
    let (base, handle) = start_server(3);
    let src = pkg_src()
        + &format!(
            "fn main() -> i32 {{ let r = http_post_text(\"{base}/echo\", \"ping\", [\"X-A: b\"]) if r[\"body\"] != \"ping\" {{ return 1 }} let f = http_post_form(\"{base}/echo\", {{\"x\": \"1\"}}) if f[\"body\"] != \"x=1\" {{ return 2 }} if f[\"headers\"][\"x-got-ct\"] != \"application/x-www-form-urlencoded\" {{ return 3 }} let j = http_post_json(\"{base}/echo\", \"{{\\\"a\\\": 1}}\") if j[\"body\"] != \"{{\\\"a\\\": 1}}\" {{ return 4 }} if j[\"headers\"][\"x-got-ct\"] != \"application/json\" {{ return 5 }} return 42 }}"
        );
    let (v, out) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 42, "post assertions, prints: {out:?}");
}

#[test]
fn http_pkg_async_wrappers() {
    let (base, handle) = start_server(2);
    let src = pkg_src()
        + &format!(
            "fn main() -> i32 async {{ task_group {{ let h = spawn http_fetch_async(\"{base}/ok\") let g = spawn http_post_async_text(\"{base}/echo\", \"ping-a\", []) let r = await h if r[\"body\"] != \"hello http\" {{ return 1 }} let p = await g if p[\"body\"] != \"ping-a\" {{ return 2 }} return 42 }} }}"
        );
    let (v, out) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 42, "async assertions, prints: {out:?}");
}

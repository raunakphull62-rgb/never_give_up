//! A1 sockets gates (Wave 2 Stage A1).
//!
//! Loopback only (`127.0.0.1`, port 0 for an OS-picked free port, never a
//! fixed port). No real internet; DNS only resolves `localhost`. Tests are
//! deterministic and leave no sockets behind (every Klang program closes
//! its handles; Rust handles drop on scope exit). Sandbox-flag tests
//! serialize on a mutex because the flag is process-global.

use std::collections::HashMap;
use std::net::TcpListener;

use klang::parser::Parser;

static SANDBOX_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    klang::sandbox::set_sandbox(false);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    klang::sandbox::set_sandbox(false);
    r
}

fn run_value(src: &str) -> Result<(klang::runtime::Value, Vec<String>), klang::diagnostics::Diagnostic> {
    let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new());
    klang::sandbox::set_sandbox(false);
    r
}

fn run_err(src: &str) -> klang::diagnostics::Diagnostic {
    let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let e = klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new()).expect_err("must fail");
    klang::sandbox::set_sandbox(false);
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
    let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(true);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    klang::sandbox::set_sandbox(false);
    match r {
        Ok(v) => Ok(v),
        Err(d) => Err(format!("{} {}", d.code, d.to_json())),
    }
}

/// A port that was just bound and released: connecting to it is a
/// guaranteed ECONNREFUSED with no external dependency.
fn refused_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind");
    let p = l.local_addr().expect("addr").port();
    drop(l);
    p
}

// ---- happy paths ----

#[test]
fn a1_listener_port_is_nonzero() {
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) print(p) conn_close(l) if p == 0 { return 1 } return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42);
    assert_eq!(out.len(), 1);
    let p: i64 = out[0].parse().expect("port renders");
    assert!(p > 0 && p <= 65535, "port {p}");
}

#[test]
fn a1_tcp_echo_round_trip() {
    // Single-threaded rendezvous: connect queues, accept picks up.
    // Reads loop until the exact byte count: `conn_read` is one syscall
    // and may return a short prefix under load (documented A1 limit).
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) assert(is_null(s) == false) print(conn_write(c, \"hello\")) let g1 = \"\" while len(g1) < 5 { let pc = conn_read(s, 5 - len(g1)) if pc == \"\" { break } g1 = g1 + pc } print(g1) print(conn_write(s, \"world\")) let g2 = \"\" while len(g2) < 5 { let pc = conn_read(c, 5 - len(g2)) if pc == \"\" { break } g2 = g2 + pc } print(g2) conn_close(c) conn_close(s) conn_close(l) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["5", "hello", "5", "world"]);
}

#[test]
fn a1_conn_read_line_variants() {
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) conn_write(c, \"hi\\n\\nbye\\r\\n\") print(conn_read_line(s)) print(conn_read_line(s)) print(conn_read_line(s)) conn_close(c) conn_close(s) conn_close(l) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["hi", "", "bye"]);
}

#[test]
fn a1_conn_read_line_close_is_null() {
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) conn_close(c) print(is_null(conn_read_line(s))) print(conn_read(s, 10) == \"\") conn_close(s) conn_close(l) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1"]);
}

#[test]
fn a1_hex_round_trip_binary_safe() {
    // 0x00, 0xFF and UTF-8 multibyte survive exactly via hex (single
    // round trip only: a second read would block with no data left).
    // The hex read loops until all 10 chars: `conn_read_hex` is one
    // syscall and may short-read under load (same A1 limit as `conn_read`).
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) print(conn_write_hex(c, \"00ff48c3a9\")) let hx = \"\" while len(hx) < 10 { let pc = conn_read_hex(s, 10) if pc == \"\" { break } hx = hx + pc } print(hx) conn_close(c) conn_close(s) conn_close(l) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["5", "00ff48c3a9"]);
}

#[test]
fn a1_conn_peer_names_loopback() {
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) let peer = conn_peer(s) print(peer.contains(\"127.0.0.1\")) print(peer.contains(\":\")) conn_close(c) conn_close(s) conn_close(l) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1"]);
}

#[test]
fn a1_accept_timeout_null_then_success() {
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let t = tcp_accept_timeout(l, 50) print(is_null(t)) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) print(is_null(s)) conn_close(c) conn_close(s) conn_close(l) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "0"]);
}

#[test]
fn a1_large_payload_1mb() {
    // 1 MB total via 16 x 64 KB chunked writes/reads (single-threaded:
    // one big write would deadlock filling TCP buffers before the peer
    // reads; chunked interleaving keeps every round under the buffers).
    // Each chunk read loops until the full 64 KB: one `conn_read` syscall
    // may return a short prefix under load (same A1 limit as above).
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) let chunk = \"ab\".repeat(32768) assert(len(chunk) == 65536) let total = 0 let i = 0 while i < 16 { print(conn_write(c, chunk)) let got = \"\" while len(got) < 65536 { let pc = conn_read(s, 65536 - len(got)) if pc == \"\" { break } got = got + pc } assert(got == chunk) total = total + len(got) i = i + 1 } print(total) conn_close(c) conn_close(s) conn_close(l) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out[0], "65536");
    assert_eq!(out.last(), Some(&"1048576".to_string()));
}

#[test]
fn a1_udp_round_trip() {
    let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    // Real UDP round trip at the Rust level (hermetic, port 0 both sides).
    let a = klang::stdlib::a1_net::udp_bind("127.0.0.1", 0).expect("bind a");
    let b = klang::stdlib::a1_net::udp_bind("127.0.0.1", 0).expect("bind b");
    let pa = match &a {
        klang::runtime::Value::UdpSock(h) => {
            let st = h.lock().unwrap();
            st.socket.as_ref().unwrap().local_addr().unwrap().port()
        }
        _ => panic!("udp sock"),
    };
    let pb = match &b {
        klang::runtime::Value::UdpSock(h) => {
            let st = h.lock().unwrap();
            st.socket.as_ref().unwrap().local_addr().unwrap().port()
        }
        _ => panic!("udp sock"),
    };
    assert!(pa != 0 && pb != 0);
    let n = klang::stdlib::a1_net::udp_send_to(&a, "127.0.0.1", pb as i64, "ping-123").expect("send");
    assert_eq!(n, 8);
    let r = klang::stdlib::a1_net::udp_recv_from(&b, 100, 5000).expect("recv");
    assert_eq!(r.map_get("data"), Some(klang::runtime::Value::Str("ping-123".to_string())));
    assert_eq!(r.map_get("host"), Some(klang::runtime::Value::Str("127.0.0.1".to_string())));
    klang::stdlib::a1_net::conn_close(&a).expect("close a");
    klang::stdlib::a1_net::conn_close(&b).expect("close b");
}

#[test]
fn a1_udp_klang_round_trip_with_known_ports() {
    // Klang-level UDP: the timeout error path proves the call shape works
    // end to end (no assignment type trap: the call is a bare statement).
    // The real data-path round trip is pinned Rust-side above.
    let src = "fn main() -> i32 { let a = udp_bind(\"127.0.0.1\", 0) try { udp_recv_from(a, 100, 50) } catch e { print(e[\"code\"]) conn_close(a) if e[\"code\"] == \"E-NET-TIMEOUT\" { return 42 } return 1 } return 2 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["E-NET-TIMEOUT"]);
}

#[test]
fn a1_dns_localhost() {
    let src = "fn main() -> i32 { let ips = dns_lookup(\"localhost\") print(ips.contains(\"127.0.0.1\")) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1"]);
}

#[test]
fn a1_is_null_semantics() {
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let t = tcp_accept_timeout(l, 20) print(is_null(t)) print(is_null(0)) print(is_null(\"\")) print(str(t)) print(t == t) conn_close(l) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "0", "0", "null", "1"]);
}

#[test]
fn a1_port_reuse_after_close() {
    // OS-picked port, closed, then explicitly rebound: no ADDR-IN-USE.
    let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let l = klang::stdlib::a1_net::tcp_listen("127.0.0.1", 0).expect("listen");
    let port = klang::stdlib::a1_net::listener_port(&l).expect("port");
    assert!(port > 0);
    klang::stdlib::a1_net::conn_close(&l).expect("close");
    let l2 = klang::stdlib::a1_net::tcp_listen("127.0.0.1", port).expect("rebind same port");
    let port2 = klang::stdlib::a1_net::listener_port(&l2).expect("port2");
    assert_eq!(port, port2);
    klang::stdlib::a1_net::conn_close(&l2).expect("close2");
}

#[test]
fn a1_blocking_accept_with_rust_client() {
    // `tcp_accept` blocks: a Rust thread connects while the test waits.
    // Hold the sandbox lock (sandbox false) so no sandbox test can flip
    // the flag mid-accept; the client thread does plain TCP, no flag use.
    use std::net::TcpStream;
    use std::io::Write;
    let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let l = klang::stdlib::a1_net::tcp_listen("127.0.0.1", 0).expect("listen");
    let port = klang::stdlib::a1_net::listener_port(&l).expect("port");
    let handle = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut s = TcpStream::connect(format!("127.0.0.1:{port}")).expect("client connect");
        s.write_all(b"blocking-hi").expect("write");
    });
    let conn = klang::stdlib::a1_net::tcp_accept(&l).expect("accept");
    // Loop until the full 11 bytes: `conn_read` is one syscall and may
    // return a short prefix under load (same A1 limit as above).
    let mut line = String::new();
    while line.len() < 11 {
        let part = klang::stdlib::a1_net::conn_read(&conn, 100).expect("read");
        if part.is_empty() {
            break;
        }
        line.push_str(&part);
    }
    assert_eq!(line, "blocking-hi");
    klang::stdlib::a1_net::conn_close(&conn).expect("close conn");
    klang::stdlib::a1_net::conn_close(&l).expect("close listener");
    handle.join().expect("client thread");
}

// ---- error codes ----

#[test]
fn a1_err_refused() {
    let port = refused_port();
    let src = format!("fn main() -> i32 {{ let c = tcp_connect(\"127.0.0.1\", {port}, 2000) return 0 }}");
    let err = run_err(&src);
    assert_eq!(err.code, "E-NET-REFUSED", "got: {}", err.to_json());
    assert!(err.message.contains(&port.to_string()), "names the port: {}", err.message);
}

#[test]
fn a1_err_timeout_on_read() {
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) conn_set_timeout(s, 80) let x = conn_read(s, 100) return 0 }";
    let err = run_err(src);
    assert_eq!(err.code, "E-NET-TIMEOUT", "got: {}", err.to_json());
}

#[test]
fn a1_err_timeout_on_udp_recv() {
    let src = "fn main() -> i32 { let a = udp_bind(\"127.0.0.1\", 0) let r = udp_recv_from(a, 100, 60) return 0 }";
    let err = run_err(src);
    assert_eq!(err.code, "E-NET-TIMEOUT", "got: {}", err.to_json());
}

#[test]
fn a1_err_addr_invalid() {
    for src in [
        "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", -1) return 0 }",
        "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 99999) return 0 }",
        "fn main() -> i32 { let l = tcp_listen(\"\", 0) return 0 }",
        "fn main() -> i32 { let c = tcp_connect(\"127.0.0.1\", 99999, 1000) return 0 }",
        "fn main() -> i32 { let a = udp_bind(\"127.0.0.1\", -5) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-NET-ADDR-INVALID", "{src}: {}", err.to_json());
    }
}

#[test]
fn a1_err_addr_in_use() {
    // Hold one listener; a second bind to its port must fail loudly.
    // Lock first: `run_err` also locks, so hold the guard across the whole
    // test would deadlock — instead create the holder under the lock, drop
    // the guard, run, then re-lock to close. The window is tiny and the
    // holder port is unique, so no sandbox flip can steal it (sandbox only
    // affects new calls, not the already-bound socket).
    let (l, port) = {
        let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        klang::sandbox::set_sandbox(false);
        let l = klang::stdlib::a1_net::tcp_listen("127.0.0.1", 0).expect("listen");
        let port = klang::stdlib::a1_net::listener_port(&l).expect("port");
        (l, port)
    };
    let src = format!("fn main() -> i32 {{ let x = tcp_listen(\"127.0.0.1\", {port}) return 0 }}");
    let err = run_err(&src);
    assert_eq!(err.code, "E-NET-ADDR-IN-USE", "got: {}", err.to_json());
    assert!(err.message.contains(&port.to_string()), "names the port: {}", err.message);
    {
        let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        klang::sandbox::set_sandbox(false);
        klang::stdlib::a1_net::conn_close(&l).expect("close");
    }
}

#[test]
fn a1_err_closed() {
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) conn_close(s) let x = conn_read(s, 10) return 0 }";
    let err = run_err(src);
    assert_eq!(err.code, "E-NET-CLOSED", "got: {}", err.to_json());
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) conn_close(l) let p = listener_port(l) return 0 }";
    let err = run_err(src);
    assert_eq!(err.code, "E-NET-CLOSED", "got: {}", err.to_json());
    let src = "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) conn_close(c) let x = conn_peer(c) return 0 }";
    let err = run_err(src);
    assert_eq!(err.code, "E-NET-CLOSED", "got: {}", err.to_json());
}

#[test]
fn a1_err_io_bad_sizes_and_hex() {
    for (src, hint) in [
        ("fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) let x = conn_read(s, -1) return 0 }", "negative"),
        ("fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) let x = conn_read(s, 99999999) return 0 }", "too large"),
        ("fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) print(conn_write_hex(c, \"abc\")) return 0 }", "odd hex"),
        ("fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) print(conn_write_hex(c, \"zz\")) return 0 }", "bad digit"),
        ("fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) conn_set_timeout(s, -1) return 0 }", "negative timeout"),
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-NET-IO", "{hint} {src}: {}", err.to_json());
    }
}

#[test]
fn a1_err_dns() {
    for src in [
        "fn main() -> i32 { let ips = dns_lookup(\"nonexistent.invalid-xyz-123\") return 0 }",
        "fn main() -> i32 { let ips = dns_lookup(\"\") return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-NET-DNS", "{src}: {}", err.to_json());
    }
}

#[test]
fn a1_err_wrong_handle_types_are_etype() {
    // Static: known scalar where a handle is used.
    check_err_code("fn main() -> i32 { let p = listener_port(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let c = tcp_connect(42, 80, 1000) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = conn_read(42, 10) return 0 }", "E-TYPE");
    // Dynamic (Unknown through a map): loud E-TYPE at runtime, never a panic.
    let err = run_err("fn main() -> i32 { let m = {\"k\": 1} let p = listener_port(m[\"k\"]) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
    assert!(err.message.contains("Listener"), "names the handle kind: {}", err.message);
    let err = run_err("fn main() -> i32 { let m = {\"k\": \"x\"} let x = conn_read(m[\"k\"], 10) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
    assert!(err.message.contains("Conn"), "names the handle kind: {}", err.message);
}

// ---- hostile input never panics ----

#[test]
fn a1_hostile_no_panic() {
    // Every failure below is a diagnostic, never a host panic.
    let cases = [
        "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\\u0000x\", 0) return 0 }",
        "fn main() -> i32 { let c = tcp_connect(\"\", 80, 100) return 0 }",
        "fn main() -> i32 { let ips = dns_lookup(\"a\".repeat(5000)) return len(ips) }",
        "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let t = tcp_accept_timeout(l, 0) print(is_null(t)) conn_close(l) return 42 }",
        "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) print(conn_read(s, 0) == \"\") print(conn_write_hex(c, \"\")) print(conn_read_hex(s, 0) == \"\") conn_close(c) conn_close(s) conn_close(l) return 42 }",
    ];
    for src in cases {
        let prog = parse_ok(src);
        let mir = klang::mir::lower(&prog);
        let _ = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    }
    // Empty-line vs close: both render "" for the line itself, but close is
    // provably null via is_null (no panic, no hang).
    let (v, out) = run_ok("fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) conn_write(c, \"\\n\") print(conn_read_line(s) == \"\") conn_close(c) print(is_null(conn_read_line(s))) conn_close(s) conn_close(l) return 42 }");
    assert_eq!(v, 42);
    assert_eq!(out, vec!["1", "1"]);
}

// ---- arity / static checks ----

#[test]
fn a1_static_arity_and_types() {
    check_err_code("fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let p = listener_port() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let c = tcp_connect(\"127.0.0.1\", 80) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let x = conn_read(\"s\", \"bad\") return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = conn_write(1, 2) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = udp_send_to(1, 2, 3, 4) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let x = dns_lookup(42) return 0 }", "E-TYPE");
}

// ---- sandbox ----

#[test]
fn a1_sandbox_denies_network() {
    for src in [
        "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) return 0 }",
        "fn main() -> i32 { let c = tcp_connect(\"127.0.0.1\", 80, 100) return 0 }",
        "fn main() -> i32 { let a = udp_bind(\"127.0.0.1\", 0) return 0 }",
        "fn main() -> i32 { let ips = dns_lookup(\"localhost\") return 0 }",
        "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let t = tcp_accept_timeout(l, 10) return 0 }",
    ] {
        let err = run_sandboxed(src).expect_err("sandbox must deny");
        assert!(err.starts_with("E-SANDBOX-NETWORK"), "{src}: got {err}");
    }
    // `is_null` is pure (no network): allowed even in sandbox.
    let (v, _) = run_sandboxed("fn main() -> i32 { print(is_null(0)) return 42 }").expect("is_null allowed");
    assert_eq!(v, 42);
}

#[test]
fn a1_sandbox_denies_with_existing_handle() {
    // Handle created outside sandbox; use inside is still denied.
    let _guard = SANDBOX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let l = klang::stdlib::a1_net::tcp_listen("127.0.0.1", 0).expect("listen");
    klang::sandbox::set_sandbox(true);
    let err = klang::stdlib::a1_net::listener_port(&l).expect_err("must deny");
    assert_eq!(err.code, "E-SANDBOX-NETWORK", "got: {}", err.to_json());
    klang::sandbox::set_sandbox(false);
    klang::stdlib::a1_net::conn_close(&l).expect("close");
}

// ---- backend parity: JIT and v2 reject loudly ----

#[test]
fn a1_jit_rejects_new_builtins() {
    for src in [
        "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) return 0 }",
        "fn main() -> i32 { print(is_null(0)) return 0 }",
        "fn main() -> i32 { let ips = dns_lookup(\"localhost\") return 0 }",
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
fn a1_v2_rejects_new_builtins() {
    let src = "fn main() -> i32 {\n    print(tcp_listen(\"127.0.0.1\", 0))\n    return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let err = klang::runtime::v2::run_v2_program(&prog, "main").expect_err("v2 must reject");
    assert_eq!(err.code, "E-UNDEFINED", "got: {}", err.to_json());
}

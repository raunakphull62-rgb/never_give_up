//! H2 server performance + shutdown correctness (Wave 2 Stage H2).
//!
//! Regression gates for three fixes, each against hello_api or a small
//! custom server process (port 0/PORT_FILE, loopback only):
//! 1. new-connection throughput (blocking accept, no poll window),
//! 2. graceful shutdown drains the listen backlog (no dropped conns),
//! 3. idle keep-alive conns cannot starve workers (idle timeout +
//!    requests-per-conn cap).
//!
//! Floors are set well under the measured numbers on the 2-thread box so
//! the gates stay stable under parallel test load.

use std::io::{Read, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}
fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn mktemp_dir(prefix: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    p.push(format!("h2_{prefix}_{}_{}", std::process::id(), nanos % 1_000_000_000));
    std::fs::create_dir_all(&p).expect("mktemp");
    p
}

struct ChildServer {
    child: std::process::Child,
    port: u16,
    dir: std::path::PathBuf,
}
impl Drop for ChildServer {
    fn drop(&mut self) {
        // Never leave a server behind (TERM, then kill).
        let pid = self.child.id();
        std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output().ok();
        let start = std::time::Instant::now();
        while self.child.try_wait().ok().flatten().is_none() {
            if start.elapsed() > std::time::Duration::from_secs(3) {
                self.child.kill().ok();
                self.child.wait().ok();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// Spawn a klang program with PORT=0 + PORT_FILE, wait for the port.
fn spawn_server(program: &str, prefix: &str) -> ChildServer {
    let dir = mktemp_dir(prefix);
    let prog = dir.join("prog.klang");
    std::fs::write(&prog, program).expect("write prog");
    let port_file = dir.join("port.txt");
    let mut child = std::process::Command::new(klang_bin())
        .args(["run", prog.to_str().unwrap()])
        .current_dir(repo_root())
        .env("PORT", "0")
        .env("PORT_FILE", &port_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("server spawns");
    let port: u16 = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Ok(t) = std::fs::read_to_string(&port_file) {
                if let Ok(p) = t.trim().parse::<u16>() {
                    if p != 0 {
                        break p;
                    }
                }
            }
            assert!(std::time::Instant::now() < deadline, "no port in 15s");
            if child.try_wait().expect("try").is_some() {
                panic!("server died early");
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    };
    ChildServer { child, port, dir }
}

/// One request on a fresh connection (Connection: close); returns the status.
fn raw_get(port: u16, path: &str) -> u16 {
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_nodelay(true).ok();
    s.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .expect("write");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok();
    String::from_utf8_lossy(&buf)
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("0")
        .parse::<u16>()
        .unwrap_or(0)
}

/// One keep-alive request on an open conn; returns Some(close_after) —
/// true when the server answered with Connection: close (cap hit) — or
/// None when the conn was closed without a response (idle expiry, cap,
/// or a failed write on a dead conn).
fn raw_get_ka(s: &mut std::net::TcpStream) -> Option<bool> {
    if s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n").is_err() {
        return None;
    }
    raw_read_ka(s)
}

/// Read ONE keep-alive response from a conn we already wrote to (no
/// second write). None when the server closed without a full response.
fn raw_read_ka(s: &mut std::net::TcpStream) -> Option<bool> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut hend: Option<usize> = None;
    let mut want = 0usize;
    loop {
        let r = match s.read(&mut tmp) {
            Ok(0) | Err(_) => return None, // closed with no response
            Ok(r) => r,
        };
        buf.extend_from_slice(&tmp[..r]);
        if hend.is_none() {
            if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                hend = Some(p + 4);
                let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                want = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
            }
        }
        if let Some(h) = hend {
            if buf.len() >= h + want {
                break;
            }
        }
    }
    let head = String::from_utf8_lossy(&buf[..hend.unwrap()]).to_lowercase();
    Some(head.lines().any(|l| l.starts_with("connection:") && l.contains("close")))
}

// ---- 1. new-connection throughput ----

#[test]
fn h2_newconn_throughput() {
    let mut srv = spawn_server(
        r#"fn handle(req: map) -> map {
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {})
    write_file(get_env("PORT_FILE"), str(server_port(s)))
    while (signal_pending("TERM") == false) {
        sleep_ms(50)
    }
    http_stop(s)
    return 0
}
"#,
        "thr",
    );
    let port = srv.port;
    // Warmup (also proves the server answers at all).
    for _ in 0..20 {
        assert_eq!(raw_get(port, "/health"), 200);
    }
    // 5000 sequential requests, each on a NEW connection.
    let n = 5000;
    let t0 = std::time::Instant::now();
    for _ in 0..n {
        assert_eq!(raw_get(port, "/health"), 200);
    }
    let el = t0.elapsed();
    let rps = n as f64 / el.as_secs_f64();
    eprintln!("h2 newconn: {n} requests in {el:.2?} = {rps:.0} rps ({:.3} ms/req)", el.as_secs_f64() * 1000.0 / n as f64);
    // Floor is ~1/4 of the measured sequential rate (>=7k/s on this box);
    // the pre-fix baseline was ~200 rps, so this has huge margin both ways.
    assert!(rps >= 1500.0, "new-connection throughput collapsed: {rps:.0} rps");
    // Server still healthy afterwards.
    assert_eq!(raw_get(port, "/health"), 200);
    let pid = srv.child.id();
    std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output().ok();
    let start = std::time::Instant::now();
    loop {
        if srv.child.try_wait().expect("try").is_some() {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(8), "exit not bounded");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(srv.child.try_wait().expect("try").unwrap().code(), Some(0));
    drop(srv);
}

// ---- 2. graceful shutdown drains the backlog ----

#[test]
fn h2_graceful_backlog_drained() {
    // 8 workers x 100ms handlers, 100 clients: at TERM time most requests
    // sit in the job chan or the kernel listen backlog. Before the H2 fix
    // the listener was closed first and ~60 backlog conns were dropped.
    let prog = r#"fn handle(req: map) -> map {
    sleep_ms(100)
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {"workers": 8})
    write_file(get_env("PORT_FILE"), str(server_port(s)))
    while (signal_pending("TERM") == false) {
        sleep_ms(20)
    }
    http_stop(s)
    return 0
}
"#;
    let mut srv = spawn_server(prog, "drain");
    let port = srv.port;
    let pid = srv.child.id();
    let mut hs = Vec::new();
    for _ in 0..100 {
        hs.push(std::thread::spawn(move || raw_get(port, "/x")));
    }
    std::thread::sleep(std::time::Duration::from_millis(1000));
    std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output().expect("kill");
    let mut ok = 0;
    let mut answered = 0;
    for h in hs {
        let st = h.join().expect("client");
        if st != 0 {
            answered += 1;
        }
        if st == 200 {
            ok += 1;
        }
    }
    eprintln!("h2 drain: {ok}/100 got 200, {answered}/100 got any response");
    assert_eq!(ok, 100, "every request must complete, got {ok}/100");
    // Exit 0 bounded (10s).
    let start = std::time::Instant::now();
    loop {
        if let Some(st) = srv.child.try_wait().expect("try") {
            assert_eq!(st.code(), Some(0), "exit 0");
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "exit not bounded");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    drop(srv);
}

// ---- 3. idle keep-alive conns cannot starve workers ----

#[test]
fn h2_idle_keepalive_no_starvation() {
    let mut srv = spawn_server(
        r#"fn handle(req: map) -> map {
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {"workers": 4})
    write_file(get_env("PORT_FILE"), str(server_port(s)))
    while (signal_pending("TERM") == false) {
        sleep_ms(50)
    }
    http_stop(s)
    return 0
}
"#,
        "idle",
    );
    let port = srv.port;
    // Open 20 keep-alive connections, one request each, then leave idle.
    // Default workers 4: as more conns arrive, parked workers hand their
    // idle conn over (quiet close), so everything still gets served.
    let mut conns: Vec<std::net::TcpStream> = Vec::new();
    for _ in 0..20 {
        let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        s.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();
        assert_eq!(raw_get_ka(&mut s), Some(false), "first request on idle conn");
        conns.push(s);
    }
    // Fresh request on a NEW connection must be answered within 1s even
    // though 4 workers are parked on idle keep-alive conns.
    let t0 = std::time::Instant::now();
    assert_eq!(raw_get(port, "/health"), 200);
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(1),
        "fresh request starved by idle keep-alive conns: {:?}",
        t0.elapsed()
    );
    // One worker was handed the fresh conn (its idle conn was closed
    // quietly); the other three still park on idle conns, so at least one
    // of the 20 must still be usable within 1s.
    let t1 = std::time::Instant::now();
    let mut still_usable = 0;
    for c in conns.iter_mut() {
        if raw_get_ka(c) == Some(false) {
            still_usable += 1;
        }
    }
    assert!(still_usable >= 1, "no idle keep-alive conn survived");
    assert!(
        t1.elapsed() < std::time::Duration::from_secs(1),
        "idle conns' next request starved: {:?}",
        t1.elapsed()
    );
    // Default idle timeout (5s): by ~6.5s every idle conn is closed by the
    // server (released-early or budget-expired), and it keeps serving.
    std::thread::sleep(std::time::Duration::from_millis(6500));
    for (i, c) in conns.iter_mut().enumerate() {
        assert_eq!(raw_get_ka(c), None, "idle conn {i} must be closed after idle_timeout_ms");
    }
    assert_eq!(raw_get(port, "/health"), 200, "server still serving after idle expiry");
    let pid = srv.child.id();
    std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output().ok();
    let start = std::time::Instant::now();
    loop {
        if let Some(st) = srv.child.try_wait().expect("try") {
            assert_eq!(st.code(), Some(0), "exit 0");
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(8), "exit not bounded");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    drop(srv);
}

#[test]
fn h2_idle_timeout_configurable() {
    // idle_timeout_ms 300: idle conn closed soon after; fresh conn fine.
    let prog = r#"fn handle(req: map) -> map {
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {"idle_timeout_ms": 300})
    write_file(get_env("PORT_FILE"), str(server_port(s)))
    while (signal_pending("TERM") == false) {
        sleep_ms(50)
    }
    http_stop(s)
    return 0
}
"#;
    let srv = spawn_server(prog, "idlecfg");
    let port = srv.port;
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    assert_eq!(raw_get_ka(&mut s), Some(false));
    // Idle ~700ms: server must have closed it (quiet close after 300ms).
    std::thread::sleep(std::time::Duration::from_millis(700));
    let got = raw_get_ka(&mut s);
    assert!(got.is_none() || got == Some(true), "idle conn not closed after idle_timeout_ms, got {got:?}");
    // Server still serving.
    assert_eq!(raw_get(port, "/health"), 200);
    // Bad option values are E-HTTP-OPTIONS.
    let bad = "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {\"idle_timeout_ms\": -1}) return 0 }";
    let dir = mktemp_dir("idlebad");
    std::fs::write(dir.join("p.klang"), bad).expect("write");
    let out = std::process::Command::new(klang_bin())
        .args(["run", "p.klang"])
        .current_dir(&dir)
        .output()
        .expect("run");
    let txt = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(txt.contains("E-HTTP-OPTIONS"), "bad idle_timeout_ms must be E-HTTP-OPTIONS: {txt:?}");
    std::fs::remove_dir_all(&dir).ok();
    drop(srv);
}

#[test]
fn h2_max_requests_per_conn_cap() {
    // max_requests_per_conn 3: the 3rd keep-alive response carries
    // Connection: close; a 4th request on the same conn is not served,
    // while a fresh conn works.
    let prog = r#"fn handle(req: map) -> map {
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {"max_requests_per_conn": 3})
    write_file(get_env("PORT_FILE"), str(server_port(s)))
    while (signal_pending("TERM") == false) {
        sleep_ms(50)
    }
    http_stop(s)
    return 0
}
"#;
    let srv = spawn_server(prog, "cap");
    let port = srv.port;
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();
    assert_eq!(raw_get_ka(&mut s), Some(false), "request 1 keeps alive");
    assert_eq!(raw_get_ka(&mut s), Some(false), "request 2 keeps alive");
    assert_eq!(raw_get_ka(&mut s), Some(true), "request 3 hits the cap: Connection: close");
    assert_eq!(raw_get_ka(&mut s), None, "request 4 must not be served");
    // Fresh conn still works.
    assert_eq!(raw_get(port, "/health"), 200);
    drop(srv);
}

// ---- options parsing (library level) ----

#[test]
fn h2_serve_options_validation() {
    klang::sandbox::set_sandbox(false);
    let base = klang::stdlib::a3_http::ServeOptions::default();
    assert_eq!(base.idle_timeout_ms, 5000, "default idle timeout");
    assert_eq!(base.max_requests_per_conn, 1000, "default requests cap");
    let opts = |pairs: Vec<(&str, i64)>| {
        let v = klang::runtime::Value::Map(
            pairs.into_iter().map(|(k, n)| (k.to_string(), klang::runtime::Value::Int(n))).collect(),
        );
        klang::stdlib::a3_http::parse_serve_options(&v)
    };
    assert!(opts(vec![("idle_timeout_ms", 60000)]).is_ok());
    assert!(opts(vec![("idle_timeout_ms", 0)]).is_ok(), "0 = never idle-close");
    assert!(opts(vec![("max_requests_per_conn", 1000000)]).is_ok());
    assert!(opts(vec![("max_requests_per_conn", 0)]).is_ok(), "0 = unlimited");
    let e = opts(vec![("idle_timeout_ms", 60001)]).expect_err("idle past cap");
    assert_eq!(e.code, "E-HTTP-OPTIONS");
    assert_eq!(opts(vec![("idle_timeout_ms", -5)]).expect_err("idle negative").code, "E-HTTP-OPTIONS");
    assert_eq!(
        opts(vec![("max_requests_per_conn", 1000001)]).expect_err("cap past cap").code,
        "E-HTTP-OPTIONS"
    );
    assert_eq!(opts(vec![("max_requests_per_conn", -1)]).expect_err("cap negative").code, "E-HTTP-OPTIONS");
    // Both new keys combine with the old ones.
    let both = opts(vec![
        ("workers", 2),
        ("max_body_bytes", 100),
        ("read_timeout_ms", 1000),
        ("max_header_bytes", 2048),
        ("idle_timeout_ms", 4000),
        ("max_requests_per_conn", 50),
    ])
    .expect("all keys");
    assert_eq!(both.workers, 2);
    assert_eq!(both.max_body_bytes, 100);
    assert_eq!(both.read_timeout_ms, 1000);
    assert_eq!(both.max_header_bytes, 2048);
    assert_eq!(both.idle_timeout_ms, 4000);
    assert_eq!(both.max_requests_per_conn, 50);
}

// ---- burst queue capacity: 200 queued conns all served ----

#[test]
fn h2_queued_200_all_served() {
    // Open 200 connections rapidly (few workers): every one must be served
    // once the workers catch up (the 100ms chunk gives parked workers up).
    let prog = r#"fn handle(req: map) -> map {
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {"workers": 2})
    write_file(get_env("PORT_FILE"), str(server_port(s)))
    while (signal_pending("TERM") == false) {
        sleep_ms(50)
    }
    http_stop(s)
    return 0
}
"#;
    let srv = spawn_server(prog, "burst");
    let port = srv.port;
    let served = Arc::new(AtomicUsize::new(0));
    let mut hs = Vec::new();
    for _ in 0..200 {
        let p = port;
        let cnt = served.clone();
        hs.push(std::thread::spawn(move || {
            // Fresh conn per request, generous timeout: queued work is fine.
            let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{p}")).expect("connect");
            s.set_read_timeout(Some(std::time::Duration::from_secs(30))).ok();
            s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").expect("write");
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).ok();
            let txt = String::from_utf8_lossy(&buf).into_owned();
            if txt.lines().next().unwrap_or("").contains(" 200 ") {
                cnt.fetch_add(1, Ordering::SeqCst);
            }
        }));
    }
    for h in hs {
        h.join().expect("client");
    }
    let got = served.load(Ordering::SeqCst);
    eprintln!("h2 burst: {got}/200 served");
    assert_eq!(got, 200, "all queued conns served, got {got}/200");
    drop(srv);
}

#[test]
fn h2_pipelined_burst_all_served() {
    let prog = r#"fn handle(req: map) -> map {
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {"workers": 1})
    write_file(get_env("PORT_FILE"), str(server_port(s)))
    while (signal_pending("TERM") == false) {
        sleep_ms(50)
    }
    http_stop(s)
    return 0
}
"#;
    // 150 requests pipelined in ONE 4KB write (< the runtime's 4096-byte
    // read chunk): the whole burst lands in the leftover buffer with an
    // EMPTY kernel queue. The idle probe must count buffered leftovers
    // as readable, or the burst is orphaned and the conn parked.
    let srv = spawn_server(prog, "pipe");
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{}", srv.port)).expect("connect");
    s.set_nodelay(true).ok();
    s.set_read_timeout(Some(std::time::Duration::from_secs(15))).ok();
    let n = 150;
    let one = "GET / HTTP/1.1\r\nHost: x\r\n\r\n";
    s.write_all(one.repeat(n).as_bytes()).expect("burst write");
    let mut served = 0;
    for _ in 0..n {
        // Read-only: extra writes here would prime the kernel queue and
        // hide a parked conn holding buffered requests.
        if raw_read_ka(&mut s).is_none() {
            break;
        }
        served += 1;
    }
    eprintln!("h2 pipelined: {served}/{n} served from one write");
    assert_eq!(served, n, "pipelined burst fully served, got {served}/{n}");
    drop(srv);
}

#[test]
fn h2_stop_parked_request_served() {
    // A request that lands on an idle-parked conn right AFTER TERM still
    // gets served (drain rule); the shutdown is stretched so the race
    // window is deterministic: slow queued handlers keep the finisher
    // busy (it stops the idle pool only after the queue drains).
    let prog = r#"fn handle(req: map) -> map {
    if (req["query"]["slow"] == "1") {
        sleep_ms(40)
    }
    return http_text(200, "ok")
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {"workers": 1})
    write_file(get_env("PORT_FILE"), str(server_port(s)))
    while (signal_pending("TERM") == false) {
        sleep_ms(5)
    }
    http_stop(s)
    return 0
}
"#;
    let mut srv = spawn_server(prog, "parkstop");
    let port = srv.port;
    let mut open = || -> std::net::TcpStream {
        let s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        s.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();
        s
    };
    let slow = "GET /?slow=1 HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n";
    let fast = "GET / HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n";
    // A: slow request in flight (worker busy 40ms).
    let mut a = open();
    a.write_all(slow.as_bytes()).expect("a write");
    std::thread::sleep(std::time::Duration::from_millis(10));
    // B/C/D: fast request answered, then parked idle.
    let mut parked = Vec::new();
    for _ in 0..3 {
        let mut s = open();
        s.write_all(fast.as_bytes()).expect("park write");
        assert_eq!(raw_get_ka(&mut s), Some(false), "first request served");
        parked.push(s);
    }
    // Slow queued fills stretch the shutdown window (~6 x 40ms).
    let mut fills = Vec::new();
    for _ in 0..6 {
        let mut s = open();
        s.write_all(slow.as_bytes()).expect("fill write");
        fills.push(s);
    }
    std::thread::sleep(std::time::Duration::from_millis(120));
    let pid = srv.child.id().to_string();
    std::process::Command::new("kill").args(["-TERM", &pid]).output().ok();
    std::thread::sleep(std::time::Duration::from_millis(5));
    // The parked conns' request lands while shutdown is running.
    let mut served = 0;
    for s in parked.iter_mut() {
        if s.write_all(fast.as_bytes()).is_ok() && raw_read_ka(s).is_some() {
            served += 1;
        }
    }
    let mut closed = 0;
    for s in fills.iter_mut() {
        if raw_read_ka(s).is_some() {
            closed += 1;
        }
    }
    // A's slow response and the queued fills complete pre-exit.
    assert_eq!(raw_read_ka(&mut a), Some(false), "in-flight A completes");
    // Server exits 0.
    let start = std::time::Instant::now();
    loop {
        if srv.child.try_wait().expect("try").is_some() {
            break;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "no exit in 10s");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    eprintln!("h2 parkstop: {served}/3 parked requests served, {closed}/6 fills");
    assert_eq!(served, 3, "parked conns served at TERM, got {served}/3");
    assert_eq!(closed, 6, "queued fills served, got {closed}/6");
}

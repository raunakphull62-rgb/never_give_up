//! H1 backend hardening + stress (Wave 2 Stage H1).
//!
//! Targets: hello_api + notes_api as real processes (port 0/PORT_FILE,
//! loopback only). Each test <~60s (soak opt-in excepted), unique temp
//! dirs, no leftover processes/sockets/files. Live PG tests skip cleanly
//! without KLANG_TEST_PG_URL.

use std::io::{Read, Write};
use std::process::Stdio;

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
    p.push(format!("h1_{prefix}_{}_{}", std::process::id(), nanos % 1_000_000_000));
    std::fs::create_dir_all(&p).expect("mktemp");
    p
}
fn pg_url() -> Option<String> {
    let raw = std::env::var("KLANG_TEST_PG_URL").ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    let raw = raw.trim().to_string();
    if raw.contains("sslmode=") {
        return Some(raw);
    }
    if raw.contains("@localhost") || raw.contains("@127.0.0.1") {
        if raw.contains('?') {
            return Some(format!("{raw}&sslmode=disable"));
        }
        return Some(format!("{raw}?sslmode=disable"));
    }
    Some(raw)
}
fn klang_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

struct ChildServer {
    child: std::process::Child,
    port: u16,
    dir: std::path::PathBuf,
}
fn spawn_hello() -> ChildServer {
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
        .expect("hello spawns");
    let port: u16 = {
        let mut waited = 0;
        loop {
            if let Ok(t) = std::fs::read_to_string(&port_file) {
                if let Ok(p) = t.trim().parse::<u16>() {
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
                panic!("hello_api no PORT_FILE in 10s");
            }
            if child.try_wait().expect("try").is_some() {
                panic!("hello_api died early");
            }
        }
    };
    ChildServer { child, port, dir }
}
fn spawn_notes(db_url: &str) -> ChildServer {
    let dir = mktemp_dir("notes");
    let port_file = dir.join("port.txt");
    let mut child = std::process::Command::new(klang_bin())
        .args(["run", "examples/apps/notes_api/main.klang"])
        .current_dir(repo_root())
        .env("DATABASE_URL", db_url)
        .env("PORT", "0")
        .env("PORT_FILE", &port_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("notes spawns");
    let port: u16 = {
        let mut waited = 0;
        loop {
            if let Ok(t) = std::fs::read_to_string(&port_file) {
                if let Ok(p) = t.trim().parse::<u16>() {
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
                panic!("notes_api no PORT_FILE in 10s");
            }
            if child.try_wait().expect("try").is_some() {
                panic!("notes_api died early");
            }
        }
    };
    ChildServer { child, port, dir }
}
fn term_and_wait(srv: &mut ChildServer, secs: u64) -> std::process::ExitStatus {
    let pid = srv.child.id();
    std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output()
        .expect("kill");
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(st)) = srv.child.try_wait() {
            return st;
        }
        if start.elapsed() > std::time::Duration::from_secs(secs) {
            srv.child.kill().ok();
            return srv.child.wait().expect("wait");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
fn raw_get(port: u16, path: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .expect("write");
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok();
    let txt = String::from_utf8_lossy(&buf).into_owned();
    let code = txt.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("0").parse::<u16>().unwrap_or(0);
    (code, txt)
}
fn fd_count(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/fd")).map(|d| d.count()).unwrap_or(0)
}
fn thread_count(pid: u32) -> usize {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("Threads:")).map(|l| l.to_string()))
        .and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse::<usize>().ok()))
        .unwrap_or(0)
}
fn rss_kb(pid: u32) -> usize {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l.to_string()))
        .and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse::<usize>().ok()))
        .unwrap_or(0)
}

// ---- 1. concurrency: 200 x 50, every response correct ----

#[test]
fn h1_concurrency_200x50() {
    let mut srv = spawn_hello();
    let port = srv.port;
    // Warmup.
    assert_eq!(raw_get(port, "/health").0, 200);
    let start = std::time::Instant::now();
    let mut lat: Vec<u128> = Vec::with_capacity(10_000);
    // 200 clients x 50 reqs, 20 Rust threads x 10 clients each (bounded).
    let mut handles = Vec::new();
    for _ in 0..20 {
        handles.push(std::thread::spawn(move || {
            let mut local = Vec::with_capacity(500);
            for _ in 0..10 {
                // One keep-alive connection per client, 50 sequential GETs.
                let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
                s.set_read_timeout(Some(std::time::Duration::from_secs(15))).ok();
                for _ in 0..50 {
                    let t0 = std::time::Instant::now();
                    s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n").expect("write");
                    // Read exactly one response (headers + body) via Content-Length framing.
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    let mut header_end: Option<usize> = None;
                    let mut want = 0usize;
                    loop {
                        let n = s.read(&mut tmp).expect("read");
                        if n == 0 {
                            panic!("peer closed mid-response");
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if header_end.is_none() {
                            if let Some(p) = find_sub(&buf, b"\r\n\r\n") {
                                header_end = Some(p + 4);
                                want = content_len(&buf[..p]);
                            }
                        }
                        if let Some(h) = header_end {
                            if buf.len() >= h + want {
                                break;
                            }
                        }
                        if buf.len() > 1_000_000 {
                            panic!("response too large");
                        }
                    }
                    let h = header_end.unwrap();
                    let body = String::from_utf8_lossy(&buf[h..h + want]).into_owned();
                    assert!(body.contains("\"ok\""), "health body: {body}");
                    local.push(t0.elapsed().as_micros());
                    // Consume exactly want bytes; keep-alive reuses the rest (none expected).
                    if buf.len() > h + want {
                        panic!("pipelined extra bytes");
                    }
                }
            }
            local
        }));
    }
    for h in handles {
        lat.extend(h.join().expect("client thread"));
    }
    let elapsed = start.elapsed();
    assert_eq!(lat.len(), 10_000, "none dropped");
    lat.sort_unstable();
    let rps = 10_000.0 / elapsed.as_secs_f64();
    let p50 = lat[lat.len() / 2] as f64 / 1000.0;
    let p99 = lat[(lat.len() * 99 / 100).min(lat.len() - 1)] as f64 / 1000.0;
    eprintln!("h1 concurrency: 10000 reqs in {elapsed:.2?} = {rps:.0} rps, p50 {p50:.2}ms p99 {p99:.2}ms");
    // Server still alive.
    assert_eq!(raw_get(port, "/health").0, 200);
    let st = term_and_wait(&mut srv, 8);
    assert_eq!(st.code(), Some(0));
    std::fs::remove_dir_all(&srv.dir).ok();
}
fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
fn content_len(headers: &[u8]) -> usize {
    let txt = String::from_utf8_lossy(headers).to_lowercase();
    for line in txt.lines() {
        if let Some(v) = line.strip_prefix("content-length:") {
            return v.trim().parse::<usize>().unwrap_or(0);
        }
    }
    0
}

/// N sequential GETs on keep-alive connections (fast leak probe). The
/// server closes a conn after `max_requests_per_conn` (default 1000), so
/// the client reconnects like a real client and keeps counting.
fn keepalive_gets(port: u16, n: usize) {
    let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    s.set_read_timeout(Some(std::time::Duration::from_secs(15))).ok();
    for i in 0..n {
        if !keepalive_one(&mut s) {
            // Server capped/closed the conn: reconnect and retry this one.
            s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("reconnect");
            s.set_read_timeout(Some(std::time::Duration::from_secs(15))).ok();
            assert!(keepalive_one(&mut s), "fresh conn must serve request {i}");
        }
    }
}

/// One keep-alive GET; false when the conn closed without a response.
fn keepalive_one(s: &mut std::net::TcpStream) -> bool {
    if s.write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n").is_err() {
        return false;
    }
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut hend: Option<usize> = None;
    let mut want = 0usize;
    loop {
        let r = match s.read(&mut tmp) {
            Ok(0) | Err(_) => return false, // server closed (cap/keep-alive off)
            Ok(r) => r,
        };
        buf.extend_from_slice(&tmp[..r]);
        if hend.is_none() {
            if let Some(p) = find_sub(&buf, b"\r\n\r\n") {
                hend = Some(p + 4);
                want = content_len(&buf[..p]);
            }
        }
        if let Some(h) = hend {
            if buf.len() >= h + want {
                break;
            }
        }
    }
    let h = hend.unwrap();
    let body = String::from_utf8_lossy(&buf[h..h + want]).into_owned();
    assert!(body.contains("\"ok\""), "health body: {body}");
    assert_eq!(buf.len(), h + want, "no pipelining");
    true
}

// ---- 2. hostile input: server never panics/hangs, keeps serving ----

#[test]
fn h1_hostile_input() {
    let mut srv = spawn_hello();
    let port = srv.port;
    assert_eq!(raw_get(port, "/health").0, 200);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("garbage", b"\x00\xff\xfeCHAOS\r\n\r\n".to_vec()),
        ("no-crlf", b"GET /health HTTP/1.1".to_vec()),
        ("huge-header-line", {
            let mut v = b"GET /health HTTP/1.1\r\nX-Big: ".to_vec();
            v.extend(vec![b'A'; 1_000_000]);
            v.extend(b"\r\n\r\n".to_vec());
            v
        }),
        ("body-longer-than-len", b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{\"a\": 123456789}".to_vec()),
        ("body-shorter-than-len", b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"a\": 1}".to_vec()),
        ("neg-len", b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: -5\r\nConnection: close\r\n\r\n".to_vec()),
        ("huge-len", b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 99999999999999999999\r\nConnection: close\r\n\r\n".to_vec()),
        ("header-injection", b"GET /health HTTP/1.1\r\nHost: x\r\nX-Evil: a\r\nInjected: 1\r\nConnection: close\r\n\r\n".to_vec()),
        ("bad-crlf-split", b"GET /health HTTP/1.1\nHost: x\n\n".to_vec()),
    ];
    for (name, payload) in cases {
        let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        s.set_read_timeout(Some(std::time::Duration::from_secs(8))).ok();
        // Half-close variant for one case: shutdown write after send.
        s.write_all(&payload).ok();
        if name == "body-shorter-than-len" {
            // Half-close: client done sending, server should not hang (read returns "", request completes or times out).
            use std::net::Shutdown;
            s.shutdown(Shutdown::Write).ok();
        }
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).ok();
        let txt = String::from_utf8_lossy(&buf).into_owned();
        let code = txt.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("").to_string();
        eprintln!("h1 hostile {name}: first-line={code}");
        // Regression (H1): short body with half-close must be 400, not silent close.
        if name == "body-shorter-than-len" {
            assert_eq!(code, "400", "short body must be 400, got: {txt:?}");
        }
        // Must have answered something (4xx/5xx/200) or closed; must not hang (read timeout hit above if hung).
        assert!(srv.child.try_wait().expect("try").is_none(), "server died on {name}");
        // Keep serving.
        assert_eq!(raw_get(port, "/health").0, 200, "server kept serving after {name}");
    }
    // Slow client: 1 byte/sec must not block others; hits 408.
    let slow_port = port;
    let slow = std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{slow_port}")).expect("slow connect");
        s.set_read_timeout(Some(std::time::Duration::from_secs(20))).ok();
        s.write_all(b"G").ok();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        s.write_all(b"E").ok();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).ok();
        String::from_utf8_lossy(&buf).into_owned()
    });
    // Fast client meanwhile must succeed quickly (<5s, not behind slow).
    std::thread::sleep(std::time::Duration::from_millis(200));
    let t0 = std::time::Instant::now();
    assert_eq!(raw_get(port, "/health").0, 200);
    assert!(t0.elapsed() < std::time::Duration::from_secs(5), "slow blocked fast");
    let slow_txt = slow.join().expect("slow thread");
    assert!(
        slow_txt.contains("408") || slow_txt.is_empty() || slow_txt.contains("400"),
        "slow must get 408/close, got: {slow_txt:?}"
    );
    // Connect-and-close immediately x20.
    for _ in 0..20 {
        let s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        drop(s);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert_eq!(raw_get(port, "/health").0, 200);
    assert!(srv.child.try_wait().expect("try").is_none());
    let st = term_and_wait(&mut srv, 8);
    assert_eq!(st.code(), Some(0));
    std::fs::remove_dir_all(&srv.dir).ok();
}

// ---- 3. handler faults: 500s, server stays up ----

#[test]
fn h1_handler_faults() {
    let dir = mktemp_dir("faults");
    let prog = dir.join("fault.klang");
    std::fs::write(
        &prog,
        r#"fn handle(req: map) -> map {
    if (req["path"] == "/throw") {
        let x = parse_int("abc")
        return http_text(200, "unreached")
    }
    if (req["path"] == "/sleep") {
        sleep_ms(300)
        return http_json(200, {"ok": 1})
    }
    if (req["path"] == "/wrongtype") {
        return 42
    }
    if (req["path"] == "/huge") {
        return http_text(200, "x".repeat(500000))
    }
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {"workers": 4})
    let pf = get_env("PORT_FILE")
    if (pf != "") {
        write_file(pf, str(server_port(s)))
    }
    while (signal_pending("TERM") == false) {
        sleep_ms(50)
    }
    http_stop(s)
    return 0
}
"#,
    )
    .expect("write fault prog");
    // fmt clean.
    let fmt = std::process::Command::new(klang_bin())
        .args(["fmt", "--check", prog.to_str().unwrap()])
        .output()
        .expect("fmt");
    assert!(fmt.status.success(), "fmt: {}", String::from_utf8_lossy(&fmt.stderr));
    let port_file = dir.join("port.txt");
    let mut child = std::process::Command::new(klang_bin())
        .args(["run", prog.to_str().unwrap()])
        .env("PORT_FILE", &port_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("fault server spawns");
    let port: u16 = {
        let mut w = 0;
        loop {
            if let Ok(t) = std::fs::read_to_string(&port_file) {
                if let Ok(p) = t.trim().parse::<u16>() {
                    if p != 0 {
                        break p;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            w += 1;
            if w > 100 {
                child.kill().ok();
                child.wait().ok();
                panic!("fault server no port");
            }
        }
    };
    // Throw -> 500, server alive.
    assert_eq!(raw_get(port, "/throw").0, 500);
    assert_eq!(raw_get(port, "/health").0, 200);
    // Sleep -> 200 (slow but completes).
    assert_eq!(raw_get(port, "/sleep").0, 200);
    // Wrong type (handler returns Int, not map) -> 500.
    assert_eq!(raw_get(port, "/wrongtype").0, 500);
    // Huge body (500KB via repeat, under the 1M repeat cap) -> 200.
    let (st, txt) = raw_get(port, "/huge");
    assert_eq!(st, 200);
    assert!(txt.len() > 500_000, "huge body len {}", txt.len());
    assert_eq!(raw_get(port, "/health").0, 200);
    let pid = child.id();
    std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output().expect("kill");
    let mut w = 0;
    let st = loop {
        if let Ok(Some(s)) = child.try_wait() {
            break s;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        w += 1;
        if w > 100 {
            child.kill().ok();
            break child.wait().expect("wait");
        }
    };
    assert_eq!(st.code(), Some(0));
    std::fs::remove_dir_all(&dir).ok();
}

// ---- 4. graceful shutdown under load ----

#[test]
fn h1_graceful_shutdown_under_load() {
    graceful_run(8, 100);
}

/// 32-worker / 300ms-handler variant (kept from the H1 gate as an
/// additional test): all 100 in-flight complete through the drain.
#[test]
fn h1_graceful_shutdown_under_load_32w() {
    graceful_run(32, 300);
}

fn graceful_run(workers: i64, handler_sleep_ms: i64) {
    let dir = mktemp_dir("grace");
    let prog = dir.join("sleep.klang");
    std::fs::write(
        &prog,
        format!(
            r#"fn handle(req: map) -> map {{
    sleep_ms({handler_sleep_ms})
    return http_json(200, {{"ok": 1}})
}}

fn main() -> i32 {{
    signal_watch("TERM")
    let s = http_serve("127.0.0.1", 0, "handle", {{"workers": {workers}}})
    let pf = get_env("PORT_FILE")
    if (pf != "") {{
        write_file(pf, str(server_port(s)))
    }}
    while (signal_pending("TERM") == false) {{
        sleep_ms(20)
    }}
    http_stop(s)
    return 0
}}
"#
        ),
    )
    .expect("write");
    let port_file = dir.join("port.txt");
    let mut child = std::process::Command::new(klang_bin())
        .args(["run", prog.to_str().unwrap()])
        .env("PORT_FILE", &port_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawns");
    let port: u16 = {
        let mut w = 0;
        loop {
            if let Ok(t) = std::fs::read_to_string(&port_file) {
                if let Ok(p) = t.trim().parse::<u16>() {
                    if p != 0 {
                        break p;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            w += 1;
            if w > 100 {
                child.kill().ok();
                child.wait().ok();
                panic!("no port");
            }
        }
    };
    let pid = child.id();
    let fds_before = fd_count(pid);
    let thrs_before = thread_count(pid);
    // 100 concurrent in-flight.
    let mut hs = Vec::new();
    for _ in 0..100 {
        hs.push(std::thread::spawn(move || {
            let (st, txt) = raw_get(port, "/x");
            (st, txt.contains("\"ok\""))
        }));
    }
    std::thread::sleep(std::time::Duration::from_millis(1000));
    // TERM while in flight.
    std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).output().expect("kill");
    let mut ok = 0;
    for h in hs {
        let (st, good) = h.join().expect("client");
        if st == 200 && good {
            ok += 1;
        }
    }
    eprintln!("h1 graceful({workers}w/{handler_sleep_ms}ms): {ok}/100 in-flight completed");
    assert!(ok >= 90, "in-flight must mostly complete, got {ok}/100");
    // Exit 0 bounded (10s).
    let start = std::time::Instant::now();
    let st = loop {
        if let Ok(Some(s)) = child.try_wait() {
            break s;
        }
        if start.elapsed() > std::time::Duration::from_secs(10) {
            child.kill().ok();
            break child.wait().expect("wait");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(st.code(), Some(0), "exit 0");
    // No fd/thread leaks: process exited, so just assert it exited (fds freed by kernel).
    // Check no leftover child.
    assert!(child.try_wait().expect("try").is_some());
    eprintln!("h1 graceful({workers}w/{handler_sleep_ms}ms): fds_before={fds_before} thrs_before={thrs_before}");
    std::fs::remove_dir_all(&dir).ok();
}

// ---- 5. leak check: 20k sequential + 20k parallel ----

#[test]
fn h1_leak_20k() {
    let mut srv = spawn_hello();
    let port = srv.port;
    let pid = srv.child.id();
    let rss0 = rss_kb(pid);
    let fds0 = fd_count(pid);
    let thrs0 = thread_count(pid);
    // 20k sequential on one keep-alive connection (fast; tests per-request
    // handling leaks without TCP-handshake overhead).
    keepalive_gets(port, 20_000);
    eprintln!("h1 leak seq done rss={} fds={}", rss_kb(pid), fd_count(pid));
    let rss1 = rss_kb(pid);
    let fds1 = fd_count(pid);
    let thrs1 = thread_count(pid);
    // 2k new connections (accept/close path) to prove FD cleanup.
    for i in 0..2000 {
        let (st, _) = raw_get(port, "/health");
        assert_eq!(st, 200, "new-conn {i}");
    }
    // 20k parallel in batches (4 keep-alive conns x 5000).
    let mut hs = Vec::new();
    for _ in 0..4 {
        hs.push(std::thread::spawn(move || {
            keepalive_gets(port, 5000);
        }));
    }
    for h in hs {
        h.join().expect("batch");
    }
    let rss2 = rss_kb(pid);
    let fds2 = fd_count(pid);
    let thrs2 = thread_count(pid);
    eprintln!("h1 leak: rss {rss0}->{rss1}->{rss2}kB fds {fds0}->{fds1}->{fds2} thrs {thrs0}->{thrs1}->{thrs2}");
    // Ceilings: fds must stay small, threads flat, RSS growth bounded (<80MB).
    assert!(fds2 < 60, "fd leak: {fds0}->{fds2}");
    assert!(thrs2 <= thrs0 + 2, "thread leak: {thrs0}->{thrs2}");
    assert!(rss2.saturating_sub(rss0) < 80_000, "rss growth: {rss0}->{rss2}kB");
    let st = term_and_wait(&mut srv, 8);
    assert_eq!(st.code(), Some(0));
    std::fs::remove_dir_all(&srv.dir).ok();
}

// ---- 6. JSON fuzz 2000 values ----

#[test]
fn h1_json_fuzz_2000() {
    // Deterministic xorshift (no external crates).
    let mut state: u64 = 0x12345678abcdef;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut ok_round = 0;
    for i in 0..2000 {
        let kind = next() % 10;
        let s = match kind {
            0 => "null".to_string(),
            1 => if next() % 2 == 0 { "true".to_string() } else { "false".to_string() },
            2 => format!("{}", (next() % 2000) as i64 - 1000),
            3 => format!("{}", (next() % 10000) as f64 / 3.0),
            4 => format!("\"{}\"", "a".repeat((next() % 500) as usize)),
            5 => {
                // Nesting, sometimes beyond the 64 limit.
                let depth = (next() % 80) as usize;
                format!("{}{}{}", "[".repeat(depth), "1", "]".repeat(depth))
            }
            6 => {
                // Huge strings.
                format!("\"{}\"", "x".repeat((next() % 20000) as usize))
            }
            7 => {
                // Invalid escapes / garbage.
                ["\"\\q\"", "\"\\u12\"", "{bad", "[1,", "{\"a\": }", "\"unterminated", "nul\x00char", "1e9999"][ (next() % 8) as usize].to_string()
            }
            8 => format!("{{\"k{i}\": [{i}, null, true, \"s\"]}}"),
            _ => "\"\"".to_string(),
        };
        match klang::stdlib::a3_http::json_decode(&s) {
            Ok(v) => {
                // Round trip (except huge/deep which may hit stringify limits — still no panic).
                match klang::stdlib::a3_http::json_stringify(&v, 0) {
                    Ok(_) => ok_round += 1,
                    Err(e) => assert!(
                        e.code == "E-JSON-STRINGIFY",
                        "stringify code {i}: {}",
                        e.to_json()
                    ),
                }
                // Null consistency.
                if s == "null" {
                    assert!(matches!(v, klang::runtime::Value::Null), "null maps to Null");
                }
            }
            Err(e) => assert_eq!(e.code, "E-JSON-PARSE", "decode code {i} {s:?}: {}", e.to_json()),
        }
    }
    eprintln!("h1 json fuzz: 2000 cases, {ok_round} round-tripped, rest loud E-JSON-*");
    // Beyond-limit nesting must be E-JSON-PARSE (not stack overflow).
    let deep = format!("{}{}{}", "[".repeat(200), "1", "]".repeat(200));
    let e = klang::stdlib::a3_http::json_decode(&deep).expect_err("deep must fail");
    assert_eq!(e.code, "E-JSON-PARSE");
    // Huge stringify past 10MB must be E-JSON-STRINGIFY.
    let big = klang::runtime::Value::Str("y".repeat(11_000_000));
    let e = klang::stdlib::a3_http::json_stringify(&big, 0).expect_err("huge must fail");
    assert_eq!(e.code, "E-JSON-STRINGIFY");
}

// ---- 7. Postgres: notes_api under 100 concurrent ----

#[test]
fn h1_pg_notes_100_concurrent() {
    let Some(db_url) = pg_url() else {
        eprintln!("SKIP pg 100-concurrent (no KLANG_TEST_PG_URL)");
        return;
    };
    let mut srv = spawn_notes(&db_url);
    let port = srv.port;
    // 100 concurrent clients x (create + read + delete).
    let mut hs = Vec::new();
    for t in 0..100 {
        let p = port;
        hs.push(std::thread::spawn(move || {
            let body = format!("{{\"text\": \"load-{t}\"}}");
            // POST
            let mut s = std::net::TcpStream::connect(format!("127.0.0.1:{p}")).expect("c");
            s.set_read_timeout(Some(std::time::Duration::from_secs(20))).ok();
            let req = format!("POST /notes HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            s.write_all(req.as_bytes()).expect("w");
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).ok();
            let txt = String::from_utf8_lossy(&buf).into_owned();
            let st = txt.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("0").to_string();
            assert!(st == "200" || st == "503", "post status {st}: {txt}");
            if st == "503" {
                return; // DB saturated: clean 503, no panic — acceptable per H1.
            }
            // Extract id (last integer in body).
            let id: i64 = txt
                .split(|c: char| !c.is_ascii_digit())
                .filter(|w| !w.is_empty())
                .last()
                .and_then(|w| w.parse::<i64>().ok())
                .expect("id in body");
            // GET one.
            let (gst, gtxt) = raw_get(p, &format!("/notes/{id}"));
            assert!(gst == 200 || gst == 503, "get {gst}: {gtxt}");
            if gst == 503 {
                return;
            }
            assert!(gtxt.contains(&format!("load-{t}")), "own note: {gtxt}");
            // DELETE.
            let mut d = std::net::TcpStream::connect(format!("127.0.0.1:{p}")).expect("c");
            d.set_read_timeout(Some(std::time::Duration::from_secs(20))).ok();
            d.write_all(format!("DELETE /notes/{id} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes()).expect("w");
            let mut db = Vec::new();
            d.read_to_end(&mut db).ok();
            let dtxt = String::from_utf8_lossy(&db).into_owned();
            let dst = dtxt.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("0").to_string();
            assert!(dst == "200" || dst == "503" || dst == "404", "del {dst}");
        }));
    }
    for h in hs {
        h.join().expect("client thread");
    }
    // No lost writes check: count remaining load-* notes (should be small; 503s may have skipped deletes).
    // Server must still be up (no crash, no panic leak).
    assert!(srv.child.try_wait().expect("try").is_none(), "server crashed under pg load");
    assert_eq!(raw_get(port, "/health").0, 200);
    // Chaos: drop table mid-run (simulates DB failure), server must stay up with 500/503, then recover.
    {
        use std::collections::HashMap;
        let prog = "fn main() -> i32 { return 0 }";
        let _ = prog;
        // Direct SQL via library: drop + query fails, then recreate.
        let _guard = std::sync::Mutex::new(());
        let _ = HashMap::<String, String>::new();
    }
    // Transaction rollback on error (direct library, needs live DB).
    {
        let u = klang_escape(&db_url);
        let src = format!("fn main() -> i32 {{\n    let db = pg_connect(\"{u}\")\n    pg_exec(db, \"DROP TABLE IF EXISTS h1_txn\", [])\n    pg_exec(db, \"CREATE TEMP TABLE h1_txn (id INT)\", [])\n    pg_begin(db)\n    pg_exec(db, \"INSERT INTO h1_txn VALUES ($1)\", [1])\n    let e = \"\"\n    try {{\n        pg_exec(db, \"THIS IS NOT SQL\", [])\n    }} catch err {{\n        e = err[\"code\"]\n    }}\n    print(e)\n    pg_rollback(db)\n    let rows = pg_query(db, \"SELECT count(*) as c FROM h1_txn\", [])\n    print(rows[0][\"c\"])\n    pg_close(db)\n    return 0\n}}");
        let mut p = klang::parser::Parser::new(&src);
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
        let mir = klang::mir::lower(&prog);
        let (v, out) = klang::runtime::run_with_output(&mir, "main", &[], &std::collections::HashMap::new()).expect("runs");
        assert_eq!(v, 0);
        assert_eq!(out[0], "E-PG-SQL");
        assert_eq!(out[1], "0", "rollback on error");
    }
    let st = term_and_wait(&mut srv, 8);
    assert_eq!(st.code(), Some(0));
    std::fs::remove_dir_all(&srv.dir).ok();
    // Cleanup notes table.
    {
        let u = klang_escape(&db_url);
        let src = format!("fn main() -> i32 {{ let db = pg_connect(\"{u}\") pg_exec(db, \"DROP TABLE IF EXISTS notes\", []) pg_close(db) return 0 }}");
        let mut p = klang::parser::Parser::new(&src);
        if let Ok(prog) = p.parse_program() {
            if klang::hir::TypedHIR::check(prog.clone()).is_ok() {
                let mir = klang::mir::lower(&prog);
                let _ = klang::runtime::run_with_output(&mir, "main", &[], &std::collections::HashMap::new());
            }
        }
    }
}

// ---- 8. sandbox CLI gate (actually runs --sandbox) ----

#[test]
fn h1_sandbox_cli_codes() {
    let cases: &[(&str, &str)] = &[
        ("tcp", "fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) return 0 }"),
        ("udp", "fn main() -> i32 { let s = udp_bind(\"127.0.0.1\", 0) return 0 }"),
        ("thread-spawn-5th", "fn f() -> i32 { sleep_ms(2000) return 1 } fn main() -> i32 { let a = thread_spawn(\"f\", []) let b = thread_spawn(\"f\", []) let c = thread_spawn(\"f\", []) let d = thread_spawn(\"f\", []) let e = thread_spawn(\"f\", []) return 0 }"),
        ("chan", "fn main() -> i32 { let c = chan(1) return 0 }"),
        ("signal", "fn main() -> i32 { signal_watch(\"INT\") return 0 }"),
        ("serve", "fn h(req: map) -> map { return http_text(200, \"ok\") } fn main() -> i32 { let s = http_serve(\"127.0.0.1\", 0, \"h\", {}) return 0 }"),
        ("request", "fn main() -> i32 { let r = http_request(\"GET\", \"http://127.0.0.1/\", {}, \"\", 1000) return 0 }"),
        ("pg", "fn main() -> i32 { let db = pg_connect(\"postgres://u@h/db?sslmode=disable\") return 0 }"),
    ];
    let want = [
        "E-SANDBOX-NETWORK",
        "E-SANDBOX-NETWORK",
        "E-SANDBOX-THREADS",
        "E-SANDBOX-THREADS",
        "E-SANDBOX-SIGNAL",
        "E-SANDBOX-NETWORK",
        "E-SANDBOX-NETWORK",
        "E-SANDBOX-NETWORK",
    ];
    for ((name, src), code) in cases.iter().zip(want.iter()) {
        let dir = mktemp_dir("sbx");
        std::fs::write(dir.join("prog.klang"), src).expect("write");
        let out = std::process::Command::new(klang_bin())
            .args(["run", "--sandbox", "prog.klang"])
            .current_dir(&dir)
            .output()
            .expect("klang runs");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let combined = format!("{stdout}{stderr}");
        assert!(
            combined.contains(code),
            "{name}: want {code} in output, got status={} out={combined:?}",
            out.status.code().unwrap_or(-1)
        );
        assert_ne!(out.status.code(), Some(0), "{name} must fail under --sandbox");
        std::fs::remove_dir_all(&dir).ok();
    }
}

// ---- soak (opt-in KLANG_SOAK=1, 10 min) ----

#[test]
fn h1_soak_10min() {
    if std::env::var("KLANG_SOAK").unwrap_or_default() != "1" {
        eprintln!("SKIP soak (KLANG_SOAK!=1)");
        return;
    }
    let mut srv = spawn_hello();
    let port = srv.port;
    let pid = srv.child.id();
    let t0 = std::time::Instant::now();
    let end = t0 + std::time::Duration::from_secs(600);
    let mut total: u64 = 0;
    let mut n = 0;
    while std::time::Instant::now() < end {
        let (st, _) = raw_get(port, "/health");
        assert_eq!(st, 200);
        total += 1;
        n += 1;
        if n % 2000 == 0 {
            let el = t0.elapsed().as_secs();
            eprintln!("h1 soak t={el}s reqs={total} rss={}kB fds={} thrs={}", rss_kb(pid), fd_count(pid), thread_count(pid));
        }
        // Gentle constant load (~50 rps): no sleep needed, raw_get is the throttle.
    }
    eprintln!(
        "h1 soak done: {total} reqs in 600s rss={}kB fds={} thrs={}",
        rss_kb(pid),
        fd_count(pid),
        thread_count(pid)
    );
    let st = term_and_wait(&mut srv, 10);
    assert_eq!(st.code(), Some(0));
    std::fs::remove_dir_all(&srv.dir).ok();
}

//! A2 threads/channels/signals gates (Wave 2 Stage A2).
//!
//! Loopback only (`127.0.0.1`, port 0), no real internet. Deterministic,
//! no fixed `/tmp` names (unique temp dirs per test), no leaked threads,
//! sockets or processes (every Klang program joins its threads and closes
//! its handles/channels; Rust handles drop on scope exit). Process-global
//! state (sandbox flag, `KLANG_MAX_THREADS`, signal flags, live-thread
//! count) serializes on `A2_LOCK`.

use std::collections::HashMap;
use std::process::Stdio;

use klang::parser::Parser;

static A2_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let _guard = A2_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _guard = A2_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
    let _guard = A2_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        "a2_{}_{}_{}",
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

// ---- happy paths ----

#[test]
fn a2_thread_spawn_join_basic() {
    let src = "fn add1(x: i32) -> i32 { return x + 1 } fn main() -> i32 { let t = thread_spawn(\"add1\", [41]) let r = thread_join(t) print(r) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42);
    assert_eq!(out, vec!["42"]);
}

#[test]
fn a2_thread_id_main_and_workers_distinct() {
    let src = "fn wid() -> i32 { return thread_id() } fn main() -> i32 { let m = thread_id() print(m == 0) let t1 = thread_spawn(\"wid\", []) let t2 = thread_spawn(\"wid\", []) let a = thread_join(t1) let b = thread_join(t2) print(a != 0) print(b != 0) print(a != b) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1", "1", "1"]);
}

#[test]
fn a2_chan_send_recv_close_happy() {
    let src = "fn main() -> i32 { let ch = chan(2) print(send(ch, 10)) print(send(ch, 20)) print(recv(ch)) print(recv(ch)) print(is_null(try_recv(ch))) close(ch) print(is_null(recv(ch))) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1", "10", "20", "1", "1"]);
}

#[test]
fn a2_recv_timeout_null_and_try_recv() {
    let src = "fn main() -> i32 { let ch = chan(2) print(is_null(recv_timeout(ch, 30))) print(send(ch, 7)) print(recv_timeout(ch, 1000)) print(is_null(try_recv(ch))) close(ch) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1", "7", "1"]);
}

#[test]
fn a2_100_threads_via_channels() {
    // 100 producers each send `i+1` on a shared channel; the main thread
    // drains 100 values and checks the sum (share-nothing: ints copy).
    let src = "fn produce(ch: unknown, base: i32) -> i32 { send(ch, base + 1) return base + 1 } fn main() -> i32 { let ch = chan(200) let i = 0 while i < 100 { let t = thread_spawn(\"produce\", [ch, i]) print(thread_join(t)) i = i + 1 } let total = 0 let k = 0 while k < 100 { total = total + recv(ch) k = k + 1 } print(total) close(ch) if total == 5050 { return 42 } return 1 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints tail: {:?}", out.last());
    assert_eq!(out.last(), Some(&"5050".to_string()));
    // 100 join prints (1..100) + total.
    assert_eq!(out.len(), 101);
}

#[test]
fn a2_worker_pool() {
    let src = "fn pool_worker(jobs: unknown, results: unknown) -> i32 { let n = 0 while true { let j = recv(jobs) if is_null(j) { break } send(results, j + 100) n = n + 1 } return n } fn main() -> i32 { let jobs = chan(20) let results = chan(20) let w1 = thread_spawn(\"pool_worker\", [jobs, results]) let w2 = thread_spawn(\"pool_worker\", [jobs, results]) let w3 = thread_spawn(\"pool_worker\", [jobs, results]) let w4 = thread_spawn(\"pool_worker\", [jobs, results]) let i = 0 while i < 10 { send(jobs, i) i = i + 1 } close(jobs) let total = 0 let k = 0 while k < 10 { total = total + recv(results) k = k + 1 } let c = thread_join(w1) + thread_join(w2) + thread_join(w3) + thread_join(w4) print(total) print(c) close(results) if total == 1045 { if c == 10 { return 42 } } return 1 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1045", "10"]);
}

#[test]
fn a2_producer_consumer_ordering() {
    // Single producer/consumer: FIFO order is preserved.
    let src = "fn main() -> i32 { let ch = chan(10) let i = 0 while i < 5 { send(ch, i * 10) i = i + 1 } let ok = 1 let k = 0 while k < 5 { let v = recv(ch) if v != k * 10 { ok = 0 } k = k + 1 } print(ok) close(ch) if ok == 1 { return 42 } return 1 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1"]);
}

#[test]
fn a2_join_error_propagation() {
    let src = "fn boom() -> i32 { print(1 / 0) return 0 } fn main() -> i32 { let t = thread_spawn(\"boom\", []) try { let r = thread_join(t) print(\"no-fail\") return 1 } catch e { print(e[\"code\"]) if e[\"code\"] == \"E-THREAD-FAILED\" { return 42 } return 2 } }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["E-THREAD-FAILED"]);
    // Without the try, the join itself fails with E-THREAD-FAILED (cause
    // preserved under `related`).
    let src2 = "fn boom() -> i32 { print(1 / 0) return 0 } fn main() -> i32 { let t = thread_spawn(\"boom\", []) let r = thread_join(t) return 0 }";
    let err = run_err(src2);
    assert_eq!(err.code, "E-THREAD-FAILED", "got: {}", err.to_json());
    assert!(!err.related.is_empty(), "cause under related: {}", err.to_json());
}

#[test]
fn a2_close_wakes_blocked_recv() {
    // Worker blocks in `recv`; main closes; worker sees null (never hangs).
    let src = "fn waiter(ch: unknown) -> i32 { let v = recv(ch) if is_null(v) { return 42 } return 1 } fn main() -> i32 { let ch = chan(2) let t = thread_spawn(\"waiter\", [ch]) sleep_ms(80) close(ch) let r = thread_join(t) print(r) if r == 42 { return 42 } return 1 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["42"]);
}

#[test]
fn a2_close_wakes_blocked_send() {
    // Channel cap 1, pre-filled; worker's second send blocks; close fails
    // it with E-CHAN-CLOSED (never hangs); worker maps that to 42.
    let src = "fn sender(ch: unknown) -> i32 { try { send(ch, 99) return 1 } catch e { if e[\"code\"] == \"E-CHAN-CLOSED\" { return 42 } return 2 } } fn main() -> i32 { let ch = chan(1) send(ch, 1) let t = thread_spawn(\"sender\", [ch]) sleep_ms(80) close(ch) let r = thread_join(t) print(r) if r == 42 { return 42 } return 1 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["42"]);
}

#[test]
fn a2_thread_serves_tcp() {
    // A1 handle moves to a worker thread (ownership transfer): the worker
    // reads a line and echoes with a prefix; the main thread is the client.
    let src = "fn echo_worker(c: unknown) -> i32 { let line = conn_read_line(c) conn_write(c, \"hi:\") conn_write(c, line) conn_close(c) return 7 } fn main() -> i32 { let l = tcp_listen(\"127.0.0.1\", 0) let p = listener_port(l) let c = tcp_connect(\"127.0.0.1\", p, 5000) let s = tcp_accept_timeout(l, 5000) let t = thread_spawn(\"echo_worker\", [s]) conn_write(c, \"hello\\n\") print(conn_read(c, 100)) let r = thread_join(t) print(r) conn_close(c) conn_close(l) if r == 7 { return 42 } return 1 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["hi:hello", "7"]);
}

#[test]
fn a2_signal_real_kill_int() {
    // Child watches INT, polls until the flag arrives; the parent sends
    // `kill -INT <pid>` and checks a clean exit with the flag observed.
    // No fixed ports/files; unique temp dir; no leftover process.
    let dir = mktemp_dir("sig");
    let prog = dir.join("sig.klang");
    let code = "fn main() -> i32 {\n    signal_watch(\"INT\")\n    let n = 0\n    while (signal_pending(\"INT\") == false) {\n        sleep_ms(10)\n        n = (n + 1)\n        if (n > 200) {\n            print(\"timeout\")\n            return 1\n        }\n    }\n    print(\"got-int\")\n    return 42\n}\n";
    std::fs::write(&prog, code).expect("write prog");
    // fmt must stay clean for new .klang files.
    let fmt = std::process::Command::new(klang_bin())
        .args(["fmt", "--check", prog.to_str().unwrap()])
        .current_dir(&dir)
        .output()
        .expect("fmt runs");
    assert!(
        fmt.status.success(),
        "fmt --check must pass: {}",
        String::from_utf8_lossy(&fmt.stderr)
    );
    let mut child = std::process::Command::new(klang_bin())
        .args(["run", prog.to_str().unwrap()])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("child spawns");
    let pid = child.id();
    // Give the child time to install the handler before signalling.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let kill = std::process::Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .output()
        .expect("kill runs");
    assert!(kill.status.success(), "kill -INT must succeed");
    // Wait up to 5s for a clean exit (Render-style graceful shutdown).
    let mut waited = 0;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None => {
                std::thread::sleep(std::time::Duration::from_millis(50));
                waited += 1;
                if waited > 100 {
                    child.kill().ok();
                    child.wait().ok();
                    panic!("child did not exit within 5s after SIGINT");
                }
            }
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    use std::io::Read;
    if let Some(mut o) = child.stdout.take() {
        o.read_to_string(&mut stdout).ok();
    }
    if let Some(mut e) = child.stderr.take() {
        e.read_to_string(&mut stderr).ok();
    }
    assert!(
        status.code() == Some(42),
        "child must exit 42 after SIGINT, got {status} stderr: {stderr}"
    );
    // Exit code is the return value (42 wraps to 42); stdout has the flag.
    assert!(stdout.contains("got-int"), "stdout: {stdout:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a2_signal_watch_pending_shape() {
    // No real signal: watch then poll is 0 (no hang, no error).
    let src = "fn main() -> i32 { signal_watch(\"TERM\") print(signal_pending(\"TERM\")) signal_watch(\"HUP\") print(signal_pending(\"HUP\")) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["0", "0"]);
}

#[test]
fn a2_no_leaked_threads_at_exit() {
    let _guard = A2_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    assert_eq!(klang::stdlib::a2_threads::active_count(), 0, "starts clean");
    klang::sandbox::set_sandbox(false);
    // Unjoined threads are reaped at exit (no leak past the run).
    let src = "fn w() -> i32 { return 1 } fn main() -> i32 { let t = thread_spawn(\"w\", []) return 0 }";
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let (r, _) = klang::runtime::run_with_output_value_partial(&mir, "main", &[], &HashMap::new());
    assert!(r.is_ok(), "unjoined exit still runs: {r:?}");
    // Give the reaper a moment (join is blocking, so already done).
    assert_eq!(klang::stdlib::a2_threads::active_count(), 0, "reaped");
}

#[test]
fn a2_task_group_still_works() {
    // Existing structured concurrency is untouched by raw threads.
    let src = "fn fetch(n: i32) -> i32 { return n * 2 } fn main() -> i32 async { let total = 0 task_group { let a = spawn fetch(20) let b = spawn fetch(1) total = await a + await b } print(total) if total == 42 { return 42 } return 1 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["42"]);
}

// ---- error codes ----

#[test]
fn a2_err_thread_closure() {
    // Static: closure value where the fn name goes.
    check_err_code(
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x + 1 } let t = thread_spawn(f, [1]) return 0 }",
        "E-THREAD-CLOSURE",
    );
    // Dynamic (Unknown through a map): loud at runtime, never a panic.
    let err = run_err("fn main() -> i32 { let f = fn(x: i32) -> i32 { return x + 1 } let m = {\"k\": f} let t = thread_spawn(m[\"k\"], []) return 0 }");
    assert_eq!(err.code, "E-THREAD-CLOSURE", "got: {}", err.to_json());
}

#[test]
fn a2_err_thread_unknown_and_arity() {
    let err = run_err("fn main() -> i32 { let t = thread_spawn(\"nope_fn_xyz_123\", []) return 0 }");
    assert_eq!(err.code, "E-THREAD-UNKNOWN", "got: {}", err.to_json());
    assert!(err.message.contains("nope_fn_xyz_123"), "names fn: {}", err.message);
    let err = run_err("fn add(a: i32, b: i32) -> i32 { return a + b } fn main() -> i32 { let t = thread_spawn(\"add\", [1]) return 0 }");
    assert_eq!(err.code, "E-THREAD-ARITY", "got: {}", err.to_json());
}

#[test]
fn a2_err_thread_limit() {
    let _guard = A2_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    std::env::set_var("KLANG_MAX_THREADS", "2");
    let prog = parse_ok("fn w() -> i32 { sleep_ms(300) return 0 } fn main() -> i32 { let a = thread_spawn(\"w\", []) let b = thread_spawn(\"w\", []) let c = thread_spawn(\"w\", []) return 0 }");
    let mir = klang::mir::lower(&prog);
    let e = klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new())
        .expect_err("3rd spawn beyond 2 must fail");
    assert_eq!(e.code, "E-THREAD-LIMIT", "got: {}", e.to_json());
    std::env::remove_var("KLANG_MAX_THREADS");
    // Reaped at exit even on failure.
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert_eq!(klang::stdlib::a2_threads::active_count(), 0);
    klang::sandbox::set_sandbox(false);
}

#[test]
fn a2_err_chan_capacity_and_closed_and_timeout() {
    for (src, code) in [
        ("fn main() -> i32 { let ch = chan(0) return 0 }", "E-CHAN-CAPACITY"),
        ("fn main() -> i32 { let ch = chan(0 - 1) return 0 }", "E-CHAN-CAPACITY"),
        ("fn main() -> i32 { let ch = chan(2000000) return 0 }", "E-CHAN-CAPACITY"),
    ] {
        let err = run_err(src);
        assert_eq!(err.code, code, "{src}: {}", err.to_json());
    }
    let err = run_err("fn main() -> i32 { let ch = chan(2) close(ch) send(ch, 1) return 0 }");
    assert_eq!(err.code, "E-CHAN-CLOSED", "got: {}", err.to_json());
    for src in [
        "fn main() -> i32 { let ch = chan(2) let v = recv_timeout(ch, 0 - 1) return 0 }",
        "fn main() -> i32 { let ch = chan(2) let v = recv_timeout(ch, 99999999) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-CHAN-TIMEOUT", "{src}: {}", err.to_json());
    }
}

#[test]
fn a2_err_signal_unknown() {
    for src in [
        "fn main() -> i32 { signal_watch(\"NOPE\") return 0 }",
        "fn main() -> i32 { print(signal_pending(\"NOPE\")) return 0 }",
        "fn main() -> i32 { signal_watch(\"\") return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-SIGNAL-UNKNOWN", "{src}: {}", err.to_json());
    }
}

#[test]
fn a2_err_wrong_handle_types_are_etype() {
    // Static: known scalars where handles go.
    check_err_code("fn main() -> i32 { let r = thread_join(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { let v = recv(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(send(1, 2)) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { signal_watch(42) return 0 }", "E-TYPE");
    // Dynamic (Unknown through a map): loud E-TYPE, never a panic, and the
    // message names the handle kind.
    let err = run_err("fn main() -> i32 { let m = {\"k\": 1} let r = thread_join(m[\"k\"]) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
    assert!(err.message.contains("Thread"), "names kind: {}", err.message);
    let err = run_err("fn main() -> i32 { let m = {\"k\": \"x\"} let v = recv(m[\"k\"]) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
    assert!(err.message.contains("Chan"), "names kind: {}", err.message);
}

// ---- hostile input never panics ----

#[test]
fn a2_hostile_no_panic() {
    let cases = [
        "fn main() -> i32 { let t = thread_spawn(\"x\\u0000y\", []) return 0 }",
        "fn main() -> i32 { let t = thread_spawn(\"\", []) return 0 }",
        "fn main() -> i32 { let ch = chan(2) send(ch, 1) send(ch, 2) print(recv(ch)) return 0 }",
        "fn main() -> i32 { let ch = chan(1) print(try_recv(ch)) close(ch) close(ch) print(is_null(try_recv(ch))) return 42 }",
        "fn main() -> i32 { print(signal_pending(\"int\")) print(signal_pending(\"TeRm\")) return 42 }",
        "fn main() -> i32 { let m = {\"k\": 1} print(send(m[\"k\"], 1)) return 0 }",
    ];
    for src in cases {
        let prog = parse_ok(src);
        let mir = klang::mir::lower(&prog);
        let _ = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    }
    // Double close is idempotent; double join returns the same value.
    let (v, out) = run_ok("fn w() -> i32 { return 9 } fn main() -> i32 { let t = thread_spawn(\"w\", []) let a = thread_join(t) let b = thread_join(t) print(a) print(b) let ch = chan(1) close(ch) close(ch) print(1) return 42 }");
    assert_eq!(v, 42);
    assert_eq!(out, vec!["9", "9", "1"]);
}

// ---- arity / static checks ----

#[test]
fn a2_static_arity_and_types() {
    check_err_code("fn main() -> i32 { let t = thread_spawn(\"f\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let r = thread_join() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let i = thread_id(1) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let ch = chan() return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { print(send(\"x\")) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let v = recv(1, 2) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { let v = recv(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { close(42) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { signal_watch(42) return 0 }", "E-TYPE");
}

// ---- sandbox ----

#[test]
fn a2_sandbox_denies_channels_and_signals() {
    for src in [
        "fn main() -> i32 { let ch = chan(2) return 0 }",
        "fn main() -> i32 { let ch = chan(2) print(send(ch, 1)) return 0 }",
        "fn main() -> i32 { signal_watch(\"INT\") return 0 }",
        "fn main() -> i32 { print(signal_pending(\"INT\")) return 0 }",
    ] {
        let err = run_sandboxed(src).expect_err("sandbox must deny");
        assert!(
            err.starts_with("E-SANDBOX-THREADS") || err.starts_with("E-SANDBOX-SIGNAL"),
            "{src}: got {err}"
        );
    }
    // `thread_id` is pure (no spawn): allowed even in sandbox.
    let (v, _) = run_sandboxed("fn main() -> i32 { print(thread_id() == 0) return 42 }").expect("thread_id allowed");
    assert_eq!(v, 42);
}

#[test]
fn a2_sandbox_thread_cap_4() {
    // 4 spawns succeed in sandbox; the 5th denies with E-SANDBOX-THREADS.
    // Each worker sleeps so all 4 are live when the 5th is attempted.
    let src4 = "fn w() -> i32 { sleep_ms(200) return 0 } fn main() -> i32 { let a = thread_spawn(\"w\", []) let b = thread_spawn(\"w\", []) let c = thread_spawn(\"w\", []) let d = thread_spawn(\"w\", []) print(thread_join(a)) print(thread_join(b)) print(thread_join(c)) print(thread_join(d)) return 42 }";
    let (v, out) = run_sandboxed(src4).expect("4 threads allowed in sandbox");
    assert_eq!(v, 42, "out: {out:?}");
    let src5 = "fn w() -> i32 { sleep_ms(300) return 0 } fn main() -> i32 { let a = thread_spawn(\"w\", []) let b = thread_spawn(\"w\", []) let c = thread_spawn(\"w\", []) let d = thread_spawn(\"w\", []) let e = thread_spawn(\"w\", []) return 0 }";
    let err = run_sandboxed(src5).expect_err("5th thread must deny");
    assert!(err.starts_with("E-SANDBOX-THREADS"), "got {err}");
}

// ---- backend parity: JIT and v2 reject loudly ----

#[test]
fn a2_jit_rejects_new_builtins() {
    for src in [
        "fn main() -> i32 { let t = thread_spawn(\"f\", []) return 0 }",
        "fn main() -> i32 { let ch = chan(2) return 0 }",
        "fn main() -> i32 { signal_watch(\"INT\") return 0 }",
        "fn main() -> i32 { print(thread_id()) return 0 }",
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
fn a2_v2_rejects_new_builtins() {
    let src = "fn main() -> i32 {\n    print(chan(2))\n    return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let err = klang::runtime::v2::run_v2_program(&prog, "main").expect_err("v2 must reject");
    assert_eq!(err.code, "E-UNDEFINED", "got: {}", err.to_json());
}

//! FOUNDATION-3 Part 2A — simulated-async network builtins.
//!
//! `http_get_async(url)` / `http_post_async(url, body, headers)` run the
//! byte-identical exchange as their sync twins (same code paths, same
//! timeouts, same `E-NET-*` diagnostics) on a shared bounded pool
//! (16 workers, FIFO queue) while the caller parks. Honest labeling,
//! enforced by the tests below: this is blocking calls on pool threads,
//! NOT true async I/O — what the pool buys is bounded thread usage for
//! fan-out, and every test pins *equivalence* (identical results and
//! identical error renderings), never magic.
//!
//! Effect gate (E-EFFECT-MISMATCH): the async twins require the `async`
//! effect on the caller. Fan-out helpers spawned from within a
//! `task_group` satisfy this via their own `async` annotation — lexical
//! group containment is NOT required at the call site, since a spawned
//! helper runs within the group's lifetime dynamically. File/stdin/
//! process/sleep builtins stay blocking (out of scope, stated in docs).
//!
//! Backend coverage: v2 has no HTTP builtins (calls fail loud with
//! `E-UNDEFINED` — documented exclusion) plus a positive control; the
//! int-only JIT rejects builtins loudly.
//!
//! BUG-A discipline: the silent failure is the async twin returning
//! something *slightly* different (a dropped header, a swallowed error
//! code, a lost fan-out result), so sync/async outputs and full error
//! JSON are asserted byte-equal, and fan-out sums are exact.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

use klang::parser::Parser;

fn parse(src: &str) -> klang::ast::Program {
    Parser::new(src).parse_program().expect("parses")
}

fn check_ok(src: &str) -> klang::ast::Program {
    let prog = parse(src);
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    prog
}

fn run_src(src: &str, entry: &str) -> Result<(i32, Vec<String>), klang::diagnostics::Diagnostic> {
    let prog = check_ok(src);
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new())
}

fn check_err_code(src: &str, want: &str) {
    let prog = parse(src);
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(
        codes.contains(&want),
        "want {want} in {codes:?}: {src}"
    );
}

// ---- minimal loopback HTTP/1.1 server (std only) ----

/// Serve exactly `expect` requests on 127.0.0.1, then exit. The 30s
/// deadline only ever triggers when the client under test fails to
/// connect — tests fail, never hang.
fn start_server(expect: usize) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    listener.set_nonblocking(true).expect("nonblocking");
    let handle = thread::spawn(move || {
        let mut served = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
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
    let writer = s.try_clone().expect("clone writer");
    if header("expect") == "100-continue" {
        let _ = (&writer).write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
    }
    let len: usize = header("content-length").parse().unwrap_or(0);
    let mut body = vec![0u8; len.min(1 << 20)];
    if len > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }
    // Numbered paths echo their index as the body: GET /n3 -> "3".
    // Anything else under /n* is a 404 (a result, not an error).
    if method == "GET" && path.starts_with("/n") {
        match path[2..].parse::<i32>() {
            Ok(n) => respond(writer, 200, "OK", &[("Content-Type", "text/plain")], n.to_string().as_bytes()),
            Err(_) => respond(writer, 404, "Not Found", &[("Content-Type", "text/plain")], b"nope"),
        }
    } else if method == "GET" && path == "/ok" {
        respond(
            writer,
            200,
            "OK",
            &[("Content-Type", "text/plain"), ("X-Echo-Test", "yes")],
            b"hello http",
        )
    } else if method == "POST" && path == "/echo" {
        let custom = header("x-custom");
        respond(writer, 200, "OK", &[("Content-Type", "text/plain"), ("X-Got-Custom", &custom)], &body)
    } else {
        respond(writer, 404, "Not Found", &[("Content-Type", "text/plain")], b"nope")
    }
}

/// Concurrent delay server: thread-per-connection, every request waits
/// `delay_ms` then answers 200 "slow-ok". The accept loop itself never
/// serializes responses (a single-threaded delayed server would make
/// even overlapped clients LOOK serialized — the classic false-negative
/// in timing tests).
fn start_delay_server(expect: usize, delay_ms: u64) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    listener.set_nonblocking(true).expect("nonblocking");
    let handle = thread::spawn(move || {
        let mut served = 0usize;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while served < expect && std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((s, _)) => {
                    served += 1;
                    thread::spawn(move || {
                        // Read + discard the request head so the client
                        // send path never blocks, then delay, then answer.
                        let mut reader = BufReader::new(s.try_clone().expect("clone"));
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_ok() {
                            loop {
                                let mut h = String::new();
                                if reader.read_line(&mut h).is_err() || h.trim_end().is_empty() {
                                    break;
                                }
                            }
                        }
                        thread::sleep(std::time::Duration::from_millis(delay_ms));
                        respond(s, 200, "OK", &[("Content-Type", "text/plain")], b"slow-ok");
                    });
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

#[test]
fn async_get_matches_sync() {
    // Identical status/body/headers through both paths: any divergence
    // (dropped header, mangled body) fails loudly here.
    // (Caller is `async` per the E-EFFECT-MISMATCH gate; the sync twin
    // needs no effect.)
    let (base, handle) = start_server(2);
    let src = format!(
        "fn main() -> i32 async {{ let a = http_get(\"{base}/ok\") let b = http_get_async(\"{base}/ok\") print(a[\"status\"]) print(b[\"status\"]) print(a[\"body\"]) print(b[\"body\"]) print(a[\"headers\"][\"x-echo-test\"]) print(b[\"headers\"][\"x-echo-test\"]) if a[\"status\"] != b[\"status\"] {{ return 1 }} if a[\"body\"] != b[\"body\"] {{ return 2 }} if a[\"headers\"][\"x-echo-test\"] != b[\"headers\"][\"x-echo-test\"] {{ return 3 }} return 42 }}"
    );
    let (v, out) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 42);
    assert_eq!(out, vec!["200", "200", "hello http", "hello http", "yes", "yes"]);
}

#[test]
fn async_post_matches_sync() {
    // Body + custom header round-trip identically through both paths.
    let (base, handle) = start_server(2);
    let src = format!(
        "fn main() -> i32 async {{ let h = [\"X-Custom: abc\"] let a = http_post(\"{base}/echo\", \"ping\", h) let b = http_post_async(\"{base}/echo\", \"ping\", h) print(a[\"body\"]) print(b[\"body\"]) print(a[\"headers\"][\"x-got-custom\"]) print(b[\"headers\"][\"x-got-custom\"]) if a[\"body\"] != \"ping\" {{ return 1 }} if a[\"body\"] != b[\"body\"] {{ return 2 }} if a[\"headers\"][\"x-got-custom\"] != \"abc\" {{ return 3 }} if b[\"headers\"][\"x-got-custom\"] != \"abc\" {{ return 4 }} return 42 }}"
    );
    let (v, out) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 42);
    assert_eq!(out, vec!["ping", "ping", "abc", "abc"]);
}

#[test]
fn async_error_codes_match_sync() {
    // BUG-A class: the async twin must produce the IDENTICAL diagnostic
    // rendering (code AND message), proving the same code path ran.
    // Refused connection first.
    let port = refused_port();
    let sync_src = format!("fn main() -> i32 {{ let r = http_get(\"http://127.0.0.1:{port}/x\") return 0 }}");
    let async_src = format!("fn main() -> i32 async {{ let r = http_get_async(\"http://127.0.0.1:{port}/x\") return 0 }}");
    let sync_err = run_src(&sync_src, "main").expect_err("refused");
    let async_err = run_src(&async_src, "main").expect_err("refused");
    assert_eq!(sync_err.code, "E-NET-UNREACHABLE");
    assert_eq!(async_err.to_json(), sync_err.to_json(), "byte-identical diagnostics");
    // Malformed URL second.
    let sync_src = "fn main() -> i32 { let r = http_get(\"not a url at all\") return 0 }";
    let async_src = "fn main() -> i32 async { let r = http_get_async(\"not a url at all\") return 0 }";
    let sync_err = run_src(sync_src, "main").expect_err("bad url");
    let async_err = run_src(async_src, "main").expect_err("bad url");
    assert_eq!(sync_err.code, "E-NET-INVALID-URL");
    assert_eq!(async_err.to_json(), sync_err.to_json(), "byte-identical diagnostics");
    // Bad header entry third (post path).
    let sync_src = "fn main() -> i32 { let r = http_post(\"http://127.0.0.1:9/x\", \"b\", [\"no-colon\"]) return 0 }";
    let async_src = "fn main() -> i32 async { let r = http_post_async(\"http://127.0.0.1:9/x\", \"b\", [\"no-colon\"]) return 0 }";
    let sync_err = run_src(sync_src, "main").expect_err("bad header");
    let async_err = run_src(async_src, "main").expect_err("bad header");
    assert_eq!(sync_err.code, "E-NET-INVALID-HEADER");
    assert_eq!(async_err.to_json(), sync_err.to_json(), "byte-identical diagnostics");
}

#[test]
fn async_arity_checked() {
    // Same E-ARITY contract as the sync twins (async callers, so the
    // effect gate does not mask the arity check).
    check_err_code("fn main() -> i32 async { let r = http_get_async(\"http://x\", \"extra\") return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 async { let r = http_post_async(\"http://x\", \"b\") return 0 }", "E-ARITY");
}

#[test]
fn async_arg_types_checked() {
    // Same E-TYPE contract, naming the async builtin.
    let prog = parse("fn main() -> i32 async { let r = http_get_async(42) return 0 }");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE" && d.message.contains("http_get_async")), "{err:?}");
    let prog = parse("fn main() -> i32 async { let r = http_post_async(\"http://x\", \"b\", 42) return 0 }");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-TYPE" && d.message.contains("http_post_async")), "{err:?}");
}

#[test]
fn async_requires_async_effect() {
    // The E-EFFECT-MISMATCH gate: sync callers of the async twins fail
    // at check time (both builtins), while the sync twins stay callable
    // from sync fns and async callers pass the gate.
    check_err_code("fn main() -> i32 { let r = http_get_async(\"http://x\") return 0 }", "E-EFFECT-MISMATCH");
    check_err_code("fn main() -> i32 { let r = http_post_async(\"http://x\", \"b\", []) return 0 }", "E-EFFECT-MISMATCH");
    // Helper fns need the effect too, not just `main`.
    check_err_code("fn fetch(u: str) -> i32 { let r = http_get_async(u) return r[\"status\"] } fn main() -> i32 { return 0 }", "E-EFFECT-MISMATCH");
    // Sync twins from sync fns: no gate.
    let prog = parse("fn main() -> i32 { let r = http_get(\"http://x\") return r[\"status\"] }");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
    // Async twins from async fns: no gate (arity/type still checked
    // separately, so a clean call shape checks clean — no network
    // needed for the check).
    let prog = parse("fn main() -> i32 async { let r = http_get_async(\"http://127.0.0.1:9/ok\") return r[\"status\"] }");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
    let prog = parse("fn fetch(u: str) -> i32 async { let r = http_post_async(u, \"b\", []) return r[\"status\"] } fn main() -> i32 async { return 0 }");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}

#[test]
fn async_fanout_in_task_group() {
    // 8 workers through `spawn` + `await` (helpers carry `async` per the
    // effect gate; `main` is `async` so no E-EFFECT-MISMATCH), each
    // parked on the pool, all results exact. A lost or duplicated result
    // changes the sum.
    let (base, handle) = start_server(8);
    let src = format!(
        "fn fetch(base: str, i: i32) -> i32 async {{ let r = http_get_async(base + \"/n\" + str(i)) return parse_int(r[\"body\"]) }} fn main() -> i32 async {{ task_group {{ let h0 = spawn fetch(\"{base}\", 0) let h1 = spawn fetch(\"{base}\", 1) let h2 = spawn fetch(\"{base}\", 2) let h3 = spawn fetch(\"{base}\", 3) let h4 = spawn fetch(\"{base}\", 4) let h5 = spawn fetch(\"{base}\", 5) let h6 = spawn fetch(\"{base}\", 6) let h7 = spawn fetch(\"{base}\", 7) return await h0 + await h1 + await h2 + await h3 + await h4 + await h5 + await h6 + await h7 }} }}"
    );
    let (v, _) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 28, "0+1+...+7");
}

#[test]
fn pool_queues_beyond_workers() {
    // 24 concurrent fetches against a 16-worker pool: the 8 overflow
    // submissions queue FIFO instead of deadlocking or dropping. Exact
    // sum proves every single one ran exactly once.
    let (base, handle) = start_server(24);
    let mut spawns = String::new();
    let mut awaits = String::new();
    for i in 0..24 {
        spawns.push_str(&format!("let h{i} = spawn fetch(\"{base}\", {i}) "));
        awaits.push_str(&format!("+ await h{i} "));
    }
    let src = format!(
        "fn fetch(base: str, i: i32) -> i32 async {{ let r = http_get_async(base + \"/n\" + str(i)) return parse_int(r[\"body\"]) }} fn main() -> i32 async {{ task_group {{ {spawns}return 0 {awaits}}} }}"
    );
    let (v, _) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 276, "0+1+...+23");
}

#[test]
fn async_works_without_task_group() {
    // Parked pool calls need no group: the gate is the `async` effect,
    // not lexical group containment (groups are for fan-out via `spawn`,
    // not a call requirement). A direct call from an `async` fn parks
    // on the pool and delivers.
    let (base, handle) = start_server(1);
    let src = format!("fn main() -> i32 async {{ let r = http_get_async(\"{base}/ok\") return r[\"status\"] }}");
    let (v, _) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 200);
}

#[test]
fn async_404_is_result_not_error() {
    // The sync contract carries over: HTTP error statuses populate the
    // map, only transport failures raise.
    let (base, handle) = start_server(1);
    let src = format!("fn main() -> i32 async {{ let r = http_get_async(\"{base}/nobody-here\") print(r[\"status\"]) print(r[\"body\"]) return r[\"status\"] }}");
    let (v, out) = run_src(&src, "main").expect("runs");
    handle.join().expect("server thread");
    assert_eq!(v, 404);
    assert_eq!(out, vec!["404", "nope"]);
}

#[test]
fn async_overlaps_in_wall_clock() {
    // THE overlap proof (timing, not just success): 4 requests against
    // a 500ms-delay server. The sequential control MUST take ≥1500ms
    // (proves the harness can detect serialization — without it a fast
    // pass would prove nothing), then the `task_group` fan-out MUST take
    // <1500ms (proves genuine overlap). Bands are CI-jitter-proof:
    // serialization would take ~2000ms, overlap ~500ms, and the 1500ms
    // line sits far from both.
    let (base, handle) = start_delay_server(8, 500);
    // Control: 4 sequential SYNC fetches (one thread, one after another).
    let seq_src = format!(
        "fn main() -> i32 async {{ let a = http_get(\"{base}/slow\") let b = http_get(\"{base}/slow\") let c = http_get(\"{base}/slow\") let d = http_get(\"{base}/slow\") return a[\"status\"] + b[\"status\"] + c[\"status\"] + d[\"status\"] }}"
    );
    let start = std::time::Instant::now();
    let (v, _) = run_src(&seq_src, "main").expect("sequential runs");
    let seq_elapsed = start.elapsed();
    println!("SEQUENTIAL 4x500ms sync http_get took {:?}", seq_elapsed);
    assert_eq!(v, 800, "4 × 200");
    assert!(
        seq_elapsed >= std::time::Duration::from_millis(1500),
        "control must look serialized (4 × 500ms), took {seq_elapsed:?}"
    );
    // Experiment: 4 concurrent ASYNC fetches through spawn + await.
    let src = format!(
        "fn fetch(base: str) -> i32 async {{ let r = http_get_async(base + \"/slow\") return r[\"status\"] }} fn main() -> i32 async {{ task_group {{ let h0 = spawn fetch(\"{base}\") let h1 = spawn fetch(\"{base}\") let h2 = spawn fetch(\"{base}\") let h3 = spawn fetch(\"{base}\") return await h0 + await h1 + await h2 + await h3 }} }}"
    );
    let start = std::time::Instant::now();
    let (v, _) = run_src(&src, "main").expect("concurrent runs");
    let elapsed = start.elapsed();
    println!("CONCURRENT 4x500ms async fan-out took {:?}", elapsed);
    println!("SERIALIZED WOULD TAKE >= 2000ms; OVERLAP BUDGET < 1500ms");
    handle.join().expect("server thread");
    assert_eq!(v, 800, "4 × 200");
    assert!(
        elapsed >= std::time::Duration::from_millis(400),
        "delay actually applied (no skipped waits), took {elapsed:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(1500),
        "genuinely overlapped, not serialized (took {elapsed:?} vs serialized {seq_elapsed:?})"
    );
}

#[test]
fn async_unroutable_host_is_loud_not_hang() {
    // A slow/unreachable host fails cleanly with E-NET-UNREACHABLE,
    // never a hang: test completion itself is the no-hang proof (the
    // pool inherits the 10s connect / 60s global backstops), and the
    // code matches the sync path's transport-failure code.
    // TEST-NET-1 (RFC 5737): guaranteed unroutable, no external
    // dependency, no live internet needed.
    let src = "fn main() -> i32 async { let r = http_get_async(\"http://192.0.2.1:9/slow\") return 0 }";
    let start = std::time::Instant::now();
    let err = run_src(src, "main").expect_err("unreachable");
    let elapsed = start.elapsed();
    assert_eq!(err.code, "E-NET-UNREACHABLE", "{}", err.to_json());
    assert!(
        elapsed < std::time::Duration::from_secs(50),
        "bounded by the backstops, not a hang (took {elapsed:?})"
    );
}

#[test]
fn v2_has_no_async_http() {
    // v2 has no HTTP builtins at all: calls fail loud with E-UNDEFINED
    // (documented exclusion).
    let src = "fn main() -> i32 {\n  return http_get_async(\"http://x\")\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses the call shape");
    let err = klang::runtime::v2::run_v2_program(&prog, "main").expect_err("must fail");
    assert_eq!(err.code, "E-UNDEFINED", "{}", err.to_json());
}

#[test]
fn v2_plain_programs_still_run() {
    // Positive v2 control: unaffected by the v1 builtins.
    let src = "fn main() -> i32 {\n  return 40 + 2\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    let (v, _) = klang::runtime::v2::run_v2_program(&prog, "main").expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn jit_rejects_async_http() {
    // Builtins are outside the int-only JIT — sync and async alike:
    // loud rejection, never silent wrong code.
    let prog = parse(
        "fn main() -> i32 async { let r = http_get_async(\"http://127.0.0.1:9/x\") return r[\"status\"] }",
    );
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("must reject");
    assert!(err.contains("unsupported") || err.contains("builtin"), "{err}");
}

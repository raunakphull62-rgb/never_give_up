//! A2 threads, channels, signals (Wave 2 Stage A2).
//!
//! Flat builtins (matching `read_file` / `tcp_listen` convention):
//! `thread_spawn`, `thread_join`, `thread_id`, `chan`, `send`, `recv`,
//! `recv_timeout`, `try_recv`, `close`, `signal_watch`, `signal_pending`.
//!
//! Share-nothing OS threads: each `thread_spawn(fn_name, args_array)`
//! runs a NAMED top-level function on a new OS thread with its own
//! interpreter state; arguments and results are deep-copied (`Value::clone`;
//! plain data copies, handles share the underlying OS resource via `Arc`).
//! Closures as thread bodies are `E-THREAD-CLOSURE` (check-time when the
//! static type is a closure, runtime when a dynamic value holds one).
//!
//! Channels are bounded `VecDeque` queues behind `Mutex` + `Condvar`
//! (no external crates): `send` blocks when full, `recv` blocks when
//! empty, `close` wakes all waiters. `recv` on a closed empty channel
//! returns null (never hangs); `recv_timeout` returns null on timeout or
//! closed-empty; `try_recv` returns null when empty. `send` on a closed
//! channel is `E-CHAN-CLOSED`. `close` is idempotent.
//!
//! A1 handles (`Listener`/`Conn`/`UdpSock`) are `Send` (`Arc<Mutex<...>>`
//! over OS sockets) and may be passed to threads by cloning the `Arc`
//! (the queue's "transferring ownership"): after the move both sides hold
//! the same socket; concurrent operations serialize on the internal mutex,
//! `conn_close` from any thread closes for all, use-after-close is
//! `E-NET-CLOSED` (never a crash). Documented rule: move a handle to at
//! most one worker and do not use it from the parent afterwards; sharing
//! is safe but interleaves. Channels (`Chan`) are freely shared.
//!
//! Signals: `signal_watch("INT"/"TERM"/"HUP")` installs a minimal
//! async-signal-safe handler that sets an atomic flag;
//! `signal_pending(name)` polls and clears it (`1` when a signal arrived,
//! else `0`). Windows supports only `"INT"` (`TERM`/`HUP` are
//! `E-SIGNAL-UNSUPPORTED` there). Names are case-insensitive.
//!
//! Limits: max live threads default 256, override via `KLANG_MAX_THREADS`;
//! sandbox caps at 4 (`E-SANDBOX-THREADS` beyond that). `thread_id`
//! stays allowed in sandbox (pure, like `is_null`); all other A2 ops deny
//! in sandbox (`E-SANDBOX-THREADS` for threads/channels,
//! `E-SANDBOX-SIGNAL` for signals).

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Condvar, Mutex,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::diagnostics::Diagnostic;
use crate::runtime::Value;

// ---------------------------------------------------------------------------
// Limits.
// ---------------------------------------------------------------------------

/// Default max live Klang threads (also the existing task-group bound).
pub const MAX_THREADS_DEFAULT: usize = 256;
/// Sandbox cap for live Klang threads.
pub const SANDBOX_MAX_THREADS: usize = 4;
/// Max `chan` capacity (queue length, not preallocated).
pub const MAX_CHAN_CAPACITY: i64 = 1_000_000;
/// Max `recv_timeout` / `thread` timeout window (24h, like A1).
const MAX_TIMEOUT_MS: i64 = 24 * 3600 * 1000;

// ---------------------------------------------------------------------------
// Error constructors (every message names the offending value).
// ---------------------------------------------------------------------------

fn a2_err(code: &str, message: String, cause: String, rule: &str) -> Diagnostic {
    Diagnostic::error(code, &message, "runtime", 0, 0, &cause, &["check the thread, channel or signal name, then retry"], rule)
}

fn thread_closure(op: &str, detail: &str) -> Diagnostic {
    a2_err(
        "E-THREAD-CLOSURE",
        format!("{op} failed: closure cannot run as a thread: {detail}"),
        "thread bodies must name a top-level function".to_string(),
        "thread/closure",
    )
}

fn thread_unknown(op: &str, name: &str) -> Diagnostic {
    a2_err(
        "E-THREAD-UNKNOWN",
        format!("{op}(\"{name}\") failed: unknown function `{name}`"),
        format!("no top-level function named `{name}`"),
        "thread/unknown",
    )
}

fn thread_arity(op: &str, name: &str, want: usize, got: usize) -> Diagnostic {
    a2_err(
        "E-THREAD-ARITY",
        format!("{op}(\"{name}\") failed: `{name}` takes {want} args, got {got}"),
        "thread argument count must match the function".to_string(),
        "thread/arity",
    )
}

fn thread_limit(op: &str, max: usize) -> Diagnostic {
    a2_err(
        "E-THREAD-LIMIT",
        format!("{op} failed: too many live threads (limit {max})"),
        format!("at most {max} live Klang threads"),
        "thread/limit",
    )
}

pub fn thread_failed(thread_id: u64, thread_name: &str, cause: &Diagnostic) -> Diagnostic {
    let mut d = a2_err(
        "E-THREAD-FAILED",
        format!(
            "thread_join(thread {thread_id} \"{thread_name}\") failed: {} {}",
            cause.code, cause.message
        ),
        format!("thread `{thread_name}` failed: {}", cause.message),
        "thread/failed",
    );
    d.related.push(cause.clone());
    d
}

pub fn thread_panicked(thread_id: u64, thread_name: &str) -> Diagnostic {
    a2_err(
        "E-THREAD-FAILED",
        format!("thread_join(thread {thread_id} \"{thread_name}\") failed: thread panicked"),
        format!("thread `{thread_name}` panicked"),
        "thread/failed",
    )
}

fn chan_closed(op: &str) -> Diagnostic {
    a2_err(
        "E-CHAN-CLOSED",
        format!("{op} failed: channel is closed"),
        "channel was closed".to_string(),
        "chan/closed",
    )
}

fn chan_capacity(op: &str, got: i64) -> Diagnostic {
    a2_err(
        "E-CHAN-CAPACITY",
        format!("{op}({got}) failed: invalid capacity {got} (want 1..{MAX_CHAN_CAPACITY})"),
        format!("capacity {got} out of range"),
        "chan/capacity",
    )
}

fn chan_timeout_invalid(op: &str, got: i64) -> Diagnostic {
    a2_err(
        "E-CHAN-TIMEOUT",
        format!("{op}({got}) failed: invalid timeout {got}ms (want 0..{MAX_TIMEOUT_MS})"),
        format!("timeout {got}ms out of range"),
        "chan/timeout",
    )
}

fn signal_unknown(op: &str, name: &str) -> Diagnostic {
    a2_err(
        "E-SIGNAL-UNKNOWN",
        format!("{op}(\"{name}\") failed: unknown signal `{name}` (want INT, TERM or HUP)"),
        format!("unknown signal `{name}`"),
        "signal/unknown",
    )
}

#[allow(dead_code)]
fn signal_unsupported(op: &str, name: &str) -> Diagnostic {
    a2_err(
        "E-SIGNAL-UNSUPPORTED",
        format!("{op}(\"{name}\") failed: signal `{name}` unsupported on this platform"),
        format!("signal `{name}` unsupported here"),
        "signal/unsupported",
    )
}

// ---------------------------------------------------------------------------
// Thread IDs and live-count.
// ---------------------------------------------------------------------------

/// Next Klang thread id (0 is the main thread; workers start at 1).
static NEXT_THREAD_ID: AtomicU64 = AtomicU64::new(1);
/// Live Klang threads (incremented on spawn, decremented on thread exit).
static ACTIVE_THREADS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static THREAD_ID: Cell<u64> = const { Cell::new(0) };
}

/// Id of the calling OS thread's Klang thread (0 on main).
pub fn current_thread_id() -> u64 {
    THREAD_ID.with(|c| c.get())
}

/// Set the calling thread's Klang id (main sets 0 at entry, workers set
/// their assigned id at thread start).
pub fn set_current_thread_id(id: u64) {
    THREAD_ID.with(|c| c.set(id));
}

/// Configured max live threads: `KLANG_MAX_THREADS` when it parses to a
/// positive int, else [`MAX_THREADS_DEFAULT`]. Sandbox callers clamp to
/// [`SANDBOX_MAX_THREADS`] (see `spawn_allowed`).
pub fn max_threads() -> usize {
    if let Ok(raw) = std::env::var("KLANG_MAX_THREADS") {
        if let Ok(n) = raw.trim().parse::<i64>() {
            if n > 0 && (n as u64) <= (usize::MAX as u64) {
                return n as usize;
            }
        }
    }
    MAX_THREADS_DEFAULT
}

/// Check the spawn cap before creating a thread. Sandbox beyond 4 is
/// `E-SANDBOX-THREADS`; plain mode beyond the configured max is
/// `E-THREAD-LIMIT`.
pub fn spawn_allowed(op: &str) -> Result<(), Diagnostic> {
    if crate::sandbox::is_sandbox() {
        if ACTIVE_THREADS.load(Ordering::SeqCst) >= SANDBOX_MAX_THREADS {
            return Err(crate::sandbox::deny_threads(op));
        }
        return Ok(());
    }
    let max = max_threads();
    if ACTIVE_THREADS.load(Ordering::SeqCst) >= max {
        return Err(thread_limit(op, max));
    }
    Ok(())
}

/// Allocate a fresh worker id.
pub fn alloc_thread_id() -> u64 {
    NEXT_THREAD_ID.fetch_add(1, Ordering::SeqCst)
}

/// Live-thread accounting (increment right before `spawn`, decrement when
/// the thread body finishes; decrement also on spawn failure).
pub fn inc_active() {
    ACTIVE_THREADS.fetch_add(1, Ordering::SeqCst);
}

pub fn dec_active() {
    ACTIVE_THREADS.fetch_sub(1, Ordering::SeqCst);
}

pub fn active_count() -> usize {
    ACTIVE_THREADS.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Thread handles.
// ---------------------------------------------------------------------------

/// One detached Klang thread: its `JoinHandle` plus a cached result so a
/// second `thread_join` returns the same value instead of hanging or
/// failing (idempotent, deadlock-free).
pub struct ThreadState {
    pub id: u64,
    pub name: String,
    pub handle: Option<JoinHandle<Result<Value, Diagnostic>>>,
    pub result: Option<Result<Value, Diagnostic>>,
}

impl std::fmt::Debug for ThreadState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadState")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("live", &self.handle.is_some())
            .field("done", &self.result.is_some())
            .finish()
    }
}

pub type ThreadHandle = Arc<Mutex<ThreadState>>;

fn as_thread(op: &str, v: &Value) -> Result<ThreadHandle, Diagnostic> {
    match v {
        Value::Thread(h) => Ok(h.clone()),
        other => Err(crate::runtime::type_err(op, "a Thread", other)),
    }
}

/// Build a new thread handle around a just-spawned `JoinHandle`.
pub fn new_thread_handle(
    id: u64,
    name: &str,
    handle: JoinHandle<Result<Value, Diagnostic>>,
) -> Value {
    Value::Thread(Arc::new(Mutex::new(ThreadState {
        id,
        name: name.to_string(),
        handle: Some(handle),
        result: None,
    })))
}

/// `thread_join(t)`: block until the thread finishes, then return its
/// value (deep-cloned). A thread failure re-raises as `E-THREAD-FAILED`
/// naming the thread and cause (with the cause under `related`); a Rust
/// panic becomes `E-THREAD-FAILED` too. Idempotent: a second join returns
/// the same value. Exit requests (`exit(n)`) propagate unwrapped (like
/// task groups, exit is uncatchable).
pub fn thread_join(v: &Value) -> Result<Value, Diagnostic> {
    const OP: &str = "thread_join";
    // Joining itself is allowed in sandbox (the spawn was the gated op);
    // the value was already created, so observing it is harmless. Only
    // creation (`thread_spawn`, `chan`, signals) denies.
    let h = as_thread(OP, v)?;
    // Hold the per-thread lock across the blocking join so concurrent
    // joiners serialize and the second sees the cached result (no hang,
    // no double-take of the JoinHandle).
    let mut st = h.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = st.result.clone() {
        return match cached {
            Ok(val) => Ok(val),
            Err(d) if d.is_exit() => Err(d),
            Err(d) => Err(d),
        };
    }
    let id = st.id;
    let name = st.name.clone();
    let handle = st.handle.take();
    // Release-and-rejoin dance: `JoinHandle::join` consumes the handle,
    // so take it out, drop the guard while blocking, then re-lock to
    // cache. A concurrent joiner blocking on the mutex above waits; when
    // we release to join, it acquires, sees `handle: None` and no
    // `result` yet, and must wait for us to finish. To keep it simple and
    // deadlock-free we instead hold the guard across the join (the mutex
    // is per-thread, never global, so no other channel/thread blocks).
    // Re-acquire style: put the handle back and join while holding.
    if let Some(jh) = handle {
        // Put it back temporarily so the guard-holding join below works;
        // (we already took it, so just join outside the lock and cache).
        drop(st);
        let outcome: Result<Value, Diagnostic> = match jh.join() {
            Ok(Ok(val)) => Ok(val),
            Ok(Err(d)) => Err(d),
            Err(_) => Err(thread_panicked(id, &name)),
        };
        let mut st2 = h.lock().unwrap_or_else(|e| e.into_inner());
        // Exit propagates unwrapped (uncatchable, like task groups).
        match &outcome {
            Err(d) if d.is_exit() => {
                st2.result = Some(outcome.clone());
                return Err(outcome.unwrap_err());
            }
            Err(d) if d.is_cancelled() => {
                st2.result = Some(outcome.clone());
                return Err(outcome.unwrap_err());
            }
            Err(d) => {
                let wrapped = thread_failed(id, &name, d);
                st2.result = Some(Err(wrapped.clone()));
                return Err(wrapped);
            }
            Ok(val) => {
                st2.result = Some(Ok(val.clone()));
                return Ok(val.clone());
            }
        }
    }
    // No handle and no cached result (should not happen: every handle is
    // created with `Some`): treat as a loud join error, never a panic.
    Err(a2_err(
        "E-THREAD-FAILED",
        format!("{OP}(thread {id} \"{name}\") failed: thread was already consumed"),
        format!("thread `{name}` has no joinable handle"),
        "thread/failed",
    ))
}

/// Validate `thread_spawn` string/array operands shared by the runtime
/// spawn site (which owns the module/output). Returns the owned function
/// name and the deep-copied args on success.
pub fn check_spawn_operands(
    op: &str,
    fn_name_val: &Value,
    args_val: &Value,
) -> Result<(String, Vec<Value>), Diagnostic> {
    // Closures as bodies are E-THREAD-CLOSURE even through dynamic
    // (Unknown-typed) values; any other non-string is E-TYPE.
    if let Value::Closure { .. } = fn_name_val {
        return Err(thread_closure(op, "got a closure value (spawn takes a named function)"));
    }
    let name = match fn_name_val {
        Value::Str(s) => s.clone(),
        other => return Err(crate::runtime::type_err(op, "a function name string", other)),
    };
    if name.is_empty() {
        return Err(thread_unknown(op, &name));
    }
    // Disallow NUL / path-like names loudly (never a silent lookup miss).
    if name.contains('\0') {
        return Err(thread_unknown(op, &name));
    }
    let args_vec = match args_val {
        Value::Array(items) => items.clone(),
        other => return Err(crate::runtime::type_err(op, "an array of args", other)),
    };
    Ok((name, args_vec))
}

/// Check the named target exists and the arg count matches (runtime side
/// of the spawn; HIR owns the static closure check).
pub fn check_spawn_target(
    op: &str,
    name: &str,
    args_len: usize,
    param_len: Option<usize>,
) -> Result<(), Diagnostic> {
    let Some(want) = param_len else {
        return Err(thread_unknown(op, name));
    };
    if args_len != want {
        return Err(thread_arity(op, name, want, args_len));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Channels.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ChanInner {
    queue: VecDeque<Value>,
    capacity: usize,
    closed: bool,
}

#[derive(Debug)]
pub struct ChanState {
    inner: Mutex<ChanInner>,
    cvar: Condvar,
}

pub type ChanHandle = Arc<ChanState>;

fn as_chan(op: &str, v: &Value) -> Result<ChanHandle, Diagnostic> {
    match v {
        Value::Chan(h) => Ok(h.clone()),
        other => Err(crate::runtime::type_err(op, "a Chan", other)),
    }
}

fn check_timeout(op: &str, ms: i64) -> Result<u64, Diagnostic> {
    if ms < 0 || ms > MAX_TIMEOUT_MS {
        return Err(chan_timeout_invalid(op, ms));
    }
    Ok(ms as u64)
}

/// `chan(capacity) -> Chan` (capacity `1..MAX_CHAN_CAPACITY`; 0/rendezvous
/// is rejected as `E-CHAN-CAPACITY` — use 1 for minimal buffering).
pub fn chan(capacity: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "chan";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_threads(OP));
    }
    if capacity < 1 || capacity > MAX_CHAN_CAPACITY {
        return Err(chan_capacity(OP, capacity));
    }
    Ok(Value::Chan(Arc::new(ChanState {
        inner: Mutex::new(ChanInner {
            queue: VecDeque::new(),
            capacity: capacity as usize,
            closed: false,
        }),
        cvar: Condvar::new(),
    })))
}

/// `send(ch, v) -> 1`: block while full; `E-CHAN-CLOSED` when closed.
pub fn send(ch: &Value, v: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "send";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_threads(OP));
    }
    let h = as_chan(OP, ch)?;
    let mut guard = h.inner.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if guard.closed {
            return Err(chan_closed(OP));
        }
        if guard.queue.len() < guard.capacity {
            guard.queue.push_back(v.clone());
            h.cvar.notify_all();
            return Ok(());
        }
        guard = h.cvar.wait(guard).unwrap_or_else(|e| e.into_inner());
    }
}

/// `recv(ch) -> value`: block while empty; null on closed-empty (never hangs).
pub fn recv(ch: &Value) -> Result<Value, Diagnostic> {
    const OP: &str = "recv";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_threads(OP));
    }
    let h = as_chan(OP, ch)?;
    let mut guard = h.inner.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if let Some(v) = guard.queue.pop_front() {
            h.cvar.notify_all();
            return Ok(v);
        }
        if guard.closed {
            return Ok(Value::Null);
        }
        guard = h.cvar.wait(guard).unwrap_or_else(|e| e.into_inner());
    }
}

/// `recv_timeout(ch, ms) -> value or null`: null on timeout or
/// closed-empty (never hangs past the deadline).
pub fn recv_timeout(ch: &Value, ms: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "recv_timeout";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_threads(OP));
    }
    let wait = check_timeout(OP, ms)?;
    let h = as_chan(OP, ch)?;
    let mut guard = h.inner.lock().unwrap_or_else(|e| e.into_inner());
    // Fast paths: queued value wins without waiting; closed-empty and
    // zero-timeout never block.
    if let Some(v) = guard.queue.pop_front() {
        h.cvar.notify_all();
        return Ok(v);
    }
    if guard.closed {
        return Ok(Value::Null);
    }
    if wait == 0 {
        return Ok(Value::Null);
    }
    let deadline = Instant::now() + Duration::from_millis(wait);
    loop {
        let now = Instant::now();
        if now >= deadline {
            // Re-check once: a value queued just before the deadline wins.
            if let Some(v) = guard.queue.pop_front() {
                h.cvar.notify_all();
                return Ok(v);
            }
            if guard.closed {
                return Ok(Value::Null);
            }
            return Ok(Value::Null);
        }
        let rest = deadline - now;
        let (g, timeout) = h.cvar.wait_timeout(guard, rest).unwrap_or_else(|e| e.into_inner());
        guard = g;
        if let Some(v) = guard.queue.pop_front() {
            h.cvar.notify_all();
            return Ok(v);
        }
        if guard.closed {
            return Ok(Value::Null);
        }
        if timeout.timed_out() {
            return Ok(Value::Null);
        }
    }
}

/// `try_recv(ch) -> value or null`: never blocks.
pub fn try_recv(ch: &Value) -> Result<Value, Diagnostic> {
    const OP: &str = "try_recv";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_threads(OP));
    }
    let h = as_chan(OP, ch)?;
    let mut guard = h.inner.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(v) = guard.queue.pop_front() {
        h.cvar.notify_all();
        return Ok(v);
    }
    Ok(Value::Null)
}

/// `close(ch) -> 1`: idempotent; wakes all waiters (blocked `send`s fail
/// with `E-CHAN-CLOSED`, blocked `recv`s drain then see null).
pub fn close(ch: &Value) -> Result<(), Diagnostic> {
    const OP: &str = "close";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_threads(OP));
    }
    let h = as_chan(OP, ch)?;
    let mut guard = h.inner.lock().unwrap_or_else(|e| e.into_inner());
    guard.closed = true;
    h.cvar.notify_all();
    Ok(())
}

// ---------------------------------------------------------------------------
// Signals (minimal polling flags, no handlers allocating).
// ---------------------------------------------------------------------------

static FLAG_INT: AtomicBool = AtomicBool::new(false);
static FLAG_TERM: AtomicBool = AtomicBool::new(false);
static FLAG_HUP: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_int(_: i32) {
    FLAG_INT.store(true, Ordering::Relaxed);
}

extern "C" fn handle_term(_: i32) {
    FLAG_TERM.store(true, Ordering::Relaxed);
}

extern "C" fn handle_hup(_: i32) {
    FLAG_HUP.store(true, Ordering::Relaxed);
}

#[cfg(unix)]
extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> isize;
}

#[cfg(windows)]
extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> isize;
}

#[cfg(unix)]
const SIG_INT: i32 = 2;
#[cfg(unix)]
const SIG_TERM: i32 = 15;
#[cfg(unix)]
const SIG_HUP: i32 = 1;

#[cfg(windows)]
const SIG_INT: i32 = 2;
#[cfg(windows)]
const SIG_TERM: i32 = 15;
#[cfg(windows)]
const SIG_HUP: i32 = 1;

/// Canonicalize a signal name (`INT`/`TERM`/`HUP`, case-insensitive).
fn canon_signal(name: &str) -> Result<&'static str, ()> {
    if name.eq_ignore_ascii_case("int") {
        Ok("INT")
    } else if name.eq_ignore_ascii_case("term") {
        Ok("TERM")
    } else if name.eq_ignore_ascii_case("hup") {
        Ok("HUP")
    } else {
        Err(())
    }
}

/// `signal_watch(name)`: install the flag handler for INT/TERM/HUP.
/// Windows supports only INT.
pub fn signal_watch(name: &str) -> Result<(), Diagnostic> {
    const OP: &str = "signal_watch";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_signal(OP));
    }
    let canon = canon_signal(name).map_err(|_| signal_unknown(OP, name))?;
    #[cfg(windows)]
    {
        if canon == "TERM" || canon == "HUP" {
            return Err(signal_unsupported(OP, canon));
        }
    }
    unsafe {
        match canon {
            "INT" => {
                FLAG_INT.store(false, Ordering::SeqCst);
                signal(SIG_INT, handle_int);
            }
            "TERM" => {
                FLAG_TERM.store(false, Ordering::SeqCst);
                signal(SIG_TERM, handle_term);
            }
            "HUP" => {
                FLAG_HUP.store(false, Ordering::SeqCst);
                signal(SIG_HUP, handle_hup);
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

/// `signal_pending(name) -> Bool`: `1` once when the signal arrived since
/// the last poll (or watch), else `0`; polling clears the flag.
pub fn signal_pending(name: &str) -> Result<bool, Diagnostic> {
    const OP: &str = "signal_pending";
    if crate::sandbox::is_sandbox() {
        return Err(crate::sandbox::deny_signal(OP));
    }
    let canon = canon_signal(name).map_err(|_| signal_unknown(OP, name))?;
    #[cfg(windows)]
    {
        if canon == "TERM" || canon == "HUP" {
            return Err(signal_unsupported(OP, canon));
        }
    }
    let hit = match canon {
        "INT" => FLAG_INT.swap(false, Ordering::SeqCst),
        "TERM" => FLAG_TERM.swap(false, Ordering::SeqCst),
        "HUP" => FLAG_HUP.swap(false, Ordering::SeqCst),
        _ => unreachable!(),
    };
    Ok(hit)
}

/// Test-only: raise the INT flag without a real OS signal (lets gates pin
/// the poll-and-clear shape without forking).
pub fn test_raise(name: &str) {
    match canon_signal(name) {
        Ok("INT") => FLAG_INT.store(true, Ordering::SeqCst),
        Ok("TERM") => FLAG_TERM.store(true, Ordering::SeqCst),
        Ok("HUP") => FLAG_HUP.store(true, Ordering::SeqCst),
        _ => {}
    }
}

/// Test-only: clear all flags (keeps signal gates hermetic in one process).
pub fn test_clear() {
    FLAG_INT.store(false, Ordering::SeqCst);
    FLAG_TERM.store(false, Ordering::SeqCst);
    FLAG_HUP.store(false, Ordering::SeqCst);
}

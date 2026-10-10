# D7. IO and concurrency: sockets, async, threads, signals, FFI

Status: design sketch — no implementation. (Wave1 S6; out of scope for S6.)

S6 ships only blocking process spawn (`spawn`/`run`), wall-clock time
(`now_ms`/`now_iso`/`sleep_ms`), and env reads (`get_env`). The items
below are explicitly NOT added here. Each needs its own design + gates.

## 1. Sockets (TCP/UDP, DNS, TLS)

What it would require: new builtins (`tcp_connect`, `tcp_listen`?) or a
`net` package over them; address parsing + DNS (`E-NET-*` codes like the
`http_*` twins); read/write deadlines (no unbounded block); a
capability story (which hosts/ports a program may dial — root-manifest
allow-list, like D5's `[capabilities]`); parity rules (JIT rejects loudly
until intrinsics exist; v2 has no sockets).

Options: (A) blocking sockets on the existing spawn-style thread pool
(simple, matches `http_get` today, but each conn burns a thread);
(B) true non-blocking IO with a poller (fast, but a new runtime +
waker ABI); (C) sockets only via spawned helper processes (zero new
syscalls, but fork+exec per conn).

Risks: SSRF (registry code dialing internal hosts), DNS rebinding,
slow-loris (unbounded reads), secret exfiltration via env + socket
pairing. Any socket API must inherit S6's no-shell/no-glob discipline
plus host/port allow-listing and byte caps.

## 2. Async (await, executors, cancellation)

What it would require: today `async` is an effect annotation over
blocking pool calls (`http_get_async` parks on a 16-worker pool) plus
structured `task_group`/`spawn`/`await` for CPU tasks. Real async needs:
a waker/executor (or keep the pool and call it honest), `cancel` wired
through IO back-edges (today loop-only), and `E-CANCELLED` propagation
rules for sockets/timers.

Options: (A) keep simulated async, extend the pool to sockets/timers
(smallest, honest if labeled); (B) embed a minimal executor for
socket/timer readiness (true async, but a new scheduler + fairness
policy); (C) syntax-only `async` with codegen to threads (simple, but
thread-per-task caps at ~256 like today).

Risks: cancellation-safety (dropped futures losing failures, against
the lossless `E-TASK-GROUP` rule), executor blocking (a `sleep_ms`
inside async starving the pool), and v2/JIT divergence (both reject
async pieces loudly today).

## 3. Threads (spawn, shared memory, atomics)

What it would require: today `spawn` inside `task_group` is
join-scoped (no detached tasks, every handle awaited). Raw threads need:
a spawn primitive that can outlive its group (or an explicit decision
to forbid it), shared-memory rules (Klang args are by-value; shared
maps would need a borrow story — Managed mode only today), and a
memory model (SeqCst? data-race `E-RUNTIME`?).

Options: (A) no raw threads: keep join-scoped `task_group` only, add
channels for communication (safest, matches current semantics);
(B) scoped threads with borrowed captures (expressive, but needs a
borrow checker — explicitly out of scope); (C) threads + message
passing only (no shared mutable state by construction).

Risks: data races, deadlocks (no lock ordering story), and breaking
the "no detached tasks, ever" invariant that makes failure lossless.
Any thread API must preserve join-on-failure-path.

## 4. Signals (SIGINT/SIGTERM/handlers, timeouts)

What it would require: today there are no timeouts (S6 `spawn`/`run`
block until exit; `sleep_ms` blocks until done). Signals need: a
handler table (Klang closures as handlers? — reentrancy rules),
masking during critical sections, and interaction with `task_group`
cancellation (signal → cancel token?).

Options: (A) no handlers: only `timeout_ms` wrappers around
spawn/sleep (bounded waits, `E-TIMEOUT` code, no async-signal code);
(B) cooperative signal polling at back-edges (like `cancel` today —
no true handlers, just a flag); (C) real handlers (most expressive,
but signal-unsafe code in handlers is a classic bug farm).

Risks: handler reentrancy (allocating/locking inside a handler),
lost wakeups, and platform divergence (Windows console events vs
POSIX signals — cfg-gated behavior must stay loud, never silent).

## 5. FFI (`extern`, dylib, syscalls)

What it would require: see D5 (recommended: declared `extern fn`
against a static host table + manifest capabilities, deny by default).
Any FFI needs: an ABI story (`i32`/`f64`/`str`/`bool` only at the
boundary), capability consent (root `klang.toml` only, deps inherit
nothing), and per-entry JIT intrinsics (else loud reject).

Options: per D5 — (A) static host table (recommended), (B) WASM plugins,
(C) no FFI (more builtins only).

Risks: the trust boundary widens (malicious registry packages calling
`kill`/`ptrace`/raw sockets); dynamic loading (`dlopen`) would make
the sandbox unauditable — D5 refuses it by construction.

## Recommended order

1. Timeouts first (`timeout_ms` around spawn/sleep, `E-TIMEOUT`):
   unblocks S6's "no timeout" caveat with no new concurrency.
2. Sockets via the existing bounded pool (simulated-async style, with
   host allow-list + byte caps), reusing the `http_*` error shape.
3. Channels + keep join-scoped tasks (no raw threads/shared memory).
4. D5 static-table FFI prototype (3 query-only entries), reusing the
   capability manifest sockets already need.
5. True async executor and signals last (each needs the cancellation +
   parity harness from 1–4 to be meaningful).

Each step lands with `tests/*_gates.rs` + parity allowlist entries
(JIT/v2 reject loudly until intrinsics land), never silent.

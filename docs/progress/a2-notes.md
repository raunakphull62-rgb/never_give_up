# A2 notes — proposed text for shared docs (do NOT apply until L1-docs)

## reference.md — Threads, channels, signals (new section)

- `thread_spawn(fn_name: str, args: array) -> Thread`: runs a named top-level
  `fn` on a new OS thread with its own state; args/results deep-copy (handles
  share). Closures are `E-THREAD-CLOSURE`. Unknown names `E-THREAD-UNKNOWN`,
  arity mismatch `E-THREAD-ARITY`, too many live threads `E-THREAD-LIMIT`
  (default 256, `KLANG_MAX_THREADS` overrides). Sandbox allows 4, beyond that
  `E-SANDBOX-THREADS`.
- `thread_join(t) -> value`: blocks, returns the value; failures re-raise as
  `E-THREAD-FAILED` (thread + cause named, cause under `related`); idempotent.
  Unjoined threads are reaped at exit.
- `thread_id() -> Int`: `0` on main, `1..` on workers (allowed in sandbox).
- `chan(cap) -> Chan` (`1..1000000`), `send(ch, v) -> 1` (blocks when full),
  `recv(ch)` (blocks when empty), `recv_timeout(ch, ms)` (null on timeout),
  `try_recv(ch)` (null when empty, never blocks), `close(ch) -> 1`
  (idempotent, wakes all; `recv` on closed-empty is null, `send` on closed is
  `E-CHAN-CLOSED`). Channels share freely across threads.
- A1 handles move to threads by `Arc` clone: one worker should own each
  socket afterwards; concurrent use serializes, `conn_close` anywhere closes
  everywhere, use-after-close is `E-NET-CLOSED`.
- `signal_watch("INT"/"TERM"/"HUP")`, `signal_pending(name) -> Bool` (polls
  and clears; polling loop + `http_stop`-style graceful shutdown on Render).
  Windows: `INT` only.

## limitations.md — A2 limits

- No shared mutable state (only channels/handles share); no `mutex_new`.
- `chan(0)` rendezvous unsupported (`E-CHAN-CAPACITY`); use `1`.
- Signals are polling flags, not handlers running Klang code.
- Threads are OS threads (8 MiB stack); forgetting `thread_join` reaps at
  exit but hides failures — always join.
- Sandbox: threads capped at 4, channels/signals denied.

## errors.md — new codes

`E-THREAD-CLOSURE`, `E-THREAD-UNKNOWN`, `E-THREAD-ARITY`, `E-THREAD-LIMIT`,
`E-THREAD-FAILED`, `E-CHAN-CLOSED`, `E-CHAN-CAPACITY`, `E-CHAN-TIMEOUT`,
`E-SIGNAL-UNKNOWN`, `E-SIGNAL-UNSUPPORTED`, `E-SANDBOX-THREADS`,
`E-SANDBOX-SIGNAL` (all `runtime`, `0,0`, never panic).

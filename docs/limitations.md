# Known limitations

Honest list, pulled from the audit's triaged-not-fixed findings.
Nothing here is buried: each item is either a deliberate scope cut
with a workaround or recorded debt with a stated direction.

## F11 — `&&` and `||` short-circuit (fixed in Phase 1)

The right-hand side runs only when the left side does not decide the
result, on the interpreter and the int-only JIT alike. Previously both
sides always evaluated, so a bounds guard in `&&` position
(`i < n && s[i] == "x"` with `i == n`) failed at run time with
`E-RUNTIME` instead of skipping the right-hand side; now the guard
skips cleanly. Result values are unchanged (`Int` 0/1).

```klang
// @run prints: 2 ; return: 0
fn main() -> i32 {
    let s = "ab"
    let i = 2
    if i < 2 {
        if s[i] == "x" {
            return 1
        }
    }
    print(i)
    return 0
}
```

## F7 — most diagnostics carry `0,0` spans (fixed in Wave 1 S2)

Spans are real: check-time diagnostics (parse, import, type,
strict, match, duplicate, D3 boundaries via MIR files, and
`klang check --json` with imports) carry the defining file, byte offsets, and
1-based line/column verified against the file text. Explicit
no-location (`line 0, column 0`) remains for: in-memory checks without a
real path, a missing command-line file, most runtime failures where the
position was lost in lowering, `E-CANCELLED`/`E-EXIT`, library-only
codes, v2 paths, and registry-fetch failures. See
`docs/progress/S2-notes.md` for the per-code list.

## F6 — `MAX_FUNCTION_SCOPE = 2` has no recorded rationale

The repair planner falls back to whole-file scope when diagnostics
span more than 2 functions. 2 vs 3/4 is unjustified in the code;
raising it only grows prompt size. Left as-is deliberately.

## F8 residual — no explicit AST-depth bound

Deep-stack execution (256 MiB worker) moved the ceiling for flat
expressions from ~10³ to ~10⁵ operands, but there is still no explicit
depth limit with a dedicated diagnostic — pathological input degrades
into resource use rather than a clean error. Malformed input in every
other tested shape is a diagnostic, never a crash.

## Call-depth cap is 1024 frames (interpreter); spawn threads and JIT differ

The v1 (`run`) and v2 (`run-v2`) interpreters cap nested calls at 1024
frames: deeper recursion fails with clean `E-RUNTIME` ("call depth
exceeded", cause "call stack depth limit reached", CLI exit 1), never a
silent wrong answer. Depth 1000 runs; depth 2000 fails loudly. The CLI
runs on a 256 MiB deep-stack worker so 1024 fits with margin (measured
release ~2.4 KB/frame; debug worst-case ~47 KB v1 / ~114 KB v2).

Spawned tasks run on dedicated 8 MiB threads (`SPAWN_STACK_BYTES`,
`src/runtime/mod.rs`, `src/runtime/v2.rs`), not the 256 MiB worker and
not the old Rust-default 2 MiB that aborted at ~850 frames release
(~40 debug) before the guard could fire. Measured release after the
fix: spawned `countdown(1000)` returns 1000, spawned `countdown(2000)`
fails with clean `E-RUNTIME`. Debug still overflows for 1000-deep
spawn recursion (~48 MB needed vs 8 MiB); keep spawn recursion shallow
in debug. Memory: 8 MiB is reserved virtual per task, committed on
demand — 32 concurrent shallow spawns measured ~5.5 MB RSS vs ~4.9 MB
for 1 spawn (+0.6 MB for 31 extra threads); 256-task cap bounds virtual
to 2 GiB.

## JIT parity (D6, Wave 1 S5)

`--backend-jit` is opt-in and int-only; programs outside its subset
still fail loudly with `JIT-FAIL` (never silently). Inside the subset
it now agrees with the interpreter exactly: division/modulo by zero
(`E-RUNTIME`), 64-bit overflow in `+ - * / %` and unary `-`
(`E-OVERFLOW`, never wraps), the 1024-frame call-depth guard
(`E-RUNTIME`), and the `i32`/`u32`/`u64`/`u8` argument/return/
assignment boundaries (`E-RUNTIME`, rule `types/range`). Out-of-subset
code — floats, strings, lists, maps, structs, enums, closures,
builtins (including all six S6 builtins plus the four S9 builtins
`print_raw`/`is_eof`/`chdir`/`sort_by`), `Spawn`/`Await`, `try`/`catch`,
and entry arity above 4 — is refused loudly at compile time.

## S9 output and file-guard notes

- `print(x)` ends with a newline; `print_raw(x)` writes the same
  rendering with no trailing newline and flushes (both buffered in
  order, so prompts and captured child output stay ordered — the
  `run()` inherit-vs-`print` tradeoff in FINDINGS B2 still applies to
  `run` only).
- `slice(x, a, b)` with an `unknown` static type returns `Unknown`
  (array at runtime stays array, strings stay strings), so
  `spawn(cmd, slice(toks, 1, len(toks)))` checks clean (FINDINGS B1).
- File access (F1): plain `klang run` (and `klang test`) allows ALL
  relative and absolute paths, subject only to OS permissions
  (`E-IO-NOT-FOUND`/`E-IO-PERMISSION`/`E-IO-FAILED`). Only
  `klang run --sandbox` limits files to the temp dir (absolute only
  under temp, relative with no `..`; denials are `E-SANDBOX-FILE`).

## `run` exit codes wrap at 8 bits (Linux)

`klang run` (and `run-v2`) print only program output; the entry return
value becomes the process exit code and is never printed. A `main`
with no integer return exits 0. A `main` with no declared return type
(omitted `->`, Phase 3b decision) always exits 0: its last expression
value is discarded for exit purposes, so `fn main() { print(7) }`
prints `7` and exits 0. A declared return type (`-> T`) keeps the
value mapping — including an explicit `-> void`, whose last expression
value still maps (`Int` to the exit code, else 0); only the omitted
form discards. Compile/runtime failures print the
structured JSON diagnostic on stderr and exit 1. Exit codes are OS
8-bit values and wrap without clamping: `return 256` exits 0,
`return -1` exits 255. Pinned by `exit_code_wraps_at_256_and_negatives`
(`tests/quiet_gates.rs`) and `void_main_exit_code_is_always_zero_when_omitted`
(`tests/return_ergonomics_gates.rs`).

## Design-scope cuts (not bugs)

- **Ownership**: `Managed` mode only; `Value`/`Owned`/`UnsafeFfi` are
  rejected at the library API (`E-OWNERSHIP-MODE`). There is no surface
  syntax for modes, and `klang check` never emits this code.
- **Contracts**: only the `non-empty` precondition is executable, via
  the library API (`E-CONTRACT`); nothing in surface syntax triggers it.
- **JIT**: integer-only behind `--backend-jit` (never the default;
  warns on stderr every run; see above); the interpreter is the reference
  backend. Out-of-subset programs fail loudly with `JIT-FAIL`, never
  silently. `build` always stops at the MIR listing.
- **v2 tree-walker (`run-v2`)**: an experimental subset vehicle for the
  separate v2 language track (schemas, `echo fn`/`listen`, `flow` with
  `dep=`, `tune`/`verify` — see `docs/roadmap.md`), explicitly selected
  and with its own syntax. It is NOT the reference backend (the v1
  interpreter is). It cannot run imports, loops (`while`/`for`),
  closures, float literals (float values exist at runtime but programs
  cannot spell them), `try`/`catch`, or the standard library (no `len`,
  no stdin/file/process builtins, and the 26 package self-tests do not
  parse). Whatever it rejects, it rejects loudly (`E-PARSE-V2` /
  `E-UNDEFINED`), never silently.

## Backend limits (Wave 2 A1/A2/A3/L1/A3b)

- Sockets are blocking (`conn_read` is one syscall — loop; `conn_read_line`
  frames lines); strings are UTF-8 (hex for binary); `null` has no literal.
- Threads are share-nothing except handles (give a handle to ONE worker);
  `chan(0)` rejected; join to observe failures; sandbox caps at 4.
- HTTP needs `Content-Length` (`chunked` is `400`); `json_decode` is native
  (`json_parse` is the package); handler errors are `500`.
- Datetime is UTC millis (years `1..9999`, fixed offsets only); `tar_create`
  does not compress; sandbox allows pure forms, temp-dir for files.
- Postgres params are bound `$1` (`Int`/`Float`/`Str`/`Null` only);
  NUMERIC needs `::text`/`::float8`; `BOOL` reads as `0`/`1`, writes via
  literals; one connection per request in handlers; sandbox denies all.
- **Packages**: manifest + FNV lockfile for local files is
  change-detection, not cryptography; registry pins add SHA-256
  (`package <name> <version> <sha256>` in `klang.lock`, verified on
  every download). No private packages, yanking, or version ranges.
- **LSP**: `klang lsp` serves diagnostics-as-you-type (whole-file
  re-check with debounce); hover and go-to-definition are planned v2.
  Closures (by-value capture), match guards, and nested match patterns
  ARE in the language (SPEC §2b/§2c); still absent: no trait bounds;
  no struct-style variants; no `throw`
  statement (`throws` is propagation-checked only); no const items;
  arrays and maps are dynamically typed (`unknown` by design).
- **Formatter**: `fmt` preserves comments (standalone comments keep
  their anchor, trailing comments move to end of line with one space),
  but comment placement is heuristic (segmented token alignment with a
  file-end fallback) — hand-placed comments can shift lines while
  staying attached to the same code. Treat formatted output as a
  canonical snapshot.
- **Salsa DB**: single-file inputs only.
- **Integer widths (D3, Wave 1 S4)**: `Int` is 64-bit checked
  (`E-OVERFLOW` past ±2^63, never wraps); `i32`/`u32`/`u64`/`u8`
  enforce their range at argument/return/assign boundaries (`E-RUNTIME`
  naming type and value); `W-TYPE-NARROW` is retired. Still
  unrepresentable: `u64` magnitudes past `i64::MAX` (effectively
  `0..i64::MAX`, since values are `i64` — cannot be spelled or
  produced); enum payloads are statically checked only (no runtime
  boundary there); `parse_int` bounds stay 32-bit.
- **Process/time/env (S6, F1)**: `spawn`/`run` are argv-only (no shell, no
  glob, no timeout, no stdin piping); `spawn` buffers all output in
  memory (no cap — hostile gigabytes OOM like `run_process`); `run`
  inherits stdio. `spawn` is both a task keyword and a builtin
  (`spawn (f())` is task-group spawn, `spawn(cmd, args)` is the
  builtin). `now_ms` exceeds `i32` (flows as Int, `E-OVERFLOW` on
  arithmetic); `now_iso` is wall-clock UTC. `get_env`/`env` absence is
  `""` (no null — unset is indistinguishable from empty). `set_env` with an
  empty name or `=`/NUL in the name (or NUL in the value) is `E-ENV-INVALID`
  naming the bad name, never a panic. All six are
  interpreter-only (JIT loud-rejects, v2 `E-UNDEFINED`). Missing
  binaries are `E-PROCESS-NOT-FOUND` (other OS refusals are
  `E-PROCESS-FAILED`); bad sleep durations are `E-TIME-INVALID`.
- **Benchmarks**: live-model convergence is measured inside harnesses
  (`bench/run_live.sh` checks the harness signal per task with zero
  model calls); only the offline mock baseline is measured in-tree.

## Backend hardening (H1 stress findings)

- Short body with half-close is `400` (was silent close; fixed).
- `notes_api` maps DB connection failures to `503` (`E-PG-CONNECT`),
  other DB errors to `500`; server never panics on exhaustion (4 workers
  stay under PG `max_connections` 100).
- Graceful shutdown drains queued jobs (100/100 with 32 workers);
  keep-alive workers wait out `read_timeout_ms`, so tests/clients use
  `Connection: close` for fast stop.
- 40k requests leak-flat (RSS +~2MB, fds 4, threads flat); 10k in 0.65s
  (~15k rps, p50 0.23ms, p99 51ms on 2 CPUs).

## Server throughput + shutdown correctness (H2 findings)

- **New-connection throughput**: the accept loop no longer polls (100ms
  window + 5ms sleeps ≈ 5ms latency per new conn, ~197 rps sequential
  on 2 CPUs). It now blocks in `listener_accept_blocking` (duplicated
  listener fd, no state-mutex held across the accept) and is woken on
  stop by a self-connect (`listener_wake`), because `close()` does not
  wake a blocked accept on Linux. Sequential new-conn rate after the
  fix: ~7,650-9,500 rps (0.11-0.13 ms/req); keep-alive unchanged
  (~12k rps). H2 gate floor: 1,500 rps.
- **Graceful shutdown drains the backlog**: on stop the acceptor is
  joined, the kernel listen backlog is drained non-blocking into the
  job queue, workers finish every queued + in-flight conn, and the
  listener closes last. Restored H1 gate (8 workers, 100ms handlers,
  100 in-flight): 100/100 (was 81/100 — backlog conns dropped when the
  listener closed first); the 32-worker/300ms variant stays green as an
  extra test. The price is stop latency: every queued handler runs to
  completion, so a stop with 4 workers, 300ms handlers and 300 conns
  exits after ~21s (measured); the queue cap (1000) bounds the worst
  case, and a deploy's SIGKILL grace bounds it anyway.
- **Idle keep-alive cannot starve workers**: after each response a
  worker probes the socket for ~1ms (`TcpStream::peek`, consumes
  nothing); a request already there is served immediately (fast path),
  otherwise the conn is parked in a per-server **idle pool** owned by a
  poller thread. The poller round-robins parked conns with an adaptive
  probe (one detection cycle ≤ ~50ms for ≤50 parked conns, growing
  ~1ms/conn above that) and hands readable conns back to the worker
  queue; EOF or the idle budget closes a conn quietly (no 408).
  `http_stop` stops the poller before closing the job queue, so no conn
  is lost. Gate: 4 workers + 20 idle keep-alive conns, fresh request
  answered well under 1s.
- **New serve opts** (all `E-HTTP-OPTIONS` when invalid, alongside the
  existing four): `idle_timeout_ms` (default `5000`, range `0..60000`,
  `0` = never idle-close) and `max_requests_per_conn` (default `1000`,
  range `0..1000000`, `0` = unlimited; the closing response carries
  `Connection: close`, and the H1 20k-request leak probe now reconnects
  at the cap like a real client).
- Not fixed: the poller's detection latency is per-cycle (does not
  scale to thousands of parked conns on a small box); a burst larger
  than the job-queue cap can drop SYNs to SYN-retry (~1s) as before.

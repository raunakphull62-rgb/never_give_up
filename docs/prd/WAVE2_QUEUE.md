# Klang Wave 2 — Stage Queue (single agent, run in order)

Goal: make Klang able to write and host a real web backend (on Render), plus datetime and compression.
Stages, in this order: A1 sockets -> A2 threads+signals -> A3 HTTP server+client -> L1 datetime+compression -> A3b PostgreSQL client -> R1 Render kit.
The human gives you ONE stage at a time with the message: "Read /content/drive/MyDrive/klang-notes/WAVE2_QUEUE.md fully and do STAGE <id> only."
Do only that stage. Background design for the whole language is in docs/prd/PRD_V1_COMPLETION.md (sections 1, 2.1-2.5); this file overrides it where they differ.

---------------------------------------------------------------------
## 0. Environment (read first)

- Project: Klang, a programming language; the compiler/interpreter is written in Rust. Repo working directory: /content/drive/MyDrive/never_give_up, git branch `wave2` (tracks GitHub main).
- That directory is on Google Drive, which cannot execute binaries. ALWAYS build and test with: `. $HOME/.cargo/env && CARGO_TARGET_DIR=/tmp/t1 cargo test --locked`. Run the compiled binary from /tmp/t1/debug/klang.
- Cargo runs can exceed your 10-minute shell limit: run long ones as `nohup cargo test --locked --no-fail-fast > /tmp/<stage>.log 2>&1 &` and poll the log. Delete finished build dirs if disk fills.
- NEVER push, tag or publish. Commit locally on `wave2` after each meaningful step (so work survives a restart). The human pushes.
- The MCP server was removed. Tools for AIs: `klang check --json <file>`, `klang run [--sandbox] <file>`, `klang fmt`. `klang run --sandbox` is the restricted mode (E-SANDBOX-FILE, E-SANDBOX-SPAWN, E-SANDBOX-NETWORK denials); plain `klang run` allows everything.
- Docs for the language: docs/reference.md, docs/limitations.md, docs/LANGUAGE_STATUS.md, docs/FINDINGS.md, and the verified AI guide in docs/ai/ (use docs/ai/core.md for syntax). Existing builtins already include http_get/http_post (check their exact current signatures first), spawn/run, get_env, now_ms, read_file, etc.

## 1. Rules for every stage

1. First run the full test suite and record the baseline count. NEVER let it go down. If tests named osio_* or quiet_* fail in a full run, rerun once (shared temp-file races) before treating them as real.
2. Put new code in NEW files wherever possible (for example src/stdlib/a1_net.rs); append registration lines at the end of existing lists in shared files (src/hir.rs, src/runtime/mod.rs, src/stdlib/mod.rs, src/main.rs). Keep shared-file edits small.
3. Tests go in tests/<stage>_gates.rs (a1_gates.rs, a2_gates.rs, a3_gates.rs, l1_gates.rs, a3b_gates.rs, r1_gates.rs), at least 15 tests per stage: happy paths, every error code, hostile input, and proof that nothing panics on bad input.
4. New builtins must be rejected loudly by the JIT and v2 backends (like spawn/run today). Add the allowlist entry with a one-line reason; the backend parity gate must stay green.
5. Errors: stable codes E-<AREA>-<REASON>, message names the offending value, real source span, never a panic. Handles (listener, connection, thread, channel, db, server) are opaque values with a clear type name in errors and are closed at program exit.
6. In `klang run --sandbox`, network, threads, and anything powerful new in this stage are DENIED with E-SANDBOX-* codes (add tests for plain mode and sandbox mode).
7. Tests may use ONLY loopback (127.0.0.1), with port 0 so the OS picks a free port (never fixed ports). Anything needing the real internet is skipped unless env KLANG_TEST_NET=1. Tests are deterministic and leave no processes, sockets or temp files behind. Use unique temp directories per test (never fixed /tmp names).
8. New .klang files must pass `klang fmt --check`.
9. Docs: write docs/progress/<stage>.md (what changed, builtin signatures, error codes, limits) and docs/progress/<stage>-notes.md (proposed text for shared docs). Do NOT edit shared docs (reference.md, limitations.md, SPEC.md, README.md) in a feature stage, except where the stage says so (R1). Also add a verified section for the new features to docs/ai/ ONLY in the stage called L1-docs at the end of the queue (the human will ask).
10. Each commit must build from a clean checkout (`git worktree add /tmp/clean-<stage> <commit>` and build there at least for the final commit; remove it after).
11. Ask nothing mid-stage unless a decision cannot be undone. State your reading of any ambiguity at the top of docs/progress/<stage>.md and continue.
12. Final report: commits (hash + subject), baseline and final test counts (0 failed), what was built, what could not be built and why, design deviations, known limits.

---------------------------------------------------------------------
## STAGE A1 — Sockets (TCP, UDP, DNS)

Builtins (match the existing naming style; document exact signatures):
- `tcp_listen(host, port) -> Listener` (port 0 = any), `listener_port(l) -> Int`, `tcp_accept(l) -> Conn` (blocking), `tcp_accept_timeout(l, ms) -> Conn or null`.
- `tcp_connect(host, port, timeout_ms) -> Conn`.
- `conn_read(c, max_bytes) -> Str` (empty string at peer close), `conn_read_line(c) -> Str` (without the newline; null at close), `conn_write(c, s) -> Int` (bytes written), `conn_close(c)`, `conn_set_timeout(c, ms)`, `conn_peer(c) -> Str` ("ip:port").
- Binary safety: check whether Klang has a bytes type. If not, document that strings are UTF-8 and add `conn_read_hex(c, max_bytes)`, `conn_write_hex(c, hex)` plus hex_encode/hex_decode helpers if missing (B1 may have added some; reuse).
- UDP: `udp_bind(host, port) -> Sock`, `udp_send_to(sock, host, port, s)`, `udp_recv_from(sock, max_bytes, timeout_ms) -> {data, host, port}`.
- DNS: `dns_lookup(host) -> array of ip strings`.
- Errors: E-NET-REFUSED, E-NET-TIMEOUT, E-NET-ADDR-INVALID, E-NET-ADDR-IN-USE, E-NET-CLOSED, E-NET-IO, E-NET-DNS.
- Sandbox: all of the above denied with E-SANDBOX-NETWORK.
- Tests (loopback only): echo server in a Rust test thread or a second Klang process, large payloads (1 MB), timeouts, closed peer, port reuse, UDP round trip, DNS lookup of "localhost".
- Do not implement threads here; blocking calls only.

## STAGE A2 — Threads, channels, signals

Read docs/design/D7-io-and-concurrency.md and keep existing task_group/spawn/await tasks working. Add share-nothing OS threads:
- `thread_spawn(fn_name, args_array) -> Thread` runs a NAMED top-level function on a new OS thread with its own interpreter state; arguments and results are deep-copied. Closures as thread bodies are a compile-time error E-THREAD-CLOSURE.
- `thread_join(t) -> value` (re-raises thread failure as E-THREAD-FAILED naming the thread and cause), `thread_id() -> Int`.
- Channels: `chan(capacity) -> Chan`, `send(ch, v)`, `recv(ch)` (blocking), `recv_timeout(ch, ms)` (null on timeout), `try_recv(ch)`, `close(ch)`. Values sent are deep-copied. Channel handles may be passed to threads (and listeners/connections from A1 may be passed to threads by transferring ownership: document the rule, make misuse an error not a crash). `recv` on a closed empty channel returns a documented result and never hangs.
- Also `mutex_new()`? NOT needed; skip. Shared mutable state is intentionally unsupported.
- Signals: `signal_watch(name)` for "INT", "TERM", "HUP" sets an atomic flag via a safe handler; `signal_pending(name) -> Bool` polls and clears it. Windows: only "INT".
- If the interpreter's value types are not Send, make the compiled program an Arc and give each thread its own mutable state. If a sound design is impossible, STOP and report with a concrete alternative; do not ship unsound code.
- Limits: configurable max threads (default 256) with E-THREAD-LIMIT; sandbox mode caps threads at 4 (E-SANDBOX-THREADS beyond that) or denies them.
- Tests: 100 threads incrementing results returned via channels, worker pool pattern, producer/consumer ordering, join error propagation, deadlock-free close semantics, a thread serving a TCP connection (A1), signal flag set via `kill -INT` of the child process, no leaked threads at exit.

## STAGE A3 — HTTP server and fuller HTTP client

Server:
- `http_serve(host, port, handler_fn_name, options) -> Server` where handler(req) returns a response map. `req` = {method, path, query (map), headers (map, lower-case keys), body (Str), remote}. Response = {status (Int), headers (map), body (Str)}; helper constructors `http_json(status, value)`, `http_text(status, s)`, `http_redirect(url)`.
- Options map: workers (default 4), max_body_bytes (default 1 MB), read_timeout_ms (default 10000), max_header_bytes. Built on A2 threads with a worker pool; every request handled in isolation (a handler error returns a 500 with a safe JSON body and logs the error; the server keeps running).
- `http_serve` must work on Render: bind to host "0.0.0.0" and a port from `get_env("PORT")`; handle HTTP/1.1 keep-alive and Connection: close; correct Content-Length; respond 400 to malformed requests, 413 to oversized bodies, 408 to slow clients; never crash on garbage input.
- `server_port(s)`, `http_stop(s)` (graceful: finish in-flight requests), and the serve loop returns when stopped. Provide a pattern in the docs: poll `signal_pending("TERM")` in the main thread then `http_stop` so Render deploys shut down cleanly.
- Routing helpers (small, optional, tested): `route_match(pattern, path)` returning a params map for patterns like "/notes/:id".
- Parse JSON request bodies with the existing JSON support; if JSON is only a pure-Klang package and too slow (a 1 MB body must parse in under 1 second), add native `json_parse(s)` and `json_stringify(v, pretty)` builtins in a new file, keeping the package working (avoid W-SHADOW problems; check how B1 handled this).
Client:
- `http_request(method, url, headers_map, body, timeout_ms) -> {status, headers, body}` with HTTPS (reuse the existing TLS stack used by the registry client; do not add a second TLS stack), redirects (follow up to 5; option to disable), and errors E-HTTP-*. Keep existing http_get/http_post working unchanged.
- Sandbox: all denied.
Tests (loopback): server on port 0 in a thread of the test with the client hitting it; 200 concurrent requests all correct; keep-alive reuse; malformed request line; oversize body -> 413; slow client -> 408; handler raising an error -> 500 and server still alive; graceful stop; query string and header parsing; JSON round trip. Include an example program examples/apps/hello_api/ (GET /health, GET /time, POST /echo) with a gate test.

## STAGE L1 — Datetime and compression (library stage)

Datetime (new file, for example src/stdlib/l1_datetime.rs; use the chrono crate, no timezone database):
- `dt_now() -> {epoch_ms, ...}` or a consistent representation: choose an epoch-milliseconds Int as the canonical instant type; functions: `dt_parse_iso(s) -> Int`, `dt_format(ms, pattern) -> Str` (strftime-style: %Y %m %d %H %M %S %f %z %a %A %b %B %j %p), `dt_add(ms, {days, hours, minutes, seconds, ms})`, `dt_diff_ms(a, b)`, `dt_parts(ms) -> {year, month, day, hour, minute, second, weekday, yday}` in UTC, `dt_parts_offset(ms, offset_minutes)`, `dt_from_parts(year, month, day, hour, minute, second) -> Int` (UTC), `dt_local_offset_minutes() -> Int`, `dt_is_leap(year)`, `dt_days_in_month(year, month)`.
- Errors: E-TIME-PARSE, E-TIME-RANGE, E-TIME-PATTERN. Never panic on out-of-range values. Named time zones are unsupported (document it).
Compression (pure-Rust crates only: flate2 with miniz_oxide backend, tar):
- `gzip_compress(s) -> hex or bytes-safe form` (match the binary strategy A1 chose: if no bytes type exists, compress to/from FILES: `gzip_file(src_path, dst_path)`, `gunzip_file(src, dst)`, and also string forms returning hex), `zlib_*` equivalents, `tar_create(dir, out_path)`, `tar_extract(tar_path, dest_dir)` with path-traversal protection (reject entries containing `..` or absolute paths: E-COMPRESS-UNSAFE-PATH) and a max output size (default 256 MB, option) E-COMPRESS-TOO-LARGE. Corrupt input gives E-COMPRESS-CORRUPT, not a panic.
- Sandbox: file-based functions follow the sandbox temp-dir rule.
Tests: round trips for strings and files (1 byte, 0 bytes, 10 MB), known gzip test vector, corrupt data, zip-bomb guard, tar round trip with nested directories and unusual names, path traversal rejection, datetime parse/format round trips for 50 timestamps including leap days, DST-free UTC math, invalid dates (Feb 30, month 13). Example program examples/apps/logship/ lite: group a log by day, gzip each day, print a summary table; gate test.

## STAGE A3b — PostgreSQL client

Goal: a Klang program on Render can use Render's managed Postgres via its DATABASE_URL.
- Builtins (new file): `pg_connect(url) -> Pg` (accepts postgres://user:pass@host:port/dbname?sslmode=require; password never printed in errors or logs), `pg_exec(pg, sql, params_array) -> {changes}`, `pg_query(pg, sql, params_array) -> array of maps (column name -> value)`, `pg_begin(pg)`, `pg_commit(pg)`, `pg_rollback(pg)`, `pg_close(pg)`. Parameters are ALWAYS bound ($1, $2...), never interpolated. Map Postgres types: integers -> Int, float/numeric -> Float or Str (document), text -> Str, bool -> Bool, null -> null, timestamps -> ISO Str, json/jsonb -> Str.
- Use a pure-Rust synchronous client (for example the `postgres` crate with a rustls TLS connector) so the Android/musl/Windows/macOS release builds keep working; do not use C libraries (no libpq). Support SCRAM-SHA-256 and TLS (sslmode=require and prefer; verify certificates by default, an explicit `sslmode=disable` for local tests). If a release target cannot build this crate, gate it behind a cargo feature `pg` that is ON by default and report which target needs it off, with a clean E-PG-UNAVAILABLE error when off.
- Errors: E-PG-CONNECT, E-PG-AUTH, E-PG-SQL (include the server's SQLSTATE and message, never the password), E-PG-TYPE, E-PG-CLOSED, E-PG-TX.
- Sandbox: denied (E-SANDBOX-NETWORK).
- Testing: try to get a local Postgres in this Colab VM (`apt-get install -y postgresql` then start it with a temp data directory under /tmp and a random port, as a non-root user via `su postgres` or pg_ctl) and run the real tests against it. Tests that need Postgres run only when env KLANG_TEST_PG_URL is set (the stage's own test script `scripts/a3b_pg_test.sh` sets up the temp server, runs `cargo test --test a3b_gates` with the variable, and tears it down). Tests that do not need a server (URL parsing, error mapping, password redaction, sandbox denial, JIT/v2 rejection) always run. Report clearly whether you could run the live-database tests.
- Example program examples/apps/notes_api/: a notes REST API (GET /health, POST /notes, GET /notes, GET /notes/:id, DELETE /notes/:id) using http_serve (A3) and Postgres, reading DATABASE_URL and PORT from the environment, creating its table on startup, with a gate test that runs it when the Postgres URL is available.

## STAGE R1 — Render deployment kit

Deliver a ready-to-deploy example backend and the files Render needs. You cannot deploy; the human will.
- examples/apps/render_demo/: a Klang backend (notes API with Postgres from A3b, API-key auth via an `API_KEY` environment variable, JSON structured logs to stdout, `/health` route, graceful shutdown on SIGTERM, request size limits). README.md with exact deploy steps.
- A Dockerfile that produces a small image containing the `klang` binary and the app. Two build options, both documented and both tested as far as possible without Docker access (validate syntax with `docker` if available, otherwise carefully by reading): (1) Option A, build from source in a multi-stage Dockerfile (rust image builds `cargo build --release --locked`, copy the binary to a slim runtime image); (2) Option B, download a prebuilt release binary for a given version (ARG KLANG_VERSION) from the project's GitHub releases (linux x86_64 musl asset naming: check .github/workflows/release.yml for the exact artifact names) and verify it. Default to Option A. Use a non-root user, EXPOSE the PORT, HEALTHCHECK.
- render.yaml Blueprint: one web service (docker runtime, health check path /health, env vars API_KEY (generate: true) and PORT handled by Render, DATABASE_URL linked from the database) and one managed Postgres database. Use current Render Blueprint syntax; state the Blueprint spec version you relied on and flag anything you are unsure of.
- A local smoke script scripts/render_demo_smoke.sh that starts the app against a local Postgres (like A3b), runs curl checks for /health and CRUD with and without the API key, sends SIGTERM and checks a clean exit within 5 seconds. Gate test tests/r1_gates.rs runs the non-Docker parts (the script when Postgres is available; otherwise only the in-memory/validation parts).
- Docs you MAY edit in this stage: README.md (add a short "Deploying a Klang backend on Render" section) and docs/progress/R1.md. Mention the free-tier caveats: the filesystem is ephemeral (use Postgres), the service sleeps when idle, and cold starts.

---------------------------------------------------------------------
## After the queue (the human will ask separately)
- L1-docs: add verified sections for all new builtins to docs/ai/ (core, stdlib, pitfalls, errors) and regenerate KLANG_FOR_AI.md; the docs gate test must pass.
- Release: tag v0.8.0 (builds all six targets), verify, then `npm i -g klang-cli` gives everyone the new language.

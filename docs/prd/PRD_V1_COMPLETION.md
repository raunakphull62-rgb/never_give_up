# Klang v1.0 Completion PRD (Wave 2 — final feature wave)

Status: FINAL scope for v1.0. Two agents work in parallel lanes. Author: Claude, for Raunak Phull.
Repo: raunakphull62-rgb/never_give_up (origin/main). Reference implementation = the interpreter.

---------------------------------------------------------------------
## 0. Goal and definition of "complete"

Klang v1.0 is "complete" when one person can build a complex real application in it without leaving the language. Concretely, ALL FOUR capstone programs in section 6 must run and pass their gates:

- C1 pastebin: HTTP server + SQLite storage + JSON, with a CLI client.
- C2 chat: concurrent TCP chat server and client.
- C3 ffi-demo: calls C functions from system libraries (libc/libm) through FFI.
- C4 logship: reads logs, compresses them (gzip), uploads over an HTTP client, with timestamps.

Everything in this PRD exists to make those four possible, plus tooling (tests, perf, cross-platform, docs) so a newcomer can use the language.

### Explicit NON-goals for v1.0 (do not build, do not promise)
- JIT and v2 backend parity. They stay experimental, opt-in, and reject unsupported programs loudly. The interpreter is the reference.
- AI-reliability benchmark run; "trust/stability" hardening items (user excluded them; they come after v1.0).
- GUI, WASM target, debugger, package signing, a garbage-collector rewrite.
- iOS. Android = Termux only.

---------------------------------------------------------------------
## 1. Ground rules (apply to EVERY stage and both agents)

1. Base: start from current origin/main. Record the baseline `cargo test --locked` count at the start and NEVER let it go down. Expected baseline once S9 is merged: 870 + S9 tests.
2. Never push, tag, publish, or force anything. Commit locally on your own branch. The human merges and pushes.
3. Every commit must compile from a clean checkout. No broken intermediate commits.
4. Each stage adds its tests in its OWN file: tests/<stage>_gates.rs (for example tests/a1_net_gates.rs). Minimum 10 tests per stage unless the stage says otherwise. Do not weaken or edit existing tests; if an expectation must change, say so explicitly in your final report with the reason.
5. Put new code in NEW files wherever possible (for example src/stdlib/net.rs). Touch shared files (src/hir.rs, src/runtime/mod.rs, src/main.rs, src/stdlib/mod.rs, docs) as little as possible: append registration lines at the end of lists. This is how two agents avoid merge conflicts.
6. Documentation: do NOT edit the shared docs (reference.md, limitations.md, LANGUAGE_STATUS.md, SPEC.md, README.md) inside a feature stage. Write docs/progress/<stage>.md (what changed, builtin signatures, error codes, limits) and docs/progress/<stage>-notes.md (proposed text for shared docs). Stage B8 merges all of it into the shared docs.
7. New builtins and syntax: the JIT and v2 must reject them loudly (like spawn/run/now_ms do today). Add the rejection to the parity allowlist with a one-line reason. Parity gate must stay green.
8. Errors: every new failure mode has a stable error code of the form E-<AREA>-<REASON> (for example E-NET-REFUSED, E-NET-TIMEOUT), a message that names the value involved, and a real source span (S2 gives spans). Never panic on bad user input. Add a test that proves no panic for each error path.
9. Formatting: `klang fmt --check` must pass on every .klang file you add.
10. Network in tests: tests may use ONLY loopback (127.0.0.1, port 0 for an OS-chosen free port, never a fixed port). Any test that needs the real internet is skipped unless env KLANG_TEST_NET=1. Tests must be deterministic and must not leave processes or sockets behind.
11. Long cargo runs exceed the 10-minute shell limit: run with `nohup cargo test --locked > /tmp/<stage>.log 2>&1 &` and poll the log. Use your own CARGO_TARGET_DIR (/tmp/t1 for agent 1, /tmp/t2 for agent 2). Disk is limited: delete finished target dirs.
12. Security defaults: network, FFI, and spawn are powerful. In sandbox execution mode (`klang run --sandbox`) they are DENIED by default (see section 2.7). Plain `klang run` from a terminal allows them.
13. Ask nothing mid-stage unless a decision cannot be undone. Take the most reasonable reading, state it at the top of docs/progress/<stage>.md, continue.
14. Final report for each stage: commits (hash and subject), test count (baseline, final, 0 failed), what was built, what could not be built and why, any design deviation from this PRD.
15. If something this PRD says conflicts with the code, trust the code, and record the mismatch in the report.

---------------------------------------------------------------------
## 2. Design decisions (made here; the user may revise before building)

### 2.1 Concurrency model (D7)
Read docs/design/D7-io-and-concurrency.md and the existing task_group/spawn/await tasks first, and keep them working. Add share-nothing OS threads with channels:
- `thread_spawn(fn_name, args_array) -> Thread` runs a named top-level function on a new OS thread with its OWN interpreter state. Arguments and results are deep-copied (no shared mutable state). Closures are not allowed as thread bodies in v1.0 (compile-time E-THREAD-CLOSURE).
- `thread_join(t) -> value` (returns the function result or re-raises its error as E-THREAD-FAILED naming the thread and the cause).
- `chan(capacity) -> Chan`, `send(ch, v)`, `recv(ch)` (blocks), `recv_timeout(ch, ms)` (null on timeout), `try_recv(ch)`, `close(ch)`; recv on a closed empty channel returns a documented sentinel or error (choose one, document it, test it). Channels may be passed to threads (channel handles are the only shared objects) and carry deep-copied values.
- `sleep_ms` already exists. Add `thread_id()`.
- If the interpreter's value or program types are not Send, make the immutable compiled program an Arc and give each thread its own mutable state. If a truly safe design is impossible, STOP and report with a concrete alternative; do not ship unsound code.
Deadlock safety: `recv` on a channel with no remaining senders and an empty buffer must not hang forever (return the closed-channel result).

### 2.2 Networking
- TCP: `tcp_listen(host, port) -> Listener` (port 0 = any), `listener_port(l)`, `tcp_accept(l)`, `tcp_connect(host, port, timeout_ms) -> Conn`, `conn_read(c, max_bytes)` (empty string on peer close, documented), `conn_read_line(c)`, `conn_write(c, s)`, `conn_close(c)`, `conn_set_timeout(c, ms)`, `conn_peer(c)`. Strings are UTF-8; add bytes-safe variants only if Klang already has a bytes type (check; if there is none, add a minimal documented approach: base64 or hex helpers).
- UDP: `udp_bind`, `udp_send_to`, `udp_recv_from` (small, tested on loopback).
- DNS: `dns_lookup(host) -> array of ip strings`.
- Handles (Listener, Conn, Thread, Chan, Db, Lib) are opaque values with a clear type name in errors and a leak-safe close (closed on program exit).
- HTTP client: `http_request(method, url, headers_map, body, timeout_ms) -> {status, headers, body}`; helpers `http_get(url)`, `http_post(url, body)`. HTTPS must work (reuse the TLS stack the registry client already uses; it must keep working on Android/musl/Windows/macOS: do not add a second TLS stack). Redirects: follow up to 5, configurable off.
- HTTP server: `http_serve(host, port, handler_fn_name)` where handler gets `{method, path, query, headers, body}` and returns `{status, headers, body}`. Concurrency: a fixed worker pool built on 2.1 threads (default 4 workers, option `workers`). Graceful stop via `http_stop(server)` and via signals (2.3). `http_serve` with port 0 and a `ready` callback/handle so tests can learn the port.
- Limits: max request line/header size and max body size with E-HTTP-TOO-LARGE; slow-client timeouts; malformed requests give a 400 and never crash the server.

### 2.3 Signals
Safe flag model (no async-signal-unsafe user code): `signal_watch(name)` installs a handler that sets an atomic flag for "INT", "TERM", "HUP"; `signal_pending(name) -> Bool` polls and clears. Windows supports "INT" only (documented). `http_serve` and long loops can poll it.

### 2.4 FFI (D5) — decision
- Syntax: a top-level `extern "lib" { fn name(a: Int, b: Str) -> Int }` block (check the parser style; match the existing declaration style). `lib` is a library name resolved via the platform loader (for example "libc.so.6", "libm.so.6", "libm.dylib", "msvcrt.dll"); also allow a `load_lib(path)` runtime form that returns a Lib handle and `lib_call(lib, "name", sig, args)` is NOT required.
- Types allowed across the boundary: Int (as C long long/int64), Float (double), Bool (int), Str (null-terminated UTF-8 char*, copied; returned char* is copied and NOT freed unless the declaration says `owned`), and `Ptr` (opaque integer-sized handle, no arithmetic). Up to 6 arguments. No callbacks, no structs by value in v1.0 (document as a limit).
- Implementation: dlopen/LoadLibrary via the libloading crate, with a small fixed set of call shims by signature class (pure Rust, no libffi C dependency, so cross-compiling stays easy). If libffi turns out to be needed for generality, report it as a decision instead of adding it silently.
- Safety: FFI is denied in sandbox mode. In normal mode it requires an explicit opt-in: `--allow-ffi` flag or `allow_ffi = true` under [project] in klang.toml, otherwise E-FFI-DENIED with the fix in the message. All FFI errors have codes (E-FFI-LIB-NOT-FOUND, E-FFI-SYMBOL-NOT-FOUND, E-FFI-TYPE).
- JIT/v2 reject extern loudly.

### 2.5 Standard library stages
- Audit first (stage B1): list what exists. Needed for complex programs, and each must exist and be tested: JSON parse/stringify (with a clear error on bad input and pretty-print option), regex (match, find_all, replace, captures; use the `regex` crate), CSV parse/write, base64 and hex, sha256 (exists; keep), random (seeded), uuid (v4), path utils (join, dirname, basename, extension, normalize, absolute), string utils (trim, split, join, replace, starts_with, ends_with, pad, repeat, lines, to_upper/lower, format with placeholders), file utils (read_file, write_file, append_file, exists, is_dir, list_dir, make_dirs, remove_file, rename_file, stat size and mtime), env/args (exist), exit codes, and a `printf`-style or interpolation formatter (check what exists; add only the missing).
- datetime (B2): real, not a stand-in: parse ISO-8601, format with strftime-style patterns, add/subtract durations, difference, weekday, day-of-year, UTC and fixed offsets and the machine local offset. Use the `chrono` crate (no timezone database in v1.0; document that named zones like "Europe/Paris" are unsupported).
- compress (B3): gzip and zlib compress/decompress for strings and files, plus `tar` create/extract of a directory. Pure-Rust crates only (flate2 with miniz_oxide backend, tar). Decompression bomb guard: a max output size with E-COMPRESS-TOO-LARGE.
- sql (B4): SQLite via rusqlite. API: `db_open(path)` (":memory:" allowed), `db_exec(db, sql, params_array) -> {changes, last_id}`, `db_query(db, sql, params_array) -> array of maps`, `db_close(db)`, transactions `db_begin/commit/rollback`. Parameters are always bound (no string-built SQL in examples), errors are E-SQL-*.
  RISK: rusqlite's bundled SQLite compiles C code. The release matrix (linux x86_64, linux arm64 musl, android arm64 NDK, windows, macOS x2) must still build. Put sql behind a cargo feature `sql` that is ON by default, verify the release workflow via workflow_dispatch (the human runs it), and if a target fails, turn the feature off for that target only with a clean E-SQL-UNAVAILABLE error message, not a build break.
- Each library lives in its own file under src/stdlib/ and is registered at the end of existing lists.

### 2.6 Test framework and diagnostics
- `klang test [path]`: discovers files named *_test.klang and functions named test_*; assertions `assert(cond, msg)`, `assert_eq(a, b)`, `assert_ne`, `assert_err(fn_name, code)`; reports pass/fail per test with spans, exit code 1 on any failure, `--json` output for tools. Each test runs isolated. (First check what already exists and extend it; do not duplicate.)
- Runtime errors print a stack trace: function names with file:line:col per frame (top 20 frames, with "... N more" for deep recursion). Keep existing error codes stable and keep the JSON diagnostic shape backward compatible (add a `stack` array).

### 2.7 Sandbox mode
There is a file-access guard (absolute /tmp only, no `..`) that S9 limits to sandbox mode. Define sandbox mode precisely: `klang run --sandbox` (X1: the MCP server and its `klang_run` tool were removed; sandbox is now CLI-only). In sandbox mode the following are DENIED with a clear E-SANDBOX-* code: network, FFI, spawn/run, threads beyond a small cap, file access outside the temp dir. In normal mode all are allowed. Test both modes for each powerful feature (this is a gate for stages A1, A2, A3, A5).

### 2.8 Interpreter performance
Fixed budget gates (on the CI machine class, generous so they do not flake): fib(30) recursive, a 10,000,000-iteration integer loop, building a 1,000,000-character string by appending in a loop, sorting 100,000 integers, 100,000 map inserts and lookups, and parsing a 5 MB JSON document. If any one is more than 5x slower than a plain-Python equivalent run on the same machine, profile and fix hot spots (string append must be amortized O(1)/O(n) total, not O(n^2); array push amortized O(1); map ops O(1) average; function-call overhead). Record before/after timings in docs/progress/B6.md. Do not weaken correctness (checked arithmetic, spans) for speed.

---------------------------------------------------------------------
## 3. Two lanes (parallel, minimal overlap)

Agent 1 = Lane A "systems": runtime features. Agent 2 = Lane B "library and tooling". Each agent works only in its lane. Stages inside a lane run in order. The human merges ONE STAGE AT A TIME into main, runs the full suite, pushes, and then resets the agent's worktree to main before the next stage.

### Lane A (agent 1)
- A1 Net core: TCP, UDP, DNS, handles, timeouts, sandbox denial (2.2, 2.7).
- A2 Threads and channels (2.1) plus signals (2.3).
- A3 HTTP client and server (2.2) on top of A1 and A2. Needs real loopback tests: server in one thread, client calls it, check status codes, headers, large bodies, keep-alive or close behavior, malformed request, concurrent clients (50 parallel requests all answered), graceful stop.
- A4 FFI (2.4) with tests that call real libc/libm functions (strlen, getpid, abs, sqrt, pow, toupper). Tests must skip cleanly on a platform without the library name.
- A5 Capstones C2 (chat) and C3 (ffi-demo) from section 6, as examples/apps/ with gates.

### Lane B (agent 2)
- B1 Stdlib audit and gap fill (2.5): write docs/progress/B1-audit.md first (a table: needed item, exists?, where, tested?), then fill every missing item. JSON and regex first.
- B2 datetime (2.5).
- B3 compress and tar (2.5).
- B4 sql / SQLite (2.5) with the cargo-feature safety net.
- B5 `klang test` framework and stack traces (2.6).
- B6 Interpreter performance pass (2.8).
- B7 Cross-platform CI: extend .github/workflows/ci.yml to run `cargo build --locked` and `cargo test --locked` on ubuntu-24.04 (pinned), windows-latest and macos-latest. Fix portability failures (path separators, line endings, temp dirs, process spawning, file permissions) in code or tests without disabling tests; a test may be skipped on one OS only with a comment and a documented reason. You cannot run Windows or macOS locally, so write the changes carefully, mark in the report which fixes are unverified, and the human will push and read the CI results to iterate. Also pin ubuntu-24.04 in release.yml.
- B8 Documentation: (a) merge all docs/progress/*-notes.md for A1..A5 and B1..B7 into the shared docs (reference.md, limitations.md, LANGUAGE_STATUS.md, SPEC.md, README.md, design/README.md decision log: D5 and D7 become DECIDED with the choices in section 2); (b) docs/tutorial/: eight short chapters (hello and basics; types and structs and enums; errors and try/catch; modules and packages; files and processes; HTTP and networking; concurrency; testing and tooling), every code sample is extracted and run by a gate test so docs never rot; (c) examples/README.md index of all example programs.
- B9 Capstones C1 (pastebin) and C4 (logship) from section 6.

Dependencies: C1 needs A3 (HTTP) and B4 (SQL) and B1 (JSON); do B9 only after those are merged. C4 needs B3 and A3. If Lane B finishes before A3 is merged, the agent waits or does B8 part (a) first.

Suggested human merge order: S9 (already given), A1, B1, A2, B2, A3, B3, A4, B4, B5, A5, B6, B7, B8, B9, then final verification (section 7).

---------------------------------------------------------------------
## 4. Per-stage definition of done (checklist the agent copies into its report)

- [ ] All items in the stage scope built as specified, or each deviation explained.
- [ ] tests/<stage>_gates.rs with the minimum tests; error paths each have a test; no panics on bad input.
- [ ] Baseline test count not reduced; full `cargo test --locked` passes from a CLEAN checkout (use `git worktree add /tmp/clean-<stage> <commit>` and run there).
- [ ] `klang fmt --check` passes on new .klang files.
- [ ] JIT/v2 reject new constructs loudly; parity gate green.
- [ ] Sandbox-mode denial tested where required.
- [ ] docs/progress/<stage>.md and <stage>-notes.md written; shared docs untouched (except B7/B8).
- [ ] No leftover processes, sockets, temp files; no fixed ports; no network use beyond loopback unless KLANG_TEST_NET=1.
- [ ] Final report in the format of rule 14.

---------------------------------------------------------------------
## 5. Language gaps possibly needed for complex programs (audit, do not assume)

During B1 (agent 2) and A1 (agent 1), check whether these exist and report; add only if missing AND required by a capstone:
- A bytes type or documented binary-safe strategy for network and compression data.
- Maps/dictionaries with iteration order documented, array methods (map, filter, reduce, any, all, find, contains, reverse, sort, index_of, push/pop, extend, slice), string indexing by chars not bytes (the S7 slice/len bytes-vs-chars issue must be fixed or documented with a safe helper).
- Closures that capture variables correctly, plus first-class function values passed to sort_by and friends.
- Optional/null handling convention, Result-like error handling story (try/catch exists): document the idiom.
- Integer parsing/formatting (hex, binary), float formatting with precision.
- Multi-line string literals.
If a capstone needs one of these and it is missing, the agent that hits it adds it as a small, tested, separately-committed change (own file tests/<stage>_lang_gates.rs) and tells the human in the report.

---------------------------------------------------------------------
## 6. Capstone programs (acceptance gate for v1.0)

All under examples/apps/<name>/ with a README, fixtures, and a gate test in tests/capstone_<name>.rs that runs the program for real over loopback or temp files. Programs must be idiomatic Klang, formatted, and run on the interpreter.

C1 pastebin (stages: A3, B1, B4)
- `server.klang`: HTTP server with routes POST /paste (JSON {title, body, ttl_seconds optional}) returns {id}; GET /paste/<id> returns JSON; GET /paste/<id>/raw returns text; DELETE /paste/<id>; GET /list returns the last 20 pastes. Persists in SQLite. Invalid JSON returns 400 with an error JSON; unknown id returns 404; ttl expiry works.
- `client.klang`: CLI: `client put <file>`, `client get <id>`, `client list`.
- Gate: start the server on port 0 in a thread of the test, run 100 mixed requests concurrently from 8 client threads, verify counts and contents, restart the server on the same database file and verify the data persisted.

C2 chat (stages: A1, A2)
- `server.klang`: TCP chat server: clients connect, send `/nick name`, then lines are broadcast to all others, `/who` lists users, `/quit` leaves; per-client thread, channel-based broadcast hub, graceful shutdown on SIGINT/TERM flag.
- `client.klang`: reads stdin lines and prints incoming messages concurrently.
- Gate: start server, connect 20 scripted clients, verify every broadcast reaches all other clients exactly once and in per-sender order; verify a client dropping mid-message does not crash the server.

C3 ffi-demo (stage: A4)
- Calls libc/libm through extern: strlen, getpid, toupper, sqrt, pow, plus a manual dlopen of a path given on the command line. Prints results. Gate: results match known values; --allow-ffi missing yields E-FFI-DENIED; sandbox mode yields E-SANDBOX-*.

C4 logship (stages: B2, B3, A3)
- Reads a log file, groups lines by day (datetime parse), gzips each day's file into a .tar.gz, uploads each archive with http_post to a local test HTTP server (started in the gate) that verifies size and checksum, prints a summary table. Gate: archives decompress to the originals byte-for-byte; the server received every file; bad network target gives a clear error and exit code 1.

---------------------------------------------------------------------
## 7. Final verification by the human (not the agents)

1. All stages merged one at a time; full suite green; CI green on ubuntu, windows, macOS (B7).
2. Run release workflow via workflow_dispatch; all 6 targets build (this is mandatory because of SQLite and the FFI crate).
3. Bump Cargo.toml version, tag v0.8.0 (Wave 1 plus Wave 2), confirm `klang --version` prints the tag version (first test of the version-stamp step), install on Termux and run C3 and the tutorial hello program there.
4. Run all four capstones by hand once. Then tag v1.0.0.
5. Registry hardening before v1.0.0: rotate the registry token, test wrong-token publish returns 401, test package data survives a Render restart.

---------------------------------------------------------------------
## 8. How to give this to the agents (copy-paste prompt)

Agent prompt template (replace STAGE and the lane):

```
You are agent N working in /content/wtN (branch agentN, reset to main). Read the file /content/work/docs/prd/PRD_V1_COMPLETION.md fully. Do STAGE <id> only (scope in section 3, design in section 2, rules in section 1, checklist in section 4). Do not start other stages. Commit locally, never push. Final report in the format of rule 14.
```

End of PRD.

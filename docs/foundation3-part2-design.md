# KLANG-FOUNDATION-3 Part 2 — Infrastructure Design Decisions

Status: **§A (async I/O) is implemented as specified below
(`http_get_async`/`http_post_async`, `tests/async_net_gates.rs`); §B
is implemented as specified; §C is now implemented as specified
(`klang lsp`, `src/lsp.rs`, `tests/lsp_gates.rs` — whole-file re-check
with 100 ms debounce, `eval.klang` re-check measured at ~39 ms p95, so
the Salsa path was not needed); §D is design-only (no code).**
Per the PRD, §§C–D compile nothing until reviewed. Each section
answers its questions in writing, flags uncertainties rather than
guessing, and states explicit out-of-scope lists.

Codebase facts this document relies on (all verified against the tree,
not assumed):

- Concurrency = one OS thread per `spawn`, `await` = `join`
  (`src/runtime/mod.rs:606-649`); limits `MAX_CONCURRENT_TASKS = 256`,
  `SPAWN_STACK_BYTES = 8 MiB`; cancellation observed **only** at loop
  back-edges (`Jump` with `target < pc`), no task-entry checkpoint.
- Effects: `throws` propagation-checked; `async` required for
  `task_group`/`spawn`/`await` (`E-EFFECT-MISMATCH`) but
  **annotation-only at runtime**; `cancel` parsed but **never checked**.
- Every I/O builtin blocks the calling thread: `ureq` sync HTTP
  (10 s connect / 60 s global), `std::fs`, `Command::output()`,
  `thread::sleep`, blocking stdin read. Blocking builtins delay group
  failure drains (joins wait for them).
- Packages: `klang.toml` path-deps only; lockfile `<file> <fnv-hex>`
  lines; zero registry/network code existed before 2B.
- `src/lsp.rs` is a 32-line JSON renderer stub (hardcodes `line: 0`,
  `severity: 1`); `src/mcp.rs` has a reusable hand-rolled `Json`
  parser/renderer plus a stdio JSON-RPC loop.
- No debugger/profiler/tracing infrastructure exists (zero hits for
  breakpoint/DAP/instrument/trace/profiling).

---

## A. Async I/O runtime

**A1. What goes non-blocking first? Network I/O only** (`http_get`,
`http_post`). File, stdin, process, and sleep builtins stay blocking.
Justification: network has unbounded, server-dependent latency (up to
the 60 s global timeout) and is the only operation where "waiting" is
the norm rather than the exception; file/process/sleep waits are
bounded and local. Scoping to network keeps the first version useful
(API polling, webhooks, multi-fetch fan-out via existing `spawn`) while
containing the semantic blast radius.

**A2. Mechanism: thread-pool offload, honestly labeled — NOT true
async I/O.** A bounded worker pool (fixed N threads, FIFO queue) runs
the existing blocking `ureq` calls; the calling Klang task parks until
its result arrives. Tradeoff stated plainly: this is *simulated* async
(concurrency without parallelism gains beyond N threads, no
`epoll`/`kqueue` scalability, one thread per in-flight request). A
crate like `tokio`/`mio` is explicitly rejected for v1 because true
OS-level async would require an executor permeating `run_instrs`,
`spawn`/`await` join semantics, and the checker — a rewrite of the
runtime, not a first version. The thread-pool version must be
documented as "blocking calls on pool threads" wherever it is
described, never marketed as equivalent to real async.

**A3. Effect-system interplay: one new enforcement, reusing the
existing gate.** `http_get_async`/`http_post_async` require the `async`
effect on the caller (`E-EFFECT-MISMATCH`, same code and rule family as
the `task_group`/`spawn`/`await` gate — see `tests/async_net_gates.rs`
`async_requires_async_effect`). Fan-out helpers spawned from within a
`task_group` satisfy this via their own `async` annotation; lexical
group containment is NOT required at the call site, since a spawned
helper runs within the group's lifetime dynamically (a lexical-group
requirement would forbid exactly the fan-out pattern the pool exists
for — stated interpretation, not a silent relaxation). `cancel` stays
unchecked in v1, which creates a known limit rather than a soundness
hole: a pooled network call observes cancellation only at the same loop
back-edges as today, so a slow fetch still delays group drains. Mitigation (not a redesign):
per-call timeout parameters already exist via the global timeouts; v1
adds nothing. If a later version wants cancellable fetches, THAT is
when `cancel` checking and executor integration get designed — stated
here so nobody assumes `cancel` means anything today.

**A4. Explicitly OUT of scope for v1:** async file I/O, async stdin,
async process spawn, real OS async (`epoll`/`kqueue`/`IOCP`,
`tokio`/`mio`), `cancel` enforcement, per-request timeout plumbing
beyond existing globals, backpressure/queue-depth configurability
(fixed pool size constant, documented).

**Uncertainty flagged:** the exact pool size and queue policy (fixed N
= 16? unbounded?) is left to implementation measurement, not guessed
here — but unbounded is rejected in advance (matches the existing 256
task cap philosophy).

---

## B. Package registry / network packages (hosted, on Render)

**As built (this section describes the shipped v1, not a proposal).**
Two reviewed deviations from the pre-approved sketch, both disclosed:

1. Storage is **file-backed** (`index.json` + `<name>/<version>.kpkg`
   on a Render persistent disk), not Postgres. Reason: Postgres code
   could not be verified from the build environment, and shipping
   untested storage code would violate project discipline. Postgres is
   the documented migration path when the index outgrows files.
2. Archives are a minimal owned container (`KLANGPKG1`), not tar.gz:
   std-only (no compression deps), deterministic (sorted entries, so
   re-packing verifies hashes), with explicit caps. tar.gz is the
   documented migration path.

**B1. Architecture.** One Rust service (`klang-registry` binary, same
toolchain, std-only `TcpListener` networking — no tokio), thread per
connection. Routes: `GET /` (health), `GET /api/packages` (index),
`GET /api/packages/:name` (metadata: versions + sha256 + size +
`latest`), `GET /api/packages/:name/:version/download` (raw archive
bytes), `POST /api/publish?name=&version=` (raw archive body, bearer
auth). Name/version charsets are validated before any filesystem use
(`[A-Za-z0-9_-]+`, `X.Y.Z` numerics), so traversal is impossible by
construction. Persistence is write-through (`index.json` via
tmp-file + rename); startup drops index entries whose archives are
missing (fail closed, logged).

**B2. Publishing.** `klang publish [--dir] [--registry]` packs
`*.klang` (recursive, relative paths preserved) + root `klang.toml`
as provenance, enforcing caps (512 files, 1 MiB/file, 8 MiB total),
name safety, and no symlinks — client-side AND re-validated
server-side (never trust the client). Minimum viable auth: a single
admin bearer token (`REGISTRY_ADMIN_TOKEN` env; `--token` flag
overrides; env preferred and documented so tokens avoid shell
history). No GitHub OAuth, no accounts, no teams — stated
simplification, adequate for a single-publisher v1.

**B3. Resolution.** `klang.toml` keeps path deps; registry deps read
`calc = "registry:calc@1.0.0"` (exact pins only — no ranges, no
`latest` in manifests; the server reports `latest` as human metadata
only). `klang add name@version` appends the pin and fetches;
`klang fetch` vendors everything; `run`/`check`/`build` auto-fetch
when online (`--registry` / `KLANG_REGISTRY` select the server,
`--offline` forbids network). Imports need no new syntax:
`import "calc/lib.klang"` falls back to the pinned vendor dir
(`.klang_pkgs/<name>/<version>/`) only after the relative read fails —
local files always win (documented precedence). The existing lockfile
format is extended additively (`package <name> <version> <sha256>`
lines; old file-hash lines and old parsers pass them through
untouched).

**B4. Security (non-negotiable, as built).** SHA-256 recorded at
publish, re-verified on every download, pinned in `klang.lock`; drift
on either side (registry-side swap or local tamper) fails loudly
(`E-IMPORT`) and the bad bytes never execute — the lock, not the
vendor cache, is the trust anchor (a corrupted cache re-downloads;
a corrupted lock errors). Pinned by `tampered_vendor_offline_fails`,
`lock_pin_conflict_fails`, `stale_vendor_refreshes_online`.
Namespace policy: first-come-first-served, stated as a known
limitation. Transport is trusted (localhost or Render's TLS; the
service speaks plain HTTP) and token distribution is out of band —
both stated, not silently assumed.

**B5. Explicitly OUT of scope for v1:** yanking/deletion, private
packages, web UI (CLI + raw JSON API only), version ranges,
transitive registry dependencies (packages must be self-contained),
per-user auth/OAuth/teams, request logging, Postgres/object storage
(both documented migration paths).

**B6. Deliverable (per review change).** No Render credentials were
requested or used. Verified: full local end-to-end (ephemeral
localhost server in `tests/registry_gates.rs`: publish → depend →
`klang run` returns computed values; plus live CLI runs during
development). Shipped for the operator: `render.yaml` (Rust service,
persistent disk, health check, `sync: false` token — no secrets in
repo) + `docs/deploy-registry.md` (exact dashboard steps, first
publish, first dependent project, integrity model, operating notes).
The operator creates the Render service themselves from those docs.

---

## C. LSP server

**C1. Minimum real v1: diagnostics-as-you-type, and nothing else.**
Rationale: it is the only feature that is (a) unambiguously useful on
day one, (b) implementable purely on top of the existing
parse → check pipeline with no new analysis. Hover and go-to-definition
are explicitly **v2, documented as such** — they require scope-aware
name resolution the checker does not currently expose as a query, and
building a shallow approximation would be exactly the kind of fake
surface the ground rules forbid.

**C2. What it builds on.** The existing `Diagnostic::to_json` shape
from `klang check` (codes, spans, fixes, `related`) is the message
content. The 32-line `src/lsp.rs` stub is **replaced, not reused**: it
hardcodes `"line": 0` and `severity: 1`, which destroys position
information — v1 must compute real line/character positions from byte
spans (a small, honest, fully testable function). Protocol framing
reuses the pattern already proven in `src/mcp.rs` (hand-rolled JSON +
stdio loop): **no `tower-lsp`, no tokio**, stated cost avoided (two
heavy async deps for a synchronous checker would be absurd).

**C3. Incremental strategy, stated explicitly.** v1 re-checks the
whole file on every change notification with a debounce — acceptable
because `check` is a single linear pass that the existing suite runs
in milliseconds on real files, WITH a measurement gate at
implementation time: if p95 check latency on `examples/eval.klang`
exceeds ~200 ms, v1 instead routes through the existing Salsa
incremental database (`src/db.rs`) rather than shipping slow. No
guessing here — the number decides. (Uncertainty flagged: the 200 ms
budget and the eval.klang corpus are proposals; review may adjust
either, but some gate must exist before "fast enough" is claimed.)

Out of scope for v1: hover, go-to-definition, rename, formatting via
LSP (the `fmt` CLI already exists), multi-file project model beyond
what `load_with_imports` already does, debounce tuning beyond a
constant.

---

## D. Debugger/profiler

**Debugger mechanism (when built, not now): interpreter
instrumentation.** Breakpoints keyed by `NodeId`/MIR origin, checked
in `run_instrs` (v1 interpreter); **explicitly interpreter-only —
the JIT path is out of scope** (machine-code breakpoints are a
different project involving Cranelift safepoints, stated so nobody
plans on it accidentally). Pause/resume/step semantics ride on the
existing `ExecFlow` plumbing; variable inspection reads the current
`values` map. No DAP wire protocol in v1 (a simple line-based or
JSON-RPC control channel reusing the MCP pattern, stated).

**Profiler first metric: call counts.** One realistic target, chosen
over wall-clock and memory for testability: counts are deterministic
(no timing flakiness in gates), cheap (one counter per function), and
directly answer the most common performance question ("what runs how
often"). Wall-clock per-function (noisy, needs sampling infrastructure
that does not exist) and memory profiling (needs allocator hooks the
owned-`Value` model does not expose) are deferred with reasons, not
silently dropped.

**Scope conclusion (the legitimate answer the PRD allows): D is
design-only in this round and needs its own dedicated PRD after A–C
land.** Reasons: A–C already fill implementation capacity; a debugger
without a protocol and a profiler without a metric would be placeholder
surfaces, and the ground rules forbid placeholders. Nothing in D
compiles until that PRD is reviewed. This section is the complete
design record for that future PRD to build on.

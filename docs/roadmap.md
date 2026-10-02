# Build Roadmap (current state as of KLANG-FOUNDATION-3)

This file used to describe "Stage 1.5" as current work and an MCP
server as a future goal. Both are stale: enums/generics/modules
shipped (SPEC §7), and `klang mcp` exists (`src/mcp.rs`,
`tests/mcp_gates.rs`). Rewritten to match what `cargo test` proves.

## Done and pinned by tests

- Typed core + effects (`throws`/`async`/`cancel`), structured JSON
  diagnostics, structured concurrency (`task_group`/`spawn`/`await`),
  Salsa incremental compilation, formatter, basic package manager
  (`klang.toml` + lockfile).
- Maturity: enums + `match` with exhaustiveness, generics with
  per-site inference, multi-file modules (`mod` + `pub` + `import`
  merging).
- Stdlib: OS file/env/process/regex/time/HTTP/net interop, variadic
  `format()`, stdin (`read_line`/`parse_int`/`parse_float`),
  `for k in map` + array `insert`, `try`/`catch` recoverable errors.
- Backends: tree-walk interpreter (default) + int-only Cranelift JIT
  behind `--backend-jit` (anything non-int is a loud rejection, never
  silent wrong code).
- Repair (Phase 4): Klang no longer calls a model — the harness owns
  the model connection and drives the loop via `klang mcp`
  (`klang_check`, `klang_run`, `klang_fmt`, `klang_scope_plan`);
  `klang repair --dry-run` prints the planned prompt.
- v2 language track: schemas, `echo fn`/`listen`, `flow` with
  `dep=`, `tune`/`verify`, real v2 interpreter (`run-v2`).
- Robustness: CLI runs on a 256 MiB deep-stack worker; call depth
  capped at 1024 frames with loud `E-RUNTIME` (never a native
  stack overflow).

## Done and pinned by tests (incl. KLANG-FOUNDATION-2)

In dependency order: multi-statement match arms, then match guards,
then closures (by-value capture), then selective multi-file imports
(`import { name } from "file.klang"` alongside the existing whole-file
`import "file.klang"` merge). SPEC §1/§2b/§2c track each item; §8 no
longer lists any of them.

## Done and pinned by tests (incl. KLANG-FOUNDATION-3 Part 1)

- Recursive/nested enum payloads: self- and mutually-recursive payloads
  resolve nominally (no new allocation — values were already
  heap-indirected); mistyped payloads are loud `E-TYPE`; traversal
  honors the 1024-frame cap (`tests/recursive_enum_gates.rs`).
- Tuple variants (`V(i32, str)`) + partial destructuring (`_` ignore,
  nested `E::V(sub...)` with fallthrough, count-exact `E-ARITY`)
  (`tests/tuple_destructure_gates.rs`).

## Done and pinned by tests (incl. KLANG-FOUNDATION-3 Part 2B)

- Local package registry: `klang-registry` service (file-backed index +
  content-addressed archives, bearer-token auth) verified locally end
  to end; `klang publish` / `klang add` / `klang fetch`; SHA-256
  integrity pinned in `klang.lock`; Render deploys from `render.yaml` +
  `docs/deploy-registry.md` (operator-run, no credentials in repo).
  Tested in `tests/registry_gates.rs`.
- Async I/O: network-only simulated async SHIPPED
  (`http_get_async`/`http_post_async` on a 16-worker pool, callable
  only from `async` fns via `E-EFFECT-MISMATCH`; file/stdin/process/
  sleep stay blocking). Tested in `tests/async_net_gates.rs` (incl. a
  wall-clock overlap proof and a slow-host timeout case).
- LSP server v1 (`klang lsp`): stdio JSON-RPC with `Content-Length`
  framing, diagnostics-as-you-type over the exact `klang check`
  pipeline with real line/character positions, whole-file re-check
  with 100 ms debounce. Hover/go-to-definition are planned v2.
  Tested in `tests/lsp_gates.rs` (raw request/response proof).

## Design-only, explicitly not built (SPEC §8 is the binding list)

- True OS-level async, async file/stdin/process I/O, and `cancel`
  enforcement remain future work — see
  `docs/foundation3-part2-design.md` §A.
- Hover/go-to-definition (LSP v2), and debugger/profiler
  (interpreter-only debugger, call-count profiler first; needs its own
  PRD) — see `docs/foundation3-part2-design.md` §§C–D.

## Not here yet (SPEC §8 is the binding list)

No full-value machine-code backend, no real borrow checker (Managed
mode only), no true async I/O runtime (simulated network-only pool),
no LSP hover/go-to-definition (diagnostics-as-you-type v1 only),
no debugger/profiler.

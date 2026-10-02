# Build Roadmap (current state as of KLANG-FOUNDATION-2)

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

## Not here yet (SPEC §8 is the binding list)

No full-value machine-code backend, no real borrow checker (Managed
mode only), no async I/O runtime, no registry/network packages, no
LSP server (only a JSON renderer), no debugger/profiler, no
recursive/nested enum payloads needing indirection, no tuple-variant
syntax or partial destructuring.

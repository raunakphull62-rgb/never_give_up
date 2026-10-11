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
strict, match, duplicate, D3 boundaries via MIR files, and MCP
`klang_check` with `file`) carry the defining file, byte offsets, and
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
- File guard is strict for MCP/agent execution and library calls
  (absolute only under the temp dir, relative with no `..`); plain
  `klang run` additionally allows relative `..` resolved against the
  current directory. Absolute outside temp stays `E-RUNTIME` in both.

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
- **Process/time/env (S6)**: `spawn`/`run` are argv-only (no shell, no
  glob, no timeout, no stdin piping); `spawn` buffers all output in
  memory (no cap — hostile gigabytes OOM like `run_process`); `run`
  inherits stdio. `spawn` is both a task keyword and a builtin
  (`spawn (f())` is task-group spawn, `spawn(cmd, args)` is the
  builtin). `now_ms` exceeds `i32` (flows as Int, `E-OVERFLOW` on
  arithmetic); `now_iso` is wall-clock UTC. `get_env`/`env` absence is
  `""` (no null — unset is indistinguishable from empty). All six are
  interpreter-only (JIT loud-rejects, v2 `E-UNDEFINED`). Missing
  binaries are `E-PROCESS-NOT-FOUND` (other OS refusals are
  `E-PROCESS-FAILED`); bad sleep durations are `E-TIME-INVALID`.
- **Benchmarks**: live-model convergence is measured inside harnesses
  (`bench/run_live.sh` checks the harness signal per task with zero
  model calls); only the offline mock baseline is measured in-tree.

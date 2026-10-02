# Known limitations

Honest list, pulled from the audit's triaged-not-fixed findings.
Nothing here is buried: each item is either a deliberate scope cut
with a workaround or recorded debt with a stated direction.

## F11 — `&&` and `||` do not short-circuit

Both sides always evaluate. A bounds guard in `&&` position
(`i < n && s[i] == "x"` with `i == n`) is well-typed but fails at run
time with `E-RUNTIME` instead of skipping the right-hand side. The
failure is loud, never a silent wrong answer. Workaround: nest the
guard as an `if` inside the loop; `&&` over pure comparisons is
unaffected. The principled fix (short-circuit lowering) would change
observable semantics for side-effecting right-hand sides and is
recorded as future work.

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

## F7 — most diagnostics carry `0,0` spans (file threading fixed by F12 second series)

`primary_span` is `0,0` for nearly every HIR diagnostic (offsets still
unthreaded), but the `file` field now carries the real input path
(`TypedHIR::check_with_file` / `modules::resolve_with_file`; parser
`E-PARSE` always did). So repair-scope attribution is heuristic
(name/use-site based) rather than span-derived. Real spans exist only for `E-PARSE`, `E-TASK-CANCEL`,
`E-SPAWN-OUTSIDE-GROUP`, and `E-EFFECT-MISMATCH` (the caller's name
span). Mitigations in place: full-program re-check after every splice,
loud refusal instead of silent drops, and declaration-level escalation
to file scope. Threading real token spans through HIR checks is the
largest known debt in the repair path.

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

JIT (`--backend-jit`, integer-only) has no depth guard: recursion
compiles to native calls. `countdown(2000)` returns 2000 under JIT
while the interpreter reports `E-RUNTIME` — a known divergence.
Very deep JIT recursion will abort on native overflow, not report
`E-RUNTIME`.

## `run` exit codes wrap at 8 bits (Linux)

`klang run` (and `run-v2`) print only program output; the entry return
value becomes the process exit code and is never printed. A `main`
with no integer return exits 0. Compile/runtime failures print the
structured JSON diagnostic on stderr and exit 1. Exit codes are OS
8-bit values and wrap without clamping: `return 256` exits 0,
`return -1` exits 255. Pinned by `exit_code_wraps_at_256_and_negatives`
(`tests/quiet_gates.rs`).

## Design-scope cuts (not bugs)

- **Ownership**: `Managed` mode only; `Value`/`Owned`/`UnsafeFfi` are
  rejected at the library API (`E-OWNERSHIP-MODE`). There is no surface
  syntax for modes, and `klang check` never emits this code.
- **Contracts**: only the `non-empty` precondition is executable, via
  the library API (`E-CONTRACT`); nothing in surface syntax triggers it.
- **JIT**: integer-only behind `--backend-jit`; the interpreter is the
  reference backend. `build` always stops at the MIR listing.
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
- **Formatter**: `fmt` renders the AST, so comments are dropped from
  its output. Keep commented sources; treat formatted output as a
  canonical snapshot.
- **Salsa DB**: single-file inputs only.
- **Benchmarks**: live-model convergence is measured inside harnesses
  (`bench/run_live.sh` checks the harness signal per task with zero
  model calls); only the offline mock baseline is measured in-tree.

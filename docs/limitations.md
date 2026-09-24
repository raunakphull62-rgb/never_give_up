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

## Design-scope cuts (not bugs)

- **Ownership**: `Managed` mode only; `Value`/`Owned`/`UnsafeFfi` are
  rejected at the library API (`E-OWNERSHIP-MODE`). There is no surface
  syntax for modes, and `klang check` never emits this code.
- **Contracts**: only the `non-empty` precondition is executable, via
  the library API (`E-CONTRACT`); nothing in surface syntax triggers it.
- **JIT**: integer-only behind `--backend-jit`; the interpreter is the
  reference backend. `build` always stops at the MIR listing.
- **Packages**: manifest + FNV lockfile is change-detection, not
  cryptography; there is no registry or network fetching.
- **LSP**: JSON diagnostic renderer only, no server.
  No closures or function values; no trait bounds; no match guards,
  partial destructuring, or struct-style variants; no `throw`
  statement (`throws` is propagation-checked only); no const items;
  arrays and maps are dynamically typed (`unknown` by design).
- **Formatter**: `fmt` renders the AST, so comments are dropped from
  its output. Keep commented sources; treat formatted output as a
  canonical snapshot.
- **Salsa DB**: single-file inputs only.
- **Benchmarks**: live-model convergence is measured inside harnesses
  (`bench/run_live.sh` checks the harness signal per task with zero
  model calls); only the offline mock baseline is measured in-tree.

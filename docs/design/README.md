# Klang Phase 2 design notes — summary (docs only, no implementation)

Status: design notes. Nothing here changes compiler or library behavior;
each item's D-doc holds the problem statement, options with syntax
examples, migration analysis for the 26 published packages, backend
impact, gate-test plan, and effort estimate. Implementation of approved
items comes as separate prompts.

## Per-item recommendations

**D1 — Namespaces / qualified imports** (`D1-namespaces.md`):
recommend file aliases — `import "math/lib.klang" as m` with `m.max(1,
2)` call sites — resolving at load/check time to the existing `Call`
op, while every unqualified import and prefixed stdlib name keeps
merging exactly as today. It reads as a value path (functions are
first-class values, so `m.f` can later be passed to `apply`), unlike
the `m::f` desugar-to-module alternative which overloads the
declaration namespace and inherits `pub`-visibility semantics plain
files never had. Effort M; no runtime work.

**D2 — Real bool, null, and JSON** (`D2-bool-null-json.md`): recommend
real runtime values (`Value::Bool`/`Value::Null`, `print(true)` →
`true`, native JSON `true`/`false`/`null`) behind a permanent
truthiness bridge — `Int` conditions keep working and `Bool`/`Int`
`==` compares truthiness — so all 125 stdlib `== true` sites and every
`if 1` keep their meaning, and the `{"$json": …}` marker maps keep
working dual-read for the published `json` package. A version gate
doubles every rule by edition forever, an always-on warning fires on
all existing code, and a codemod cannot republish immutable packages —
all rejected in §4. Effort L (every builtin coercion/render path plus
JSON dual-read).

**D3 — Integer types** (`D3-integer-types.md`): recommend one 64-bit
`Int` now (the storage is already `i64`; only the range checks move
from 2³¹ to 2⁶³, with range-inferred bare literals and `int(x)` for
narrowing), deferring a small checked `u8` follow-up and declining the
full width lattice (L effort, years of mixed-width edge cases, no
stdlib demand). `len`/`file_size` gain past-2 GiB counts for free;
`ord`/`chr`/`random_int`/`parse_int` keep their signatures. Effort M,
mostly moving boundary tests; `W-TYPE-NARROW` retires for honest
widths.

**D4 — Return-type ergonomics** (`D4-return-type-ergonomics.md`):
recommend making `->` optional with absence desugared to `-> void` at
parse time (`fn main() { … }` becomes the hello-world shape), keeping
explicit annotations required for value functions. A dropped `-> i32`
then fails as a local `E-TYPE` on the `return` line instead of an
`E-PARSE` at the `{`; full inference is rejected as uncheckable
annotations are the point for AI-written code. Keep rejecting
`-> ()` (one `void` spelling; SPEC already documents void-only).
Effort S; no checker, runtime, or backend work.

**D5 — FFI / extern** (`D5-ffi-extern.md`): recommend declared
`extern fn` against a static in-tree host table with deny-by-default
manifest capabilities (`[capabilities]` in the root `klang.toml`,
transitive deps inherit nothing) — no `dylib` path, no `dlopen`, no
raw addresses by construction, so the sandbox is enforceable against
malicious registry packages. WASM plugins are the right answer to a
different question (arbitrary native code) at 10× the surface; staying
builtin-only leaves the core team the bottleneck forever. Effort L
overall, sliceable to an M-sized 3-entry query-only prototype.

**D6 — Backend parity** (`D6-backend-parity.md`): the v1 interpreter is
the reference; recommend a fallible JIT (checked arithmetic via host
calls returning the identical `E-OVERFLOW`/`E-RUNTIME` diagnostics,
eliminating the `sdiv`/`srem` SIGILL class and silent wrapping) with
the differential corpus harness landed *first* — every
`tests/parity/*.klang` run under `run`, `run --backend-jit`, and
`run-v2` comparing value, stdout, and exit codes, so the fix is proven
by tests that already exist. The audit adds the missing rows: the
JIT's absent depth guard, v2's loud feature gaps (no closures,
`try`/`catch`, stdin, float literals), and byte-vs-char string rules.
Effort M; no language or interpreter work.

## Suggested implementation order, with reasons

1. **D6 harness first (S slice).** The corpus runner passes on day one
   by encoding today's behavior (divergent cases allow-listed), costs
   almost nothing, and becomes the acceptance gate for D2 and D3 —
   without it, real `bool` rendering and 64-bit boundaries would land
   with no cross-backend proof. It unblocks everything below.
2. **D4 (S).** Pure parse-time sugar, zero backend interaction, and it
   makes every subsequent example and test programs shorter to write.
   Nothing depends on it, but nothing is risked by it either.
3. **D1 (M).** Load-time name resolution only; no MIR/runtime change,
   so it composes with all other items. Doing it before D5 matters:
   D5's `extern` declarations and capability tables should be designed
   alias-aware from the start rather than retrofitted.
4. **D2 (L) then D3 (M), or in parallel.** D2 touches every value path
   (render, truthy, equality, JSON) and D3 moves the overflow boundary
   both backends check — each redefines what "agreement" means, so
   both land *against* the D6 corpus (flipping allow-list entries to
   agreement assertions in the same PRs). D2 first because its
   `Value::Bool` representation decision constrains D3's conversion
   rules, not the reverse.
5. **D6 fallible JIT (M) with D3.** The checked-arithmetic host calls
   take the boundary as a parameter, so implementing them together with
   the 64-bit move avoids breaking parity twice. Hard requirement:
   never declare D3 done until the D6 gates cover the new boundary on
   both backends.
6. **D5 last (L, or M prototype).** It is the only item that widens the
   trust boundary (new syscalls reachable from AI-written code), so it
   should inherit the finished parity harness, the finished value
   model (D2), and the finished width model (D3) — and its per-entry
   JIT intrinsics depend on D6's error channel existing first. A
   query-only prototype can start earlier for shape validation, but
   registry/index surfacing waits until the language core settles.

Most breaking first is a deliberate anti-goal here: D2/D3 change value
semantics, so they go after the harness that proves they did not break
the 26 packages; D5 changes the security posture, so it goes last when
the machinery that audits it (corpus gates, `klang caps` listing) is
real.

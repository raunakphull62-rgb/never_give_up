# D6. Backend parity: making the JIT and the interpreter agree

Status: design note — no implementation. (Phase 2; see the Phase 2 report
for the recommendation summary and ordering.)

## 1. Problem

Klang has two backends with one spec (the interpreter's behavior): the
default tree-walking interpreter over MIR (`src/runtime/mod.rs`) and the
int-only Cranelift JIT behind `--backend-jit` (`src/jit.rs`). On the
happy path they agree (pinned by the `diff` helpers in
`tests/jit_int.rs`). Off it, they diverge — probed live 2026-10-07 on
this checkout (`klang 0.7.0`):

| Program | Interpreter | JIT (`--backend-jit`) |
|---|---|---|
| `print(2147483647 + 1)` | `E-OVERFLOW`, exit 1 | prints `2147483648`, exit 0 (**silent wrap**, value outside the language's int range) |
| `print(0 - x)` with `x = 0 - 2147483647 - 1` | `E-OVERFLOW` in `sub`, exit 1 | prints `2147483648`, exit 0 (silent wrap) |
| `print(1 / 0)` | `E-RUNTIME` "division by zero", exit 1 | **SIGILL, core dumped, exit 132** (raw `sdiv` trap — a crash, not an error) |
| `print(1 % 0)` | `E-RUNTIME` "modulo by zero", exit 1 | same trap class (`srem`; `src/jit.rs:427-431` documents Div/Mod traps — as established behavior, not as errors) |
| `print(-x)` with `x = i32::MIN` | `E-OVERFLOW` in `neg` | wraps (same `ineg` class, unverified live but identical code shape) |
| float/str/list code | runs | loud `JIT-FAIL` naming the op (**parity by rejection** — fine) |

Root causes, all in `src/jit.rs`: `arith_v!` emits raw `iadd`/`isub`/
`imul`/`ineg` with no overflow check (`:424-479`), while the interpreter
routes through `to_i32_checked` + `checked_*` (`src/runtime/mod.rs:212-
234,806-872); `sdiv`/`srem` trap on zero instead of returning a
diagnostic (`:427-431`); the JIT signature (`fn(i64…) -> i64`,
`run_jit` at `:100`) has **no error channel at all**, so even a
detected divergence could not be reported as `E-…` today.

Confirmed *agreeing* (do not regress): int comparisons/`&&`/`||`/`!`
(truthiness `!= 0` both sides, incl. Phase 1a short-circuit —
`tests/shortcircuit_gates.rs` diffs), `if`/`while` control flow,
`Copy`/`Const(int)`, user-fn `Call` (missing-args-0/extras-ignored
mirrors the interpreter's zip), `Print` of ints, fall-off-the-end `0`,
`EnterGroup`/`LeaveGroup` no-ops, entry-arity checks (loud on both).

The v2 tree-walker (`run-v2`, `src/runtime/v2.rs`) vs the v1
interpreter — closer, but not identical:

| Area | v1 interpreter | v2 tree-walker |
|---|---|---|
| `1 / 0`, `1 % 0` | `E-RUNTIME`, exit 1 | same `E-RUNTIME` (`v2.rs:642,650`) — agrees |
| i32 overflow (`MAX+1`, `MIN/-1`, `abs(MIN)`, neg) | loud `E-OVERFLOW` | same loud `E-OVERFLOW` (own `to_i32_checked`, `v2.rs:47`, + return-range check `:431-440`) — agrees |
| Call-depth cap | 1024 frames, then `E-RUNTIME` | same 1024 cap (`v2.rs:449-450`) — agrees |
| Closures | by-value capture, runs | **no closure literals at all** (SPEC §2c; `E-PARSE`/`E-UNDEFINED` at the boundary — loud, but a program using closures runs on v1 and is rejected on v2) |
| `try`/`catch` | runs | **no `try`/`catch`** (reference.md §3; loud reject) |
| `read_line` / stdin | runs | **`E-UNDEFINED`** (reference.md §5; v2 has no stdin builtins) |
| Floats | IEEE `f64`, runs | values supported at runtime (`v2.rs:115`), but **no float literals** (reference.md §5), so float programs cannot be spelled in v2 source |
| `args()` / `exit()` | runs (`args` real argv) | open probe: BUILTINS.md pins `[]` only for JIT/MCP paths — the corpus gate (§6) must record v2's actual behavior and pin agreement-or-loud-reject per builtin |

More known/likely divergences to pin (JIT vs interpreter):

- **Call-depth guard**: the JIT has none — `countdown(2000)` returns
  2000 under `--backend-jit` while the interpreter reports `E-RUNTIME`
  (docs/limitations.md). Very deep JIT recursion aborts natively
  instead of reporting. The fallible-JIT work (Option A) should add a
  depth counter to compiled calls, or the corpus allow-list must record
  "JIT depth >1024" as accepted-divergent with a subprocess harness
  (never in-process).
- **Float edge cases**: the interpreter is full IEEE `f64`
  (`total_cmp` sorts, NaN-tolerant `min`/`max`, loud `sqrt(-1)` /
  `log(<=0)` / overflowing `pow` per BUILTINS.md). The JIT rejects
  `ConstFloat` loudly — that *is* parity for now, but any future
  float JIT support must reproduce the loud-error edges (never quiet
  NaN/inf where the interpreter errors), diffed case by case.
- **String handling**: same shape — `len()` is byte length while
  `s[i]`/`chars()` are char-based (SPEC §4). JIT rejects `ConstStr`
  loudly today; future string support must pin byte-vs-char semantics
  in the corpus (`len("é") == 2`, `s[1]` char indexing) before any
  string op compiles.
- **Exit codes**: `return 256` exits 0, `return -1` exits 255 (8-bit
  wrap, limitations.md) — the corpus compares process exit codes, not
  just stdout, on every backend so a backend that clamps instead of
  wrapping trips the gate.

## 2. Options

### Option A — fallible JIT: checked arithmetic via host calls (recommended; see §3)

```text
$ klang run --backend-jit ovf.klang
run: FAIL
{ "code": "E-OVERFLOW", "message": "integer overflow in `add`: result out of i32 range", … }
exit: 1
```

Give the JIT an error channel and checked ops: compiled code calls back
into host helpers (`klang_checked_add`, `klang_checked_div`, …) that
return `(value, error_code)`; on nonzero error the JIT stub exits
through the same `Err(Diagnostic)` path as `run_jit`'s existing
unsupported-op errors, with the *identical* code/message/cause/fixes as
the interpreter (`E-OVERFLOW`, `E-RUNTIME` division/modulo-by-zero).
`Div`/`Mod`-by-zero and `Add`/`Sub`/`Mul`/`Neg` overflow become loud
diagnostics; the SIGILL class disappears. Opcodes stay the same MIR —
only lowering of five arithmetic ops changes (plus the stub epilogue).

Pros: the JIT becomes correct on its subset with the identical
code/message/cause/fixes as the interpreter — safe to run on
untrusted input, which today it is not. Cons: error-slot plumbing plus
five lowering rewrites; checked host calls cost some speed vs raw ops
(measure in the PR; correctness first).

### Option B — shrink the JIT's accepted subset to what it computes exactly

```text
$ klang run --backend-jit add.klang   # fn add(a: i32, b: i32) -> i32
run: JIT-FAIL
{ "code": "JIT-FAIL", "message": "unsupported op Add in 'main'", … }
exit: 1
```

Reject (loud `JIT-FAIL`, as today for strings) any function containing
`Div`/`Mod`/`Add`/`Sub`/`Mul`/`Neg` … i.e. all arithmetic — which
reduces the JIT to moves, comparisons, and control flow: honest, but a
JIT that cannot add is a demo, not a backend. Alternatively reject only
*statically provable* danger (`x / 0` literal) — worthless, since all
real overflows are dynamic. Documented here as the honest retreat if A
proves too costly.

Pros: honest, S effort (deletions + docs). Cons: surrenders the
backend's purpose; literal-only rejection catches none of the real
dynamic failures.

### Option C — differential testing only: pin agreement where it holds, document the rest

```text
// tests/jit_parity_gates.rs (sketch)
for prog in corpus("tests/parity/*.klang") {
    let (v, out) = interp(prog);
    match jit(prog) {
        Ok((jv, jout)) => assert_eq!((v, out), (jv, jout)),
        Err(e) => assert!(ACCEPTED_REJECTS.contains(e.op), "{e}"),
    }
}
```

No backend change: build a corpus runner that executes every int-only
program on both backends and asserts identical value+output, plus an
explicit allow-list of currently-divergent programs (overflow/div-zero)
that assert *the documented divergence* (wrap value / trap) so any
change — fix or regression — trips the gate. Makes the status quo
legible and the fix verifiable, but ships the SIGILL and the silent
wrap indefinitely.

Pros: S effort; legible status quo; the fix later is proven by tests
that already exist. Cons: alone, it normalizes incorrectness — the
crash and the silent wrap ship indefinitely.

## 3. Recommendation and why

**Option A, with C's harness as its acceptance gate (C first, then A).**

Why: the reference backend must be the **interpreter** — it implements
the whole language, every error code, and every published package runs
on it. A JIT that wraps where the language errors, and core-dumps where
the language reports, is not "narrower but correct"; it is incorrect on
its own subset. Option C alone normalizes that. The sequencing matters:
land C's corpus runner *first* (it passes on day one by encoding today's
behavior, divergent cases included), then implement A against it — the
divergent allow-list entries flip to agreement assertions in the same
PR that adds checked arithmetic, so the fix is proven by tests that
already exist. Option B surrenders the backend's purpose.

Concretely: (1) `run_jit` gains an out-param error slot (in code:
compiled functions return `(error_code, value)` or write a thread-local
error slot the stub reads; host helpers mirror
`to_i32_checked`/`checked_*`/zero-checks exactly, including messages);
(2) the five arithmetic lowerings call the helpers; (3) the corpus
gates flip. `Div`/`Mod`-by-zero and all integer overflows then surface
the same JSON the interpreter surfaces, and `--backend-jit` becomes
safe to run on untrusted input — today it is not (SIGILL).

## 4. Migration impact

- The 26 published packages: **no edits required**. Packages run on the
  interpreter by default; JIT behavior only changes from
  wrong/crashing to correct on programs the interpreter already
  rejects-or-accepts identically. Any package test run under
  `--backend-jit` that asserted wrapped values (none in-tree — grep
  finds no such assertion) would need updating to the loud error.
- Existing programs: programs that run cleanly under both backends are
  unaffected (agreement is the invariant). Programs that *relied* on
  JIT wrapping (computed garbage past 2³¹ without error) now fail
  loudly under JIT exactly as they always did under the interpreter —
  that reliance was a bug against the spec (`E-OVERFLOW` is specified).
- The `Div`/`Mod`-by-zero trap note in `src/jit.rs:65-70` is deleted
  and replaced by the error-channel contract; `docs/limitations.md`
  gains (then loses, when A lands) a parity section.

## 5. JIT / interpreter / v2 impact

- Interpreter and v2: **zero changes** (v1 is the reference; v2 must
  match v1 value-for-value on their shared subset — C's harness
  double-locks both behaviors as a side effect by running the corpus
  under `run` and `run-v2`).
- JIT: the only backend touched. Error-slot plumbing in `run_jit` +
  stub epilogue; five op lowerings switch to checked host calls
  (small perf cost vs raw ops — measure in the PR; correctness first,
  and the int-only JIT is not yet the performance story); Trap paths
  (`sdiv`/`srem` zero) eliminated, not caught — code never traps.
  Unsupported-op rejections (`ConstStr`, builtins, …) are untouched:
  rejection *is* parity for out-of-subset code.
- D2/D3 interplay: D3's 64-bit `Int` moves the checked boundary to
  2⁶³ — the new host helpers take the boundary as a parameter (or two
  entry points), so D3 does not re-break parity; D2's `Bool` stays
  `I64` 0/1 inside JIT code with rendering handled per D2 §5.

## 6. Test plan

- **Corpus runner first** (`tests/jit_parity_gates.rs`): every
  `tests/parity/*.klang` runs on **all three executors** (`run`,
  `run --backend-jit`, `run-v2`); agreement asserted on
  value + printed output + exit code + (where applicable) rejection code. Seed the
  corpus with: all current `jit_int.rs`/`shortcircuit_gates.rs` diff
  programs (fold them in or share the helper), boundary arithmetic
  (`MAX+1`, `MIN-1`, `MIN/-1`, `abs(MIN)`, neg), div/mod-by-zero,
  `if`/`while` logic nests, multi-arg calls, prints, **plus the v2
  feature matrix** (closure use → v1-runs/v2-loud-reject;
  `try`/`catch`, `read_line`, float literals likewise) and the
  **depth probes** (`countdown(1000)` runs everywhere,
  `countdown(2000)` is `E-RUNTIME` on both interpreters).
- **Divergence allow-list** (interim, deleted by the fix): the five
  overflow/trap programs assert today's exact behavior (wrap values,
  SIGILL class — assert via subprocess exit code, never in-process, so
  a trap cannot take down the harness; reuse the `klang_bin()` pattern
  from `registry_fixes_gates.rs`).
- **Fix PR flips the list**: the same corpus asserts identical loud
  `E-OVERFLOW`/`E-RUNTIME` JSON on both backends; SIGILL-class tests
  assert clean `run: FAIL` + exit 1. New invariant gate: "no test in
  the repo executes `--backend-jit` on a program whose backends
  disagree" (the runner enforces it structurally).
- Fuzz (follow-up, stated not promised): random int-expression
  generator diffed across backends in CI — catches the next wrap-class
  bug; file under this D-item when A lands.

## 7. Effort estimate (S / M / L) and risks

**M** (harness + corpus first: S; error-slot plumbing + five checked
lowerings + flipping the gates: M). No language, MIR, or interpreter
work. Option B would be S (deletions + docs) and Option C alone S —
both leave the crash in place, which is why neither is recommended.

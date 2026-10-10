# D2. A real `bool` type, a `null` literal, and JSON values after that

Status: design note — no implementation. (Phase 2; see the Phase 2 report
for the recommendation summary and ordering.)

## 1. Problem

Klang has half a boolean. The checker knows `Ty::Bool`: `true`/`false`
literals, comparisons, and `&&`/`||`/`!` all check as `Bool`, and
returning one from `-> i32` is `E-TYPE` (`src/hir.rs:1698,2416,2470,2508`).
But the runtime erases it: `Expr::Bool` lowers to `Const 0/1`
(`src/mir/mod.rs:401-412`), `Value` has no `Bool` variant
(`src/runtime/mod.rs:58-76`), and `print(true)` prints `1`. There is no
`null` literal at all (`print(null)` is `E-UNDEFINED`).

Consequences:

- JSON `true`/`false`/`null` cannot be runtime values, so the `json`
  package smuggles them as `{"$json": "true"/"false"/"null"}` marker maps
  (`stdlib-packages/json/src/parse.klang:10-16`, `:552-640`) — with a
  documented collision (`{"$json": "null"}` written by a user parses as
  null, pinned by `json_test.klang`).
- Int-as-bool is load-bearing in user code: **125** `== true` / `== false`
  comparisons across the 28 stdlib `.klang` files that use them (e.g.
  `assert(d["ok"] == true)` where both sides are `Int` 1 today), plus
  `== 1` / `== 0` flag checks (`cli_get_flag(r, "verbose") == 1`).
  Any real `bool` that breaks `1 == true` breaks the published packages.
- `if`/`while`/`assert` accept `Bool` *and* `Int`
  (`require_condition`, `src/hir.rs:1303-1313`), so `if 1` is legal and
  must stay legal unless every existing program is rewritten.

## 2. Options

### Option A — full strictness: runtime `Bool`/`Null`, `Bool`-only conditions

```text
let b: bool = true          // Value::Bool(true)
let n = null                // Value::Null
if b { print("yes") }       // OK
if 1 { print("yes") }       // E-TYPE: want bool, got i32
assert(d["ok"] == true)     // E-TYPE unless d["ok"] is bool
```

New `Value::Bool(bool)` and `Value::Null` variants; literals lower to
them; comparisons/`&&`/`||`/`!` produce them; `require_condition`
accepts only `Bool` (and `Unknown`); `==` is strict (`Bool` vs `Int` is
`E-TYPE`); JSON parses `true`/`false`/`null` to native values and the
`$json` markers are retired (kept readable for one major version, then
removed). Rendering: `print(true)` → `true`, `print(null)` → `null`.

Pros: cleanest semantics; no bridge rule to maintain. Cons: flag-day
breakage — 125 stdlib sites plus every `if 1` fail at once, needing a
codemod over 26 immutable, unrepublishable packages. Unimplementable
without breaking the registry contract.

### Option B — real values with a permanent truthiness bridge (recommended; see §3)

```text
let b = true                // Value::Bool(true)
let n = null                // Value::Null
if b { print("yes") }       // OK
if 1 { print("yes") }       // still OK: Int conditions keep working
assert(d["ok"] == true)     // OK: Bool(1-ish) == Int(1) by truthiness
assert(json_is_true(v))     // OK: dual-read, native or marker map
print(true)                 // now prints "true" (was "1")
```

New `Value::Bool`/`Value::Null` and a `null` literal, but:

- Conditions (`if`/`while`/`assert`/`!`/`&&`/`||` operands) keep
  accepting `Int` — truthiness is unchanged, so every existing branch
  behaves identically.
- `==`/`!=` between `Bool` and `Int` compares **truthiness**
  (`true == 1`, `false == 0`, `true == 2` is *false* — only 0/1 match),
  so all 125 stdlib comparisons keep their values. Same-type and
  numeric-mix rules are untouched; `Bool` vs `Str`/`Array`/… stays
  `E-TYPE`.
- JSON is dual-read, dual-write-native: `json_parse("true")` yields
  `Bool(true)`; `json_is_true`/`json_is_false`/`json_is_null` accept a
  native value **or** the old marker map; `json_type` reports
  `"bool"`/`"null"` for both; `json_pretty`/encoders emit native
  `true`/`false`/`null`. Marker maps keep round-tripping, so old and
  new code interoperate.
- `null` flows as a value: `json_get(v, "missing")` can return real
  `Null` (today: check what it returns — keep the old behavior unless
  the json major bumps; see §4).

Pros: real values with zero package edits and no flag day; kills the
marker-collision class for new data. Cons: the truthiness bridge in
`==` is permanent specified behavior to maintain; printed bools change
rendering (`1`/`0` → `true`/`false`), which snapshot-style user
programs will notice.

### Option C — no runtime change: `null` desugars to the marker map

```text
let n = null                // lowers to {"$json": "null"} (a Map)
print(null)                 // prints {"$json": "null"}
```

The parser gains a `null` literal that lowers to today's marker map;
`bool` stays erased to `Int`. The checker adds `W-…`-style guidance
(`== true` on an `Int` stays legal). Zero runtime, MIR, JIT, or JSON
changes; the marker collision and the `print(true)` → `1` oddity live
forever.

Pros: zero runtime, MIR, JIT, or JSON work. Cons: cements the hack —
`null` printing as a map is permanent, and the checker/runtime split
("types that lie") it preserves is the original complaint.

## 3. Recommendation and why

**Option B.**

Why: it delivers real values (killing the marker-collision class for
*new* data while keeping every old program green) without a flag day.
Option A is cleaner in the abstract but breaks all 26 packages on day
one (`== true` becomes `E-TYPE` across 125 sites) and forces every
`if 1` in existence through a migration tool. Option C cements the
hack — `null` printing as a map would be a permanent embarrassment, and
the checker/runtime split it preserves is exactly the "types that lie"
complaint (1d). B's only real cost is the permanent truthiness bridge
in `==`, which is a small, well-tested rule — and it matches what the
language already teaches (`if 1` is legal, so `1` is already
truthy; `==` just agrees).

## 4. Migration impact

- The 26 published packages: **no edits required under B**. All 125
  `== true/false` sites keep their values via the truthiness bridge
  (`Int(1) == Bool(true)` → true; `Int(0) == Bool(false)` → true);
  `== 1`/`== 0` flag checks are same-type as before; `if`/`while` on
  `Int` still pass; marker maps still satisfy `json_is_*`/`json_type`.
  Verified by construction (bridge rule) and by re-running all 26
  `*_test.klang` — any change in output fails the gate.
- Known output changes (user-visible, not package-breaking): `print(b)`
  / `str(b)` for a `bool` now render `true`/`false` instead of `1`/`0`.
  Repo audit found **no** stdlib test asserting the old rendering, but
  user programs that snapshot printed bools will see diffs — call it
  out in the release notes with the one-line fix (`str()` → explicit
  `b == true` formatting where `1`/`0` output is wanted).
- Cross-version hazard (immutable registry!): a *new* program passing a
  native `Bool` into an *old* vendored `json` lib gets `json_type →
  "int"` (old code probes `$json`/`join`/`keys`/`upper`, all absent on
  `Bool`, then falls to the `str(v).contains(".")` heuristic —
  `stdlib-packages/json/src/parse.klang:594-640`). Mitigation, in order:
  (1) dual-read is in the *new* lib, so upgrading `json` fixes every
  consumer at once; (2) document "bump `json` before passing native
  bools across package boundaries"; (3) never remove marker *writing*
  from encoders until the old majors age out (encoders may keep an
  opt-in marker mode for mixed graphs).
- `null` as a new literal: anyone binding a variable literally named
  `null` breaks loudly at parse (`expected …`, never a silent rebind —
  repo-wide grep finds no such binder in stdlib/tests/examples, but user
  code must be told). `json_get` on a missing key: keep today's
  behavior in the same major; real `Null` returns arrive with the `json`
  major bump only.
- Existing programs: everything that checks today checks tomorrow
  (bridge + still-accepted `Int` conditions). The only programs that
  *fail* are ones that were already failing.
- Why not the other migration shapes named in the item scope:
  (1) a **language version gate** (`edition = "2"` in `klang.toml`
  switching `1 == true` from truthy-equal to strict) doubles every
  rule above by edition — two truthinesses, two renderings, and every
  vendored package parsed under *its own* edition while the root
  program runs under another. With immutable published packages, the
  old edition lives forever; the gate buys a flag day nobody can
  schedule. Rejected.
  (2) **implicit conversion with a warning** (bridge rule, but each
  `Bool`/`Int` comparison or `Int` condition warns) fires on all 125
  stdlib sites and on every `if 1` in existence — a warning that is
  always on is either ignored or `--deny-warnings` breaks the world.
  The bridge comparison is specified behavior, not a smell, so warning
  is the wrong signal. Rejected (warnings are reserved for genuinely
  narrowing cases like today's `W-TYPE-NARROW`).
  (3) a **migration tool** (`klang migrate-bool` rewriting `== true`
  to explicit `!= 0` and `if 1` to `if true`) rewrites 26 immutable
  packages it cannot republish and user code it cannot test — and
  under Option B there is nothing to rewrite *to*, since old code
  keeps its meaning. A codemod is only needed if Option A is chosen;
  under B, at most document the one-line rendering fix. Not needed.

## 5. JIT / interpreter / v2 impact

- Interpreter: add `Value::Bool`/`Value::Null` arms to `truthy`,
  `render`, equality, `as_int`/`as_float` (define: `Bool` → 1/0,
  `Null` → 0/0.0 — same leniency as today), every builtin coercion, and
  `exit_code_for_value` (`Bool(true)` → 1, preserving today's exit
  codes since `Bool` erases to `Int` 1 today). `And`/`Or`/`Not`/
  comparisons construct `Bool`. Short-circuit lowering (1a) is
  value-agnostic — unchanged.
- JIT: the int-only backend must decide per op. Minimal consistent
  story: keep representing truth values as `I64` 0/1 inside JIT-compiled
  code (literals `true`→1, `false`→0, comparisons already yield 0/1),
  and **reject** `Null`-producing ops loudly (`ConstNull` →
  unsupported, like `ConstStr`) until a boxed-value backend exists.
  Cross-backend rule (feeds D6): any program the JIT accepts must print
  and return exactly what the interpreter prints and returns —
  including the new `true`/`false` renderings, which forces the JIT's
  `Print` path to render 0/1-ints-as-bools correctly or reject bool
  prints.   `Null` anywhere in JIT-reachable code is a loud reject, never
  a miscompile.
- v2 tree-walker: same new variants and the same `truthy`/`render`/
  equality rules as v1 (both runtimes share the `Value` semantics, so
  the bridge matrix in §6 runs against all three executors in the D6
  corpus). If v2 gains float-literal or closure support later, `Bool`
  follows the same arms — no separate v2 design needed.
- No new MIR ops are strictly needed (reuse `Const` for bools *or* add
  `ConstBool`/`ConstNull` for debuggability — prefer explicit ops: the
  JIT's unsupported-op error then names `ConstNull` instead of silently
  treating it as an int).

## 6. Test plan

- Checker: `true`/`false` literals are `Bool`; comparisons/`&&`/`||`/`!`
  yield `Bool`; `Bool` return into `-> i32` is `E-TYPE`; `null` literal
  checks as `Null`; `if 1` still clean; `1 == true` clean (bridge).
- Runtime: `print(true/false/null)` → `true`/`false`/`null`;
  `1 == true`, `0 == false`, `2 == true` is false; `{"ok": true}` map
  round-trips; `json_type`/`json_is_*` accept native **and** marker
  forms (matrix test over both representations × all four predicates).
- Markers: `{"$json": "null"}` still parses as null; `{"$json": 42}`
  still an object (existing pinned cases keep passing).
- Back-compat: all 26 `*_test.klang` byte-identical output; plus a new
  `tests/bool_null_gates.rs` with the bridge matrix and a
  mixed-version test (native bool value fed to the *old* marker-based
  predicates vendored in-tree — documents the §4 hazard).
- `null`-as-identifier: `let null = 1` is a loud parse error, pinned.
- JIT differential (with D6): every accepted bool program compared
  interpreter-vs-JIT; `Null`-reaching programs assert loud JIT rejects.

## 7. Effort estimate (S / M / L) and risks

**L** (runtime value variants touch every builtin/coercion/render path;
JSON dual-read + encoder modes; bridge-rule tests; release-note audit of
printed-bool snapshots). The checker half alone would be S — the runtime
and JSON compat are the work.

## 8. Decision (Wave1 S4, owner-approved, implemented)

**Neither A, B, nor C is implemented in this wave. What is implemented
is a strict-mode opt-in that changes no default behavior at all** —
the recommended Option B (real `Value::Bool`/`Value::Null` with the
truthiness bridge) stays future work, and this decision constrains
nothing about it.

Default mode is byte-for-byte unchanged: `Bool` and `Int` conditions
(`if`/`while`, `&&`/`||`/`!` operands) all check clean and run with
today's truthiness. Strict mode is selected per invocation or per
project:

```text
klang run --strict prog.klang
klang check --strict prog.klang
```

```text
# klang.toml
[project]
strict = true
```

In strict mode, a statically-known non-`Bool` value used as a
condition in `if`/`while` or with `&&`, `||`, `!` is `E-TYPE` naming
the offending type (e.g. ``if condition: want bool, got i32``).
`--strict`/`--strict=true` enables, `--strict=false` explicitly
disables; without a flag, `[project] strict` decides (absent key or
absent file means default). An explicit flag beats the manifest either
way. The flag is stripped before positional parsing and never acts as
a file, entry, or program argument name — anything after `--` stays
program argv untouched — and it overrides nothing else
(registry/verbose/offline are independent). `build` honors the same
flag through its shared check stage; `run-v2`/`check-v2`, LSP, and MCP
stay default-mode (the v2 track and the editor/tool surfaces have no
strict switch in this wave).

Compile-time vs run-time (the choice the task leaves to the
implementation): **strictness is compile-time only, over statically
known types.** `Bool` and `Unknown` pass; every other static type —
including `Int` — fails. `Unknown` (map lookups, dynamic index,
generic positions) stays allowed by necessity, not by leniency: the
runtime erases `Bool` to `Int` 0/1, so no runtime rule could
distinguish a dynamic `true` from a dynamic `1`, and rejecting all
dynamic conditions would outlaw working programs. A strict violation
therefore surfaces at the check stage, identically for `check
--strict` (exit 1, `check: FAIL`) and `run --strict` (fails before
executing a line). There is no runtime truthiness change in either
mode.

Scope boundary (deliberate): only `if`/`while` conditions and
`&&`/`||`/`!` operands are strict-gated. `assert()`, match guards,
and `for`-range bounds keep the default `Bool`/`Int` rule in both
modes — they are not conditions in the decided sense, and widening
the gate would change diagnostics the task did not ask to change.

Gates: `tests/strict_mode_gates.rs` covers default vs strict for each
of the five positions (`if`, `while`, `&&`, `||`, `!`); `Bool` accepted
and `Unknown` allowed in strict mode; the flag via CLI (before/after
the file, `run` and `check`); the manifest key (true/false/absent,
wrong section ignored); flag-beats-manifest precedence in both
directions; and flag inertness (a `Bool` program prints and exits
identically with and without `--strict`; `--strict` after `--` is
argv).

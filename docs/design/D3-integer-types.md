# D3. More integer types (at least `i64`; consider `u8` and unsigned)

Status: design note — no implementation. (Phase 2; see the Phase 2 report
for the recommendation summary and ordering.)

## 1. Problem

Klang spells more integers than it implements. `i64`, `u32`, `u64` parse
to `Ty::Int` (`src/hir.rs:80-89`) but every arithmetic op funnels through
`to_i32_checked` and fails `E-OVERFLOW` past 2147483647
(`src/runtime/mod.rs:212-234,806-872`) — `fn f(x: i64) -> i64` with
`f(1) + 2147483647` still overflows (Phase 0 probe). `u8` and friends
parse as identifiers and resolve to `Unknown`: unchecked, dynamic, no
error anywhere. Phase 1d hung a `W-TYPE-NARROW` lantern on both lies
without changing behavior. Real asks underneath: file sizes past 2 GiB
(`file_size` narrows `u64 as i64`, `src/runtime/mod.rs:1811`), counts
past 2³¹ (`len()` returns `Int`), timestamps, and honest unsigned bytes
for codec work (`base64`/`hex` packages hand-roll masking today).

## 2. Options

### Option A — full widths: distinct types, suffixes, widening, per-type overflow

```text
let big: i64 = 5000000000          // literal out of i32 range: OK as i64
let b: u8 = 255
let w: i64 = b                     // widening u8 -> i32/i64: implicit
let n: i32 = big                   // E-TYPE: narrowing needs int(big)
print(255u8 + 1)                   // E-OVERFLOW for u8 (max 255)
```

Four checked types (`i32`, `i64`, `u32`/`u64`, `u8` — plus whatever
unsigned set is chosen), literal rules (plain digits default `i32`;
`…u8` / `…i64` suffixes or range-based inference — pick suffixes,
explicit beats clever), implicit widening only (`u8`→`i32`→`i64`,
`u32`→`u64`; signed/unsigned mixes and all narrowings need
`int(x)`/`i64(x)`/… conversions), per-type `E-OVERFLOW`
(`u8` 0..255, `u32` 0..4294967295), builtins gain width signatures
(`len()` → `i64`? or stays `i32`? — must be answered per builtin).

Pros: honest types with full width safety. Cons: L effort —
multiplies every builtin signature, conversion rule, and JIT op by the
width lattice, with years of mixed-width edge cases (`u32` division,
`%` across widths, cross-width `min`/`max`) for demand the stdlib does
not show.

### Option B — one 64-bit `Int` (recommended; see §3)

```text
let big: i64 = 5000000000          // OK: Int holds i64 range now
let n: i32 = 42                    // i32 stays a spelling; checks 32-bit range on conversion
print(len(huge_string))            // counts past 2^31 now representable
print(file_size(bigfile))          // u64 metadata that fits i64 now exact
```

`Value::Int` keeps its `i64` storage but the *checked range* becomes
`i64::MIN..=i64::MAX`: delete `to_i32_checked` in favor of native
`checked_*` (overflow still loud `E-OVERFLOW`, just at 64 bits).
`i32` remains a valid annotation meaning "must fit 32 bits" (enforced at
conversion/`return`/call boundaries — today's finest behavior, kept).
`u32`/`u64` become aliases of the 64-bit `Int` for values that fit
(negative literals still rejected for unsigned spellings at check time;
values past `i64::MAX` stay unrepresentable and documented).
`u8` stays out of this step (see below).

Pros: deletes the lie in M — storage is already `i64`, no program
changes value, `file_size`/`len` past 2 GiB fixed, and 90% of
`W-TYPE-NARROW` noise retires. Cons: `u32`/`u64` past `i64::MAX` stay
unrepresentable; every `E-OVERFLOW` boundary test must move to the new
limit in the same PR.

### Option C — no new types: big integers as a package

```text
import "bigint/lib.klang" as big
let x = big.from_str("5000000000")
print(big.add(x, big.from_int(1)))   // "5000000001"
```

Arbitrary precision lives in `stdlib-packages/bigint` (string- or
limb-backed, pure Klang or a blessed builtin table). The core language
gains nothing; `W-TYPE-NARROW` stays as the permanent answer for
`i64`/`u32`/`u64`.

Pros: no core change at all. Cons: every package keeps spelling `i64`
and warning; a bigint package is itself M work that solves nothing in
the core.

## 3. Recommendation and why

**Option B now; `u8` as a small checked follow-up, not bundled.**

Why: B deletes a lie (the storage is *already* `i64` — the restriction
is three lines of range checks) while changing no program's meaning:
every value representable today behaves identically, and the only newly
accepted programs are ones that fail loudly today. It fixes the
`file_size`/`len` past-2 GiB hole for every realistic input, aligns the
interpreter's storage with the JIT's registers (both 64-bit — the
remaining JIT gap is *checked vs wrapping*, which is D6's work, not
D3's), and retires 90% of `W-TYPE-NARROW` noise (`i64` becomes honest;
`u32`/`u64` become honest-for-fitting-values). Option A is the
textbook answer but multiplies every builtin signature, every
conversion rule, and every JIT op by the width lattice — L effort with
years of edge cases (`u32` division semantics, `%` on mixed widths,
`min`/`max` across widths) for demand the stdlib does not show. Option
C punts: every package would still spell `i64` and warn.
`u8` follow-up (separately, S/M): a *checked* `u8` (0..255,
construction via `u8(x)` with `E-OVERFLOW` outside range, widening to
`Int` implicit) covers the codec byte use-case without reopening the
width lattice.

Literal rules under B: plain digits keep today's rule (fit `i32`, else
`E-TYPE`… or infer `i64` when out of `i32` range? — **decision:** infer:
a bare `5000000000` in an `i64`/`Unknown` position becomes `i64`;
in an `i32` position it stays `E-TYPE`. Range-based inference with an
explicit annotation fallback; no suffixes in step B). Widening: `i32`
→ `i64` positions implicit (it is already `Int`); `i64` → `i32`
positions need `int(x)` (loud `E-OVERFLOW` outside range — the same
check `return` does today).

Builtin signatures under B (all keep returning `Int`; only the
representable range grows): `len()` (bytes/elements/entries) and
`file_size()` are pure beneficiaries — counts past 2³¹ become
representable instead of failing at the return-value check.
`ord()`/`chr()` are unaffected in practice (codepoints fit `i32` by
Unicode definition; `chr` already rejects out-of-scalar inputs with
`E-CHAR-INVALID`, and that rule does not move). `random_int(lo, hi)`
keeps `(i32, i32) -> i32` — its bounds stay 32-bit (rejection-sampled
over the `i64` span internally); widening it to 64-bit bounds is a
separate additive overload, not part of step B. `parse_int` stays
strict-`i32` (a wider `parse_i64` is additive later); `int()`/`float()`
conversions keep today's truncation rules.

## 4. Migration impact

- The 26 published packages: **no edits required**. All stdlib values
  fit `i32` today; widening only *removes* failures (previously
  overflowing programs now run). `len`/`ord`/`file_size` keep returning
  `Int` with identical values on all current inputs. `W-TYPE-NARROW`
  stops firing on `i64`/`u32`/`u64` spellings (the lint is updated, not
  the packages).
- Repo (not packages) impact: tests pinning `E-OVERFLOW` at the `i32`
  boundary (`2147483647 + 1`, `i32::MIN / -1`, literal-range `E-TYPE`s)
  must move to the `i64` boundary (`9223372036854775807 + 1`, bare
  `9223372036854775808` literal). Enumerate them in the implementation
  PR (`grep E-OVERFLOW tests/`) — mechanical, but it is the bulk of the
  diff.
- Existing programs: strictly more programs run; none change value.
  One exception to call out: programs that *depend* on `E-OVERFLOW` at
  2³¹ as control flow (e.g. probing the boundary in tests) now run past
  it — the release notes name the new boundary.
- `u32`/`u64` values in `i64::MAX..u64::MAX`: still unrepresentable
  (documented; full fix needs Option A widths).

## 5. JIT / interpreter / v2 impact

- Interpreter: replace `to_i32_checked` + `i32::checked_*` with native
  `i64::checked_*` (`Add/Sub/Mul/Div/Mod/Neg/abs`, return-value check);
  literal-range check moves to `i64`; `file_size`'s `as i64` becomes
  exact for fitting files (still loud above `i64::MAX` — add that
  error, it is silent wrap today).
- v2 tree-walker: lockstep — it has its own `to_i32_checked`
  (`src/runtime/v2.rs:47`) and its own return-value range check
  (`:431-440`), so the same swap lands in both interpreters in one PR,
  covered by the same boundary matrix run under `run` and `run-v2`.
- JIT: storage already `I64` — **no width work**. The remaining gap is
  D6's (wrapping `iadd` vs loud overflow); D3 must not be declared done
  until the D6 checked-arithmetic gates cover the new boundary on both
  backends (a JIT that wraps at 2⁶³ while the interpreter errors is the
  same bug class at a new address).
- `u8` follow-up: needs a runtime representation decision (erased to
  `Int` with construction checks is enough — no JIT width support
  required; `u8(x)` is a builtin call, which the JIT already rejects
  loudly).

## 6. Test plan

- Boundary matrix on both backends: `i64::MAX`, `i64::MIN`, `±1` past
  each, `i64::MIN / -1`, `abs(i64::MIN)`, neg — interpreter loud
  `E-OVERFLOW`, JIT identical (with D6 implemented; before D6, JIT
  cases assert the *documented* divergence, see D6 §6).
- Literal inference: bare `5000000000` in `i64`/`Unknown` position OK,
  in `i32` position `E-TYPE`; `-9223372036854775808` OK (negated-limit
  rule generalized from today's `-2147483648`).
- Builtins: `len`/`file_size` past 2³¹ (synthetic: `range`-built? no —
  `range` caps at 100k; use a >2 GiB temp file for `file_size`, and a
  long-string `len` via repetition in the test harness).
- Narrowing: `u8` follow-up tests `u8(255)` OK, `u8(256)`/`u8(-1)`
  `E-OVERFLOW`, implicit widening in calls/returns.
- Back-compat: all 26 `*_test.klang` unchanged; repo overflow tests
  moved to new boundaries in the same PR (reviewer checks each moved
  assertion).
- `W-TYPE-NARROW` lint update: no warning for `i64` (honest now);
  `u32`/`u64` warn only past-`i64::MAX`… (unspellable in literals —
  so effectively silent; keep the arm for `Unknown`-flow documentation
  or retire it — implementation PR decides, tests pin the choice).

## 7. Effort estimate (S / M / L) and risks

**M** for Option B (range-check swap + literal inference + boundary-test
migration + lint update). Option A would be **L**; the `u8` follow-up is
**S/M**. C is **M** too (a bigint package is real work) but solves
nothing in the core — not recommended.

## 8. Decision (Wave1 S4, owner-approved, implemented)

**Adopted: Option B with checked narrow boundaries and a checked `u8`,
in one step.** `Int` is 64-bit with checked arithmetic: every arithmetic
op (`+ - * / %`, unary `-`, `abs`) traps past 64 bits as loud
`E-OVERFLOW` (rule `arithmetic/overflow`), never wrapping. The
annotations `i32`, `u32`, `i64`, `u64`, `u8` (plus `int`, a full-range
spelling) are all honest checked types: the narrower four enforce their
range at value boundaries, and a value outside the range is a loud
`E-RUNTIME` (rule `types/range`) naming the type and the value, e.g.

```text
`id` arg 0: value 256 out of `u8` range 0..255
```

Ranges: `i32` −2147483648..2147483647, `u32` 0..4294967295,
`i64`/`int` the full `i64` range (no narrowing), `u64` 0..2⁶⁴−1
(non-negative `i64` values pass; magnitudes past `i64::MAX` are
unspellable — still unrepresentable, as documented in §4),
`u8` 0..255. `W-TYPE-NARROW` is retired (the lint stays as an empty
pass so call sites are untouched): nothing it used to flag is wrong
anymore.

Boundaries covered (all three, runtime-enforced for checked and
unchecked callers alike):

- **Parameter passing**: each call argument is checked against the
  callee's declared annotation, including closure calls (captures
  carry no annotation and skip).
- **Return**: explicit `return` and fall-off values are checked against
  the declared return annotation.
- **Assignment to an annotated variable**: rebinding an annotated
  parameter name (`x = …`, lowered to `Copy`) re-checks it; struct
  construction (`S { n: … }`) and field assignment (`s.n = …`) check
  against the struct schema. (A `let` shadowing an annotated parameter
  name checks against that name's annotation too — the name is
  annotated in its scope. `let`-fresh names, array elements, and
  `Unknown`-typed positions never check.)

Compile-time vs run-time split: the checker owns what it can know —
bare literals past 64 bits are `E-TYPE` (`9223372036854775808` bare;
`-9223372036854775808` is valid `i64::MIN` via the generalized
negated-limit rule, folded to a constant at lowering so it never goes
through a trapping `Neg`), and static type agreement is unchanged (all
integer spellings are still `Ty::Int`, so no program that checks today
stops checking). The runtime owns values: boundaries and overflow are
`E-RUNTIME`/`E-OVERFLOW` at execution. Literals past `u64::MAX`
magnitudes stay `E-PARSE` ("integer literal out of range").

Unchanged by decision: `parse_int` stays strict-`i32` (`E-PARSE-INT`
outside it); `random_int(lo, hi)` keeps 32-bit bounds;
`ord`/`chr` keep their scalar rules; `len`/`file_size` keep returning
`Int` (now able to report past 2 GiB — e.g. `now_ms()` epoch millis
must be returned from `-> i64`, not `-> i32`); the `run_with_output`
`i32` channel keeps its limit (a test-helper channel, not a language
boundary). Enum payloads stay statically checked only (they lower
through `StructNew` under the enum name and are not in the struct
schema — a documented gap). The int-only JIT is untouched (wrapping
stays a loud-or-allowlisted divergence per D6, now pinned at the 64-bit
boundary instead of the 32-bit one).

Interpreter/v2 overlap: the v2 tree-walker gets the same 64-bit
checked arithmetic and the same `E-OVERFLOW` edges (same corpus
assertions run under `run` and `run-v2`). v2 has no annotated
boundaries — its parameter model does not spell widths — so boundary
behavior is v1-only by construction, and parity agrees wherever the
two overlap.

Gates: `tests/int_boundary_gates.rs` covers every annotated boundary
(parameter, return, assignment incl. struct construction/field
assignment) at, just inside, and just outside each type's range;
negatives for every unsigned type; `u8` checked; 64-bit overflow
still `E-OVERFLOW` on both backends; CLI exit codes (0 vs 1 with
`E-RUNTIME`). The moved i32-era assertions are enumerated in
`docs/progress/S4.md`.

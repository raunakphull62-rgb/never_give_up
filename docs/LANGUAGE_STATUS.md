# Klang Language Status — Phase 0 Audit (no behavior changes)

Date: 2026-10-07. Binary: `klang 0.7.0` built from this checkout
(`cargo build`, `CARGO_TARGET_DIR` outside the repo because the mounted
`target/` is non-executable). Suite: `cargo test` → **725 passed,
0 failed** (77 suites). Stdlib: all **26** `stdlib-packages/*/*_test.klang`
(excluding the network-dependent `http_live_test.klang`) print `"<pkg>: OK"`
and exit 0.

Method per item: (1) read the responsible code, (2) run a minimal Klang
program, (3) record program + output + file/line. Nothing was changed to
obtain these results. SPEC.md needed no correction in this phase (the only
mismatch found — `()` listed as a void spelling in docs/reference.md — is
recorded under item 5 as an audit note, not a behavior change).

Status meanings: **still true** = the reviewed limitation reproduces as
described; **partly** = the claim is only half right (usually: the checker
knows more than the runtime, or vice versa); **fixed** = the limitation is
gone.

## Phase 1 update (2026-10-07; behavior changes, see Phase 1 report)

This section amends the audit above — the per-item evidence stays valid as
a record of what Phase 0 found.

- Item 1 is now **fixed**: `&&`/`||` short-circuit on the interpreter and
  the JIT (branch-based MIR lowering; `tests/shortcircuit_gates.rs`).
  `docs/limitations.md` F11 and `docs/reference.md` §4 say so.
- Item 8 is now **fixed**: `klang add` takes one or more
  `<name[@constraint]...>` (resolved together, all-or-none).
- Item 5 ceremony is improved (same `E-PARSE` code): a missing `->` names
  the function and suggests `add -> i32`. SPEC.md §1, §2, §6 and
  `docs/reference.md` §§2–4 now state: return type required always,
  `main` may return any type (`Int` → exit code, else 0), `-> void` is
  the only unit spelling.
- Items 3/2: new non-failing `W-TYPE-NARROW` warning (severity `warning`)
  on `i64`/`u32`/`u64` (enforced as `i32`) and `u8` (unchecked); none of
  the 26 packages trigger it (`tests/type_narrow_gates.rs`).
- Item 5 ceremony is now sugar (Phase 3a, D4): omitting `->` desugars to
  `-> void` at parse time in both the v1 and v2 parsers; a
  value-returning function without a declared type is `E-TYPE` on the
  `return` line with an `add -> T` fix; `-> ()` stays rejected. Phase 3b
  decision: a `main` with no declared return type always exits 0
  (fall-off value discarded); a declared type maps as before
  (`tests/return_ergonomics_gates.rs`).
- This file's two evidence fences are `text`, not `klang`, so
  `scripts/verify_docs.py` (which executes every `klang` block in
  `docs/`) stays green.

## Summary table

| # | Item | Status | One-line verdict | Effort to change |
|---|------|--------|------------------|------------------|
| 1 | `&&` / `\|\|` short-circuit | still true | Both sides always evaluate (eager) | M |
| 2 | Real `bool` vs Int-as-bool; comparisons; `if`; JSON `true`/`false`/`null` | partly | Static `bool` exists and is checked; runtime erases it to `Int` 0/1; JSON still uses `{"$json": …}` marker maps; there is no `null` literal | L |
| 3 | Numeric types; overflow; `len()` / `ord()` / `file_size()` returns | partly | `i32` range enforced everywhere (`E-TYPE` for literals, `E-OVERFLOW` at runtime); `i64`/`u32`/`u64` are aliases, `u8` etc. are unchecked `Unknown`; `f64` works; `len`/`ord`/`file_size` return `Int` | L |
| 4 | Import collisions; qualified / aliased imports | still true | Same-named items from different files are `E-DUPLICATE`; no `import … as m` / qualified file-import syntax (only whole-file merge, selective `import {…} from`, and `mod m::item` paths) | M |
| 5 | Required return type; `main`; unit/void | partly | `-> type` is required on **every** `fn` including `main` and closures; `main` may return **any** type (not just `i32`); `-> void` works, `-> ()` is rejected by the parser despite docs/reference.md listing it | S |
| 6 | Cranelift JIT coverage | still true | Int-only backend; everything else (floats, strings, lists, closures, structs, enums, builtins, async ops, …) fails loudly, never silently | L (per value kind) |
| 7 | `sort` on mixed types | still true | Still rejects mixed-type lists with `E-TYPE` and an actionable message (check passes; rejection is at runtime) | S (messages) / M (cross-type order) |
| 8 | `klang add` with several names | still true | Exactly one `<name[@constraint]>` accepted; two names → usage + exit 2 | S |
| 9 | FFI / `extern` | still true | No `extern`/FFI mechanism at all; closest is the `run_process(cmd, args)` builtin (argv spawn, no shell) | L |

---

## 1. `&&` and `||` are eager (still true)

Claim under test: earlier audit item F11 said `&&`/`||` evaluate eagerly.

**Answer: still true — no short-circuit.**

Evidence (program + output):

```text
fn boom() -> i32 {
    print("BOOM")
    return 1
}
fn main() -> i32 {
    let x = false && boom()
    print(x)
    return 0
}
```

```
$ klang run sc1.klang
BOOM
0
```

`false && boom()` prints `BOOM` even though the left side already decides
the result. Mirror case `true || boom()` also prints `BOOM` and yields `1`.
Control cases behave as expected: `true && boom()` prints `BOOM` → `1`,
`false || boom()` prints `BOOM` → `0`. Strongest proof, an error on the
right is still raised:

```text
fn boom() -> i32 { return 1 / 0 }
fn main() -> i32 {
    let x = false && boom()
    print("reached")
    return 0
}
```

```
$ klang run sc_err.klang
run: FAIL
{ "code": "E-RUNTIME", "message": "division by zero", … }
```

Responsible code: `src/mir/mod.rs:880-959` (`bin()` lowers **both** sides
unconditionally, then emits one `MirOp::And`/`Or`); the interpreter
`src/runtime/mod.rs:913-936` looks up both operands and combines their
`truthy()` values; the JIT `src/jit.rs:444-465` evaluates both `I64`s and
does `band`/`bor` over them. None of the three stages has a conditional
jump for `&&`/`||`.

Effort to change: **M**. Lowering must emit branch control flow
(`JumpIfFalse`/`Jump` around the RHS) instead of a single ALU op, and all
three executors (interpreter, JIT, v2 runtime) plus checker tests must agree
on result values (still `Int` 0/1 today) and on which side effects are
skipped.

## 2. `bool`: a checked static type over an Int runtime (partly)

Claims under test: is there a real `bool` type or are booleans `Int`? What do
comparisons return? What does `if` accept? How are `true`/`false`/`null`
represented in JSON values (`$json` marker maps)?

**Answer: partly — the checker has a real `bool`; the runtime does not.**

- Literals/annotations: `true`/`false` lex as `TokenKind::True`/`False`
  (`src/lexer/resonance.rs:605-606`), parse to `Expr::Bool { value: bool }`
  (`src/parser/mod.rs:2074-2082`, `src/ast/mod.rs:602-605`), and check as
  `Ty::Bool` (`src/hir.rs:1698`). `bool` is a declared annotation
  (`src/hir.rs:80-89` `parse_ty`), usable on params and returns
  (`fn is_pos(x: i32) -> bool` checks clean).
- Comparisons return `Bool` statically: `==`/`!=` → `Ty::Bool`
  (`src/hir.rs:2416`), `<`/`<=`/`>`/`>=` → `Ty::Bool` (`src/hir.rs:2470`),
  `&&`/`||`/`!` take `Bool`/`Int` operands and return `Ty::Bool`
  (`src/hir.rs:2472-2530`). Misuse is caught: `true + 1` →
  `` `+` operands: want i32/f64/str, got bool `` (`E-TYPE`); returning a
  `bool` from `-> i32` → `` `main` return: want i32, got bool `` (`E-TYPE`).
- Conditions accept **both** `Bool` and `Int`: `if 1` / `if 0` run
  (`int-truthy`, `zero-else`), `if true` runs, `if "hi"` fails:
  `` if condition: want bool, got str `` (`E-TYPE`). Same rule for `while`
  (shared `require_condition`: `Bool | Int | Unknown`,
  `src/hir.rs:1303-1313`).
- Runtime erasure: `Expr::Bool` lowers to `MirOp::Const { value: 0/1 }`
  (`src/mir/mod.rs:401-412`); `Value` has **no** `Bool` variant — only
  `Int/Float/Str/Array/Map/Struct/Closure` (`src/runtime/mod.rs:58-76`).
  `And`/`Or`/`Not`/comparisons all produce `Value::Int(0/1)`
  (`src/runtime/mod.rs:873-959`). Live proof:

```
$ klang run bool1.klang   # print(true/false/1==1/1<2/true&&false)
1
0
1
1
0
```

`print(true)` prints `1`. `let` bindings take no annotation (`let b = …`;
  `let b: bool = …` is `E-PARSE`, `expected '='`), so `bool` only appears
  in param/return/closure-type positions.
- JSON: there is **no `null` literal** (`print(null)` →
  `E-UNDEFINED`, `undefined variable 'null'`). JSON `true`/`false`/`null`
  are `{"$json": "true"/"false"/"null"}` marker maps, exactly because the
  runtime cannot distinguish bool from int — the stdlib says so itself
  (`stdlib-packages/json/src/parse.klang:10-16`: *"Klang runtime has no bool
  distinct from int, so markers keep json_type honest"*). `json_type` /
  `json_is_true` / `json_is_false` / `json_is_null` match on the `$json` key
  (`stdlib-packages/json/src/parse.klang:552-640`); a colliding user object
  `{"$json": "null"}` parses as null (pinned by `json_test.klang`), while
  `{"$json": 42}` stays an object.

Effort to change: **L**. A runtime `Value::Bool` touches rendering,
`truthy`, every builtin coercion, equality, the JIT ABI, and JSON marker
migration for existing programs/packages (version gate, warning shim, or
migration tool).

## 3. Numerics: `i32` everywhere, `f64` alongside, loud overflow (partly)

Claims under test: which numeric types exist (`i32`, `f64`, anything else)?
What happens on overflow? What do `len()`, `ord()`, `file_size()` (and
similar) return?

**Answer: partly — the surface is wider than the semantics.**

- Existing types: `i32 | i64 | u32 | u64 | int` all parse to `Ty::Int`;
  `f32 | f64 | float` parse to `Ty::Float` (`src/hir.rs:80-89`). So `i64`
  annotations check and run (`fn f(x: i64) -> i64` → `f(41)` prints `42`),
  but `u8` (or any other name) parses as an identifier and resolves to
  `Ty::Unknown` — unchecked, dynamic, no error (`fn f(x: u8) -> u8` checks
  `OK` and runs). `let` has no type annotations at all.
- Overflow is **loud, never wrapping**: out-of-range literals are `E-TYPE`
  (`` integer literal: want i32 range, got i32 `` for `2147483648`;
  `-2147483648` is accepted via the negated-literal rule,
  `src/hir.rs:2531-2545`); arithmetic overflow is `E-OVERFLOW`
  (`` integer overflow in `add`: result out of i32 range ``,
  `src/runtime/mod.rs:212-234`, enforced per-op in `Add/Sub/Mul/Div/Mod/Neg`
  at `src/runtime/mod.rs:806-872,937-949` plus `num2_checked`). The `i64`
  alias does **not** widen the range: `fn f(x: i64) -> i64 { return x +
  2147483647 }` with `f(1)` fails `E-OVERFLOW` in `add`. Return values are
  range-checked the same way (`src/runtime/mod.rs:406-412`,
  `src/main.rs:803-815`). Floats are IEEE `f64` (`1.5 + 2.25` → `3.75`,
  `1 + 2.5` → `3.5` via int→float coercion, SPEC.md §2).
- Builtin returns are all `Int`: `len` → `Ty::Int` (`src/hir.rs:3263`),
  `ord` → `Ty::Int` (`src/hir.rs:3600`), `file_size` → `Ty::Int`
  (`src/hir.rs:3594`); at runtime `len` returns element count / entry count /
  **byte** length (`s.len() as i64`, `src/runtime/mod.rs:1339-1349` —
  `len("klang") == 5`, `len("é") == 2` per SPEC.md §4, verified live),
  `ord("A")` → `65`, `ord("")` → `E-CHAR-INVALID` (`ord failed: ord() of
  empty string`, `src/stdlib/seq.rs:32-37`), `chr(65)` → `"A"`,
  `chr(-1)` → `E-CHAR-INVALID` (`src/stdlib/seq.rs:42-50`),
  `file_size(path)` → byte count as `Int` (`Value::Int(n as i64)` from the
  `u64` metadata, `src/runtime/mod.rs:1805-1811`, `src/stdlib/file.rs:180`;
  live: writing `"hi"` then `file_size` prints `2`).

Effort to change: **L**. True `i64`/`u8`/unsigned semantics need literal
rules, widening rules, per-builtin signatures (`len` on 64-bit sizes?), the
`as i64` narrowing in `file_size`, and JIT/intepreter agreement on wrap vs
trap.

## 4. Imports collide; no qualified or aliased file imports (still true)

Claims under test: do two imported files defining the same name collide
(`E-DUPLICATE`)? Is there any qualified or aliased import syntax?

**Answer: still true on both counts.**

- Collision: importing `a.klang` and `b.klang` that both define `fn shared`
  fails: `` duplicate function `shared` `` with code `E-DUPLICATE`
  (`src/imports.rs:436-456`, `insert_fn`; re-merging the *same* file is
  idempotent/diamond-safe, a *different* file with the same name is loud).
- No alias syntax: `import "a.klang" as m` is `E-PARSE`
  (`expected 'fn'` — the parser only accepts `import "path"` or
  `import { names } from "path"`, `src/parser/mod.rs:919-941,962-1015`).
  `m.shared()` therefore cannot name a file import. What *does* exist:
  selective imports (`import { shared } from "a.klang"` runs and prints `1`)
  and qualified `mod m { … }` / `m::item` paths *inside* the program
  (`src/modules.rs`, SPEC.md §7) — modules, not file aliases.

Effort to change: **M**. `import "p" as m` + `m.f()` needs name-resolution
changes (qualified lookup, disambiguation with `mod::` paths and selective
imports) while keeping every existing unqualified import and all 26 prefixed
stdlib names compiling unchanged.

## 5. Return type required always; `main` is free; `void` yes, `()` no (partly)

Claims under test: is a return type required on every function, including
`main`? Must `main` return `i32`? Is there a unit/void form?

**Answer: partly — the "required" half is true, the "`main` must be `i32`"
half is false, and the unit story has a parser gap.**

- Required: the parser unconditionally expects `->` plus a type name
  (`src/parser/mod.rs:1193-1194`). `fn foo(a: i32) { … }` and
  `fn main() { … }` both fail with `expected '->'` (`E-PARSE`). Closure
  literals also require it (`fn(x: i32) { … }` is `E-PARSE`).
- `main` may return anything: `fn main() -> bool`, `-> void`, and `-> str`
  all `check: OK`. Exit-code mapping (`src/main.rs:1677-1682`): `Int`
  becomes the process exit code, any non-`Int` (e.g. `Str`) exits 0; since
  `Bool` erases to `Int` (item 2), `fn main() -> bool { return true }`
  runs (`run main() = 1`) and exits 1.
- Unit: `-> void` parses (`void` is an identifier that `parse_ty` maps to
  `Ty::Void`, `src/hir.rs:86`) and runs. `-> ()` is **rejected** by the
  parser (`expected return type` — `parse_ty_name` only accepts identifiers,
  `::` segments, and generics, `src/parser/mod.rs:765-816`) even though
  `parse_ty` would map `"()"` to `Void` and docs/reference.md §2 lists
  `` `void`, `()` ``. Audit note, not a Phase 0 change.
- The current missing-return-type error (`expected '->'`, `E-PARSE`, pointing
  at the `{`) is terse — the Phase 1c improvement ("say so clearly and show
  `add -> i32`") applies here.

Effort to change: **S**. Making `-> type` optional (default `Void`, plus a
clear diagnostic with a suggested fix) is parser + checker-default work with
no runtime impact; `()` acceptance is a one-arm parser fix.

## 6. JIT: int-only, loud everywhere else (still true)

Claim under test: which constructs does the Cranelift JIT compile, and which
fall back to the interpreter or are unsupported (floats, strings, lists,
closures, structs, enums, generics, async)?

**Answer: still true — int-only JIT behind `--backend-jit`; the interpreter
is the default (SPEC.md §8). No silent fallback: unsupported programs print
`run: JIT-FAIL` with the offending op.**

| Construct | JIT (`--backend-jit`) | Evidence |
|-----------|----------------------|----------|
| Int arithmetic, comparisons, `&&`/`\|\|`/`!`, `if`/`while` control (`JumpIfFalse`/`Jump`), `Copy`/`Const(int)`, `Call` to user fns, `print`, `Return`, `EnterGroup`/`LeaveGroup` (no-ops) | compiles & runs | `add(20,22)` → `42`; `true && false` → `0` (exit 0) |
| Generic fn monomorphized to ints (`id<T>` called with `41`) | compiles & runs | prints `41` |
| Floats (`ConstFloat`) | **unsupported** | `jit phase-1 supports int-only MIR; unsupported op ConstFloat in 'main'` |
| Strings (`ConstStr`) | **unsupported** | `… unsupported op ConstStr in 'main'` (also blocks `print("hi")`, `len("hi")`) |
| Lists/arrays (`ArrayNew`), maps (`MapNew`), indexing/slicing (`Index`/`StoreIndex`), `Len`, `MethodCall` | **unsupported** | `… unsupported op ArrayNew in 'main'` |
| Structs (`StructNew`/`FieldGet`/`FieldSet`) | **unsupported** | `… unsupported op StructNew in 'main'` |
| Enums + `match` (lower over the struct value, SPEC.md §2b) | **unsupported** (via struct ops) | same `StructNew` rejection path |
| Closures (`ClosureNew`, dynamic dispatch) | **unsupported** | `… unsupported op ClosureNew in 'main'`; NUL-named lifted bodies are never declared (`src/jit.rs:121-134`) |
| Any builtin call (`len`, `abs`, `sort`, …) even with int args | **unsupported** | `` … builtin `abs` unsupported in `main` `` (`src/jit.rs:480-486`) |
| `Spawn` / `Await` (async) | **unsupported** | `… unsupported op Spawn in 'main'` (interpreter runs the same program, exit 20) |
| `Try` (throws) and friends | **unsupported** (catch-all) | `src/jit.rs:542-548` |
| Entry arity > 4 | **unsupported** | `jit phase-1: entry arity > 4 unsupported` (`src/jit.rs:195-196`) |
| `Div`/`Mod` by zero | compiles but **traps** (process aborts) instead of interpreter `E-RUNTIME` | documented divergence, `src/jit.rs:65-70,427-431` |

Responsible code: `src/jit.rs:55-71` (contract comment), `src/jit.rs:406-548`
(the `match`: supported arms vs `ConstFloat | ConstStr` rejection at
`:412-418`, builtin rejection at `:480-486`, catch-all at `:542-548`).

Effort to change: **L** overall (each value kind needs an ABI: string/float
registers or boxed values, GC story, effect runtime for async, closure
environments). Int-only extensions (more arity, checked div) are S/M each.

## 7. `sort` still rejects mixed types, with a clear message (still true)

**Answer: still true.**

The static checker only requires an array (`sort` → `Ty::Array`,
`src/hir.rs:3632-3637`), so `sort([1, "two", 3])` passes `check: OK`; the
rejection happens at runtime (`src/runtime/mod.rs:1850` →
`src/stdlib/seq.rs:76-121`):

```
$ klang run sort_mixed.klang
run: FAIL
{ "code": "E-TYPE",
  "message": "sort() needs a homogeneous list (all ints, all floats, or all strings)",
  "cause": "mixed-type lists have no total order; convert explicitly first",
  "fixes": ["convert elements with int()/float()/str() first"],
  "rule": "calls/sort" }
```

Homogeneous sorts work: `[3,1,2]` → `[1, 2, 3]`, `[2.5,1.5]` → `[1.5, 2.5]`
(`total_cmp`, NaN-deterministic), `["b","a"]` → `[a, b]`. Empty/all-same
lists follow the same arms.

Effort to change: **S** for message tweaks (already actionable); **M** for
any cross-type total order (a design decision, not a bug fix).

## 8. `klang add` takes exactly one package (still true)

**Answer: still true — multi-name is rejected.**

```
$ klang add foo bar --offline
usage: add <name[@constraint]> [--dev] [--caret] [--registry URL] [--offline]
exit: 2
```

Responsible code: `src/main.rs:1041-1073` (takes only
`args.positional.first()`; `positional.len() > 1` prints usage and exits 2);
single-spec handling in `src/package_manager/cli.rs:40-61`
(`cmd_add`/`cmd_add_full` → `split_spec` `name[@constraint]`). Usage text in
`src/main.rs:946-950` documents the singular form.

Effort to change: **S** (Phase 1b: resolve all specs together, install
all-or-none, write the lock once atomically, keep the single-name form).

## 9. No FFI / `extern` mechanism (still true)

**Answer: still true — there is nothing to call out of the language.**

- `extern` is not a keyword (`TokenKind` list, `src/parser/mod.rs:34-94`,
  keyword table at `:585-607`); `extern fn getpid() -> i32` fails with
  `expected 'fn'` (`E-PARSE`). No `dylib`/`dlopen`/`syscall` surface exists
  in the lexer, parser, AST, HIR, or runtime (the only `extern "C"` in the
  tree is the JIT's own `host_print_i64` host-call shim, `src/jit.rs:94`,
  plus `std::ffi::OsString` plumbing in the registry — neither is a language
  feature).
- The closest user-visible mechanism is the process boundary:
  `run_process(cmd: str, args: array) -> map` with `stdout`/`stderr`/
  `exit_code` (argv-array spawn, no shell; `E-PROCESS-NOT-FOUND` for a
  missing binary), plus `env(name)` — SPEC.md §4, `docs/reference.md:156`.

Effort to change: **L** (minimal safe `extern` needs an ABI story, a
capability/sandboxing model, and a versioning plan — Phase 2 design item D5).

---

## What the review got right vs wrong

- Right: eager `&&`/`||` (item 1), import collisions with no aliasing
  (item 4), int-only JIT (item 6), `sort` rejecting mixed lists (item 7),
  single-name `klang add` (item 8), no FFI (item 9).
- Half-right: "booleans are Int" (item 2 — true at runtime, false in the
  checker, which has enforced `bool` since before this audit), "only
  i32/f64 exist" (item 3 — `i64`/`u32`/`u64` annotations exist but are
  range-identical aliases; `u8` parses but is unchecked), "return type
  required including `main` must be `i32`" (item 5 — required yes,
  must-be-`i32` no).
- Stale doc line (not a behavior issue): docs/reference.md §2 listed `()`
  as a void spelling, but the parser rejects `-> ()`
  (`expected return type`).

## Open questions / could not verify

- The "26 published stdlib packages (their versions on the registry are
  immutable)" constraint was honored by not publishing anything; registry
  immutability itself was not re-probed (out of scope, no network registry
  used — stdlib tests ran from local checkouts).
- `http_live_test.klang` was excluded (needs live network); the other 26
  tests cover the 26 packages 1:1.
- JIT `Div`/`Mod`-by-zero trapping (abort vs `E-RUNTIME`) is documented in
  `src/jit.rs:65-70` but was deliberately not crash-probed here.
- `file_size` narrows `u64 → i64` with `as` (`src/runtime/mod.rs:1811`); files
  ≥ 2^63 bytes would wrap — unverifiable in this environment, flagged for the
  D3 design note.

## Files changed (this phase)

- `docs/LANGUAGE_STATUS.md` (this file; new). No compiler, runtime, stdlib,
  registry, workflow, or npm changes. SPEC.md untouched (nothing user-visible
  changed).

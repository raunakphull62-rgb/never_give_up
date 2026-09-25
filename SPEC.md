# Klang Language Spec (v0.5, std-only)

Small enough to read in one sitting. This is what the compiler
actually enforces today (`cargo test` proves it).

## 1. Programs

```klang
import "lib.klang"
struct Point { x: i32, y: i32 }

fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    print(add(20, 22))
    return 42
}
```

- One file = structs + imports + `fn` items. `import "x.klang"`
  merges files (duplicate `fn` names are an error).
- `fn name(params) -> type [effects] { ... }`. Effects: `throws`,
  `async`, `cancel`. Calling a `throws` function requires `throws`
  on the caller (`E-EFFECT-MISMATCH`).
- Arguments are pass-by-value: mutating a struct, array, or map
  parameter inside the callee never affects the caller's binding.
  Return the updated value (or a status code) instead.

## 2b. Enums + match (checked)

```klang
enum Shape { Circle(r: i32), Rect(w: i32), Point }
enum Opt { Some(x: i32), None }

fn area(s: Shape) -> i32 {
    return match s { Shape::Circle(r) => r * 3, Shape::Rect(w) => w * 2, Shape::Point => 0 }
}
```

- Declared `enum Name { Variant, Other(x: i32), ... }` (tuple-style
  payloads `Variant(x: i32)`; bare `Variant` for no payload).
  Construction: `Shape::Circle(10)` (bare variants still take `()`
  at construction: `Shape::Point()`).
- `match scrutinee { Enum::Variant(bindings...) => expr, ... }`,
  arms comma-separated, `_ => ...` wildcard. Arm bodies are single
  expressions — a `{ ... }` block after `=>` parses as a map literal
  and fails with `E-PARSE`, so multi-statement arm logic goes in a
  helper function called from the arm.
- Exhaustiveness is a compile error: missing variants without a
  wildcard is `E-MATCH-EXHAUSTIVE`, naming every missing variant
  (`non-exhaustive match on \`Opt\`: missing Opt::None`) with fixes
  suggesting the missing arms or a wildcard.
- Duplicate variant arm: `E-MATCH-DUPLICATE`. Arm after a `_`
  wildcard: `E-MATCH-UNREACHABLE`. Wrong binding count for the
  payload: `E-ARITY`. Unknown variant: `E-UNDEFINED`.
  Cross-enum arm or non-enum scrutinee: `E-TYPE` (names the actual
  type). Variant payload types flow into arm bodies and are checked.
- Structural-identity, fmt, MIR lowering, and runtime dispatch all
  cover enums (tag dispatch over the existing struct value); see
  `tests/enum_gates.rs` (incl. a `klang repair` end-to-end that adds
  a missing arm with no repair-module changes).

## 2. Types (checked, `E-TYPE`)

| Annotation | Type |
|---|---|
| `i32`, `int`, `i64`, `u32`, `u64` | Int |
| `f64`, `float`, `f32` | Float |
| `str`, `string` | Str |
| `bool` | Bool |

`int` coerces to `float`. Everything else (map lookups, dynamic
index, unknown names) is `unknown`, a wildcard that never emits
`E-TYPE` — this keeps dynamic code working while still catching
`"hi" - 1`, `1 + true`, `return "hi"` in `-> i32`, `add("hi", 1)`,
`if "hi"`, `while "x"`, `for i in 0.."x"`, `for x in 42`.

- `+`: Int+Int=Int, Float-involved=Float, Str+anything=Str (concat).
- `- * /`: numbers only. `%`: integers only.
- `== !=`: same type, or Int/Float mix, or anything-unknown. Else `E-TYPE`.
- `< <= > >=`: numbers, or Str/Str. Result is Bool.
- `&& || !`: Bool/Int operands, Bool result. `if`/`while` need Bool/Int.
- `for i in a..b`: `a`, `b` must be Int. `for x in it`: `it` must be
  array/map/str. Loop var of `..` is Int, of `in` is unknown.
- Calls check arity (`E-ARITY`) and arg types. Returns check against
  the declared `-> type` (`E-TYPE`).
- Struct literals check field names exist (`E-UNDEFINED`) and field
  types match (`E-TYPE`). Unknown field access is `E-UNDEFINED`.

## 3. Structured concurrency

```klang
fn combine() -> i32 async {
    task_group {
        let a = spawn fetch(20)
        let b = spawn fetch(1)
        return await a + await b
    }
}
```

- `spawn` only inside `task_group` (`E-SPAWN-OUTSIDE-GROUP`).
- Every `let h = spawn ...` must be `await`ed before its group ends
  (`E-TASK-CANCEL`). No detached tasks, ever: every spawned thread is
  joined on its group's failure path too.
- Runtime runs each spawn on its own thread, `await` joins. Failure is
  lossless: one child failure returns unwrapped; several become
  `E-TASK-GROUP` with each failure under `related` (none dropped).
- Cooperative cancellation: the first failure cancels the group; tasks
  observe it at loop back-edges and exit with `E-CANCELLED`, which is
  excluded from the group. Spawned tasks always run once scheduled
  (no start-line race), so multi-failure groups are deterministic.
  Every infinite execution crosses a back-edge, so failure drains
  always terminate.

## 4. Stdlib (free functions + methods)

Free: `len`, `push`, `pop`, `range`, `str`, `int`, `float`, `keys`,
`assert`, plus file/env IO:

| Fn | Types | Meaning |
|---|---|---|
| `read_file(p: str) -> str` | checked | file contents, `E-IO-NOT-FOUND` if missing |
| `write_file(p: str, c: str) -> i32` | checked | writes, returns byte count |
| `append_file(p: str, c: str) -> i32` | checked | appends (creating when absent), returns byte count appended |
| `exists(p: str) -> bool` | checked | path exists |
| `env(name: str) -> str` | checked | env var or `""` |
| `run_process(cmd: str, args: array) -> map` | checked | argv-array spawn (no shell); map has `stdout: str`, `stderr: str`, `exit_code: i32`; non-zero exit is a normal result, missing binary is `E-PROCESS-NOT-FOUND` |
| `regex_is_match(pat: str, text: str) -> bool` | checked | true when the pattern matches; bad pattern is `E-REGEX-INVALID-PATTERN` |
| `regex_find(pat: str, text: str) -> map` | checked | map has `matched: bool`, `match: str`, `groups: array` (indexed 1..n), `named: map`; no match is `matched == 0`, not an error |

Methods: strings (`upper/lower/trim/split/contains/starts_with/
ends_with/replace/chars/len`), arrays (`len/push/pop/contains/join`),
maps (`len/keys/contains`). Unknown receivers skip checking.

`len()` contract (pinned): `len(s: str)` and `s.len()` return **byte
length** (Rust `String::len` convention), not char count:
`len("klang") == 5`, `len("é") == 2`, `len("—") == 3`.
`len(array)` is element count, `len(map)` is entry
count. `write_file`/`append_file` likewise return byte counts. Note the deliberate
asymmetry: `s[i]` and `s.chars()` are char-based (Unicode scalar values),
so `for i in 0..len(s)` is only valid for ASCII; non-ASCII code must map
char indices to byte offsets (e.g. via `s.chars()` plus `len()` of each
single-char string). Token `start`/`end` spans in `parser.rs` are byte
offsets (`source.as_bytes()`, `n = bytes.len()`).

## 5. Toolchain

```sh
cargo run -- --version          # print version (klang 0.1.0) and exit
cargo run -- check <file>        # parse + type-check (JSON diagnostics)
cargo run -- fmt <file> [--write]# canonical format (idempotent)
cargo run -- run <file> [entry]  # parse + check + run (entry default main)
cargo run -- build <file>        # parse + check + MIR listing
cargo run -- repair <file> [--max-iters N] [--scope function|file] [--dry-run]
                                 # repair-prompt inspector (see below)
cargo run -- mcp                 # MCP server on stdio: klang_check,
                                 # klang_run, klang_fmt, klang_scope_plan
```

`fmt` rules: 4-space indent, one statement per line, binary ops fully
parenthesized. `fmt(fmt(x)) == fmt(x)`; formatted code re-parses and
runs identically (tested in `tests/toolchain_gates.rs`).

`repair` mechanism (Phase 4: Klang no longer calls a model — the
harness owns the model connection and drives the loop via `klang mcp`):
parse + `TypedHIR::check` the target file; the failing scope's source
plus `Diagnostic::to_json()` output is exactly what `--dry-run` prints
and what `klang_scope_plan` returns, for the harness to send to its own
model. A harness-side response is spliced back as `fn` block(s) and the
full program is re-checked, repeating up to `--max-iters` (default 5,
hard max 10) via `contracts::repair_loop`. Default scope is `function`
(falls back to whole-file when diagnostics span >2 functions).
`--dry-run` prints the planned prompt with zero model calls. Tested in
`tests/repair_gates.rs` (mock backend: the mock stands in for the
harness-owned model) and end-to-end through the MCP surface in
`tests/mcp_gates.rs` (scripted fake harness: check -> scope -> fix ->
check clean).

`repair` scope boundaries (Phase 1 decision, pinned by tests):
- `--scope function` never claims declaration-level diagnostics: rules
  `names/duplicate`, `modules/visibility`, and `ownership/mode` always
  repair at whole-file scope (`scope::DECLARATION_RULES` /
  `is_declaration_level`), because a declaration edit is not expressible
  by function-scoped splicing and name-keyed splicing is ambiguous
  exactly when names collide.
- A model response containing declaration text (`enum`/`struct`/`mod`/
  `import`) is accepted only as a complete file (every original function
  AND declaration present, per `splice::response_declarations`);
  otherwise the attempt fails loudly rather than silently dropping the
  declaration edit. Declaration-free responses still splice per function
  and leave declarations byte-identical.
- `match/*` diagnostics scope to the function whose body contains the
  `match`, not to every function that merely mentions the enum.

## 6. Packages

`klang.toml`:

```toml
name = "demo"
version = "0.1.0"
entry = "main"
[dependencies]
mylib = "./mylib.klang"
[repair]
# max_iters = 5
# scope = "function"  # or "file"
# (retired keys endpoint/model/api_key/timeout are ignored)
```

Lockfile: `write_lock` / `parse_lock` one `"<file> <hex>"` line per
file (FNV-1a of bytes). `verify_lock` reports hash mismatches and
missing files. Tested in `tests/package_gates.rs`.

## 7. What is here from v0.2 (audited, code-verified)

- Enums + `match` with exhaustiveness (`E-MATCH-EXHAUSTIVE`), wildcards,
  duplicate/unreachable arms, binding arity, non-enum scrutinee — see §2b.
- Generics: type parameters on `struct`, `enum`, and `fn` with per-site
  inference and conflicting-instantiation `E-TYPE`
  (`tests/generic_gates.rs`). No trait bounds/typeclasses.
- Multi-file modules: `mod name { ... }` blocks, `pub`/private
  visibility (`E-PRIVATE`), qualified `m::item` paths, plus cross-file
  `import` merging (`tests/module_gates.rs`).
- `examples/full.klang` and `examples/simple.klang` still run; the whole
  suite is 218 tests (`cargo test`), including the Phase 4 MCP gates
  (`tests/mcp_gates.rs`: scripted fake harness proving identical
  diagnostics and scope verdicts through the MCP surface), the Phase 2 adversarial
  gates (`tests/type_adversarial_gates.rs`: missing struct fields,
  nominal struct signatures) and the multi-feature dogfood
  (`examples/eval.klang`: lexer/parser/eval modules over a generic
  `Res<T>` + `Expr` enum with `throws` propagated to `main`, pinned by
  `tests/eval_gates.rs`), plus the Phase 3 offline
  benchmark baseline (`tests/benchmark_gates.rs`: 8-task corpus across
  simple/moderate/complex tiers, 100% oracle-repair convergence in 1
  median iter; live-model convergence is measured inside harnesses via
  `klang mcp`, and `bench/run_live.sh` now checks the harness signal —
  diagnostic family + planned scope per task — with zero model calls).

## 7b. Robustness (Phase 2 hardening)

The CLI runs its work on a deep-stack worker
(`klang::with_deep_stack`, 256 MiB — `src/lib.rs`), because the front end
and checker recurse over the AST: a flat `1+1+...` chain of a few hundred
operands used to overflow the default 8 MiB stack and abort the process
instead of reporting. Pathological input (empty files, truncated `fn`,
unterminated strings/comments, garbage bytes, hundreds of nested blocks,
tens of thousands of operands) now always produces either `check: OK` or a
structured diagnostic — never a crash. Pinned by
`tests/robustness_gates.rs`.

Call depth is capped separately at the interpreter level: nested
calls deeper than 64 frames fail at run time with `E-RUNTIME` ("call
depth exceeded"), never a native stack overflow. Depth 63 and below
runs normally. The cap is deliberately conservative (it guards the
interpreter's own native recursion); realistic deep recursion past it
is a known constraint, not a crash bug. Pinned by
`call_depth_limit_is_loud_never_a_crash`.

## 8. What is still NOT here (honest list)

No full-value machine-code backend (int-only Cranelift JIT behind
`--backend-jit`; interpreter is the default), no closures, no real borrow
checker (Managed mode only), no async I/O runtime, no registry/network
packages, no LSP server (only a JSON renderer), no debugger/profiler, no
recursive/nested enum payloads needing indirection, no match guards, no
multi-statement match arms (single-expression bodies; use helper
functions), no tuple-variant syntax or partial destructuring. See `roadmap.md` for
sequencing and `AUDIT.md` for the Phase 0 evidence table.

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
| `read_file(p: str) -> str` | checked | file contents, `E-RUNTIME` if missing |
| `write_file(p: str, c: str) -> i32` | checked | writes, returns byte count |
| `exists(p: str) -> bool` | checked | path exists |
| `env(name: str) -> str` | checked | env var or `""` |

Methods: strings (`upper/lower/trim/split/contains/starts_with/
ends_with/replace/chars/len`), arrays (`len/push/pop/contains/join`),
maps (`len/keys/contains`). Unknown receivers skip checking.

`len()` contract (pinned): `len(s: str)` and `s.len()` return **byte
length** (Rust `String::len` convention), not char count:
`len("klang") == 5`, `len("é") == 2`, `len("—") == 3`.
`len(array)` is element count, `len(map)` is entry
count. `write_file` likewise returns byte count. Note the deliberate
asymmetry: `s[i]` and `s.chars()` are char-based (Unicode scalar values),
so `for i in 0..len(s)` is only valid for ASCII; non-ASCII code must map
char indices to byte offsets (e.g. via `s.chars()` plus `len()` of each
single-char string). Token `start`/`end` spans in `parser.rs` are byte
offsets (`source.as_bytes()`, `n = bytes.len()`).

## 5. Toolchain

```sh
cargo run -- check <file>        # parse + type-check (JSON diagnostics)
cargo run -- fmt <file> [--write]# canonical format (idempotent)
cargo run -- run <file> [entry]  # parse + check + run (entry default main)
cargo run -- build <file>        # parse + check + MIR listing
```

`fmt` rules: 4-space indent, one statement per line, binary ops fully
parenthesized. `fmt(fmt(x)) == fmt(x)`; formatted code re-parses and
runs identically (tested in `tests/toolchain_gates.rs`).

## 6. Packages

`klang.toml`:

```toml
name = "demo"
version = "0.1.0"
entry = "main"
[dependencies]
mylib = "./mylib.klang"
```

Lockfile: `write_lock` / `parse_lock` one `"<file> <hex>"` line per
file (FNV-1a of bytes). `verify_lock` reports hash mismatches and
missing files. Tested in `tests/package_gates.rs`.

## 7. What is still NOT here (honest list)

No full-value machine-code backend (int-only Cranelift JIT behind
`--backend-jit`; interpreter is the default), no generics,
no closures, no real borrow checker (Managed mode only), no async I/O
runtime, no registry/network packages, no LSP server (only a JSON
renderer), no debugger/profiler. See `roadmap.md` for sequencing.

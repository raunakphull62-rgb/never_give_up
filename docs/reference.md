# Language reference

Precise rules per construct, then the full diagnostic catalog. Where
the tour shows, this page states. Notation: `E-CODE` entries each carry
a minimal program demonstrating the trigger.

## 1. Lexical matters

- Comments: `//` to end of line, `/* ... */` blocks (unterminated
  block comment is an `E-PARSE` error, not a silent truncation).
- Statements end at the newline; a trailing `;` is accepted and
  ignored. One statement per line in canonical `fmt` output.
- Integer literals are decimal (`42`), floats need digits on both
  sides of the dot (`4.0`; `0..10` still lexes as a range because the
  dot must be followed by a digit to count as a float). Integer
  literals must fit `i32` (`-2147483648` is allowed via unary minus);
  out-of-range literals are `E-TYPE`, and literals beyond `i64`
  are `E-PARSE`.
- String literals use double quotes with `\n \r \t \" \\` escapes
  (plus `\uXXXX`); an unterminated string is `E-PARSE`. Non-ASCII is
  preserved; token spans are byte offsets.
- Keywords: `fn let return if else while for in break continue print
  struct enum match mod pub import task_group spawn await throws
  async cancel`.

## 2. Types

| Annotation | Checked as |
|---|---|
| `i32`, `int`, `i64`, `u32`, `u64` | 32-bit integer |
| `f64`, `float`, `f32` | 64-bit float |
| `str`, `string` | string |
| `bool` | boolean |
| `void`, `()` | no value |
| enum name (e.g. `Opt`) | that nominal enum |
| struct name (e.g. `Point`, `m::Token`) | that nominal struct |
| type parameter (e.g. `T` inside `fn f<T>`) | rigid type variable |
| anything else (`array`, `map`, unknown names) | `unknown` (dynamic) |

`int` coerces to `float` wherever a float is wanted. `unknown` is the
documented escape hatch: map lookups, dynamic indexes, unknown names,
and generic-variable positions never emit `E-TYPE` — dynamic code
keeps working while concrete mismatches (`"hi" - 1`, `1 + true`,
`return "hi"` from `-> i32`) are rejected. Generic struct fields and
generic match bindings are also `unknown` at the use site.

## 3. Functions and statements

```klang
// @run prints: 22 ; return: 42
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    print(add(20, 2))
    return add(40, 2)
}
```

- Signature: `fn name(p: ty, ...) -> ret [throws] [async] [cancel]`.
  Arity is exact (`E-ARITY`); argument and return types are checked
  (`E-TYPE`); duplicate parameter names are `E-DUPLICATE`.
- `let x = expr` binds (re-`let` shadows with the new type);
  `x = expr` rebinds and checks against the bound type; assigning an
  unbound name is `E-UNDEFINED`.
- `return expr` must match the declared return type. Every variable
  must be a parameter or a prior `let`, else `E-UNDEFINED`.
- `print(expr)` accepts any value. `break`/`continue` are loop-only
  (`E-LOOP` outside one).
- `if`/`while` conditions must be `bool` or `int` (`E-TYPE`
  otherwise). `for i in a..b` needs integer bounds; `for x in xs`
  needs an array or string (a map is `E-TYPE`); the `..` loop variable
  is `i32`, the `in` element is dynamic.

## 4. Operators

| Operator | Rule |
|---|---|
| `+` | Int+Int=Int; any float mix=Float; Str+anything=Str (concat) |
| `- * /` | numbers only (int/float, `int` coerces) |
| `%` | integers only |
| `== !=` | same type, int/float mix, or anything-`unknown` |
| `< <= > >=` | numbers, or Str/Str; result Bool |
| `&& \|\| !` | Bool/Int operands, Bool result — **eager**: both sides always evaluate (see [limitations](limitations.md)) |
| unary `-` | numbers (`-x` on a variable stays checked) |

Anything else is `E-TYPE`. Match arms unify order-independently
(Int/Float mix to Float either way; disagreement is `E-TYPE`).

## 5. Structs, arrays, maps, strings

```klang
// @run prints: 42 ; return: 42
struct Point { x: i32, y: i32 }

fn main() -> i32 {
    let p = Point { x: 19, y: 20 }
    p.y = 23
    print(p.x + p.y)
    return p.x + p.y
}
```

- Literals name every declared field (`E-UNDEFINED` for unknown
  fields, `E-TYPE` for mistyped values, `E-ARITY` for missing fields).
  Unknown struct names are `E-UNDEFINED`. Field access on a missing
  field is `E-UNDEFINED`; on a non-struct is `E-TYPE`.
- Struct fields named `__variant`, `__*`, or `f<digits>` are rejected
  (`E-RESERVED-FIELD`): that namespace is the enum runtime
  representation.
- Arrays (`[1, 2]`), indexing (`a[i]`, integer or dynamic index),
  element assignment (`a[0] = 40`), maps (`{"k": v}`, `m[k]`,
  `m[k] = v`). Bad index/base combinations are `E-TYPE`; a missing
  map key fails at run time (`E-RUNTIME`), never at check time.
- `push`/`pop` (free functions and array methods) must mutate a
  variable, not a temporary (`[1,2].push(3)` is `E-TYPE`).
- String `s[i]` is character-based; `len(s)` is byte length.
  `s.chars()` yields single-character strings.

Free functions and methods:

| Function | Arity | Notes |
|---|---|---|
| `len(x)` | 1 | str→bytes, array→elements, map→entries; else `E-TYPE` |
| `push(a, v)` / `pop(a)` | 2 / 1 | variable target must be array |
| `range(a, b)` | 2 | int bounds; returns array |
| `str(x)` / `int(x)` / `float(x)` | 1 | conversions (`int(4.9)`→4, `float(2)`→2.0) |
| `keys(m)` | 1 | map→array of keys |
| `assert(c)` | 1 | bool/int; failure is `E-RUNTIME` at run |
| `read_file(p)` / `write_file(p, c)` / `append_file(p, c)` / `exists(p)` / `remove_file(p)` / `env(n)` | 1 / 2 / 2 / 1 / 1 / 1 | file/env IO; missing file is `E-IO-NOT-FOUND` (paths outside the temp dir are still `E-RUNTIME` via the unsafe-path guard) |
| `run_process(cmd, args)` | 2 | argv-array spawn, no shell; returns map `stdout`/`stderr`/`exit_code`; non-zero exit is a normal result, missing binary is `E-PROCESS-NOT-FOUND` |
| `regex_is_match(pat, text)` / `regex_find(pat, text)` | 2 / 2 | regex search; `find` returns map `matched`/`match`/`groups`/`named`; no match is a normal result, bad pattern is `E-REGEX-INVALID-PATTERN` |

String methods (`upper lower trim chars len split contains
starts_with ends_with replace`), array methods (`len push pop
contains join`), map methods (`len keys contains`) — all arity-checked
(`E-ARITY`), unknown methods on a known receiver are `E-TYPE`, and any
method on an `unknown` receiver skips checking. Calling a method on an
`int`/`bool`/struct/enum value is `E-TYPE`.

## 6. Enums and match

```klang
// @run prints: 40 ; return: 0
enum Shape { Circle(r: i32), Rect(w: i32), Point }

fn area(s: Shape) -> i32 {
    return match s { Shape::Circle(r) => r * 3, Shape::Rect(w) => w * 2, Shape::Point => 0 }
}

fn main() -> i32 {
    print(area(Shape::Circle(10)) + area(Shape::Rect(5)) + area(Shape::Point()))
    return 0
}
```

- Declaration `enum Name { V, W(x: i32), ... }`; construction
  `Name::V(...)` with exact payload count (`E-ARITY`); bare variants
  construct with `()`. Unknown variant of a known enum is
  `E-UNDEFINED`; unknown enum name is `E-UNDEFINED`.
- `match scrutinee { Enum::V(bindings) => expr, ... }`, arms separated
  by commas, `_ => ...` wildcard. Exhaustiveness is enforced
  (`E-MATCH-EXHAUSTIVE` naming every missing variant); duplicate arms
  are `E-MATCH-DUPLICATE`; arms after `_` are `E-MATCH-UNREACHABLE`;
  binding-count mismatches are `E-ARITY`; cross-enum arms and
  non-enum scrutinees are `E-TYPE` (naming the actual type).
- Not in the language: struct-style variants, partial `{ f, .. }`
  destructuring, match guards, tuple-variant syntax.

## 7. Generics

```klang
// @run prints: 3 ; return: 42
struct Entry<K, V> { key: K, val: V }

fn first<T>(e: Entry) -> i32 {
    return 0
}

fn same<T>(a: T, b: T) -> T {
    return a
}

fn main() -> i32 {
    let e1 = Entry { key: "a", val: 1 }
    let e2 = Entry { key: 2, val: "b" }
    print(e1.val + e2.key)
    return same(20, 22) + 22
}
```

- `fn f<T, ...>`, `struct S<T, ...>`, `enum E<T, ...>`; duplicate type
  parameters are `E-DUPLICATE`.
- Inference is per call/construction site from argument types; a
  conflicting site is `E-TYPE` naming the parameter and its binding
  site (e.g. `` `same` arg 1: T was inferred as i32 from arg 0, got str``).
  Unconstrained variables become
  `unknown`, never an error. Explicit type arguments are supported both
  in annotations (`Opt<i32>`, `a::Box<i32>`, nested `Opt<Opt<i32>>`)
  and at call sites (`count<T>(n)`, `count<i32>(5)`); call-site `<...>`
  disambiguates by backtracking (commits only on the full
  `< Type (, Type)* > (` shape, otherwise `<` stays a comparison, so
  `a < b` is unaffected).
- No trait bounds or typeclasses.

## 8. Modules and imports

```klang
// @run prints: 40 ; return: 40
mod shape {
    pub enum Kind { Circle(r: i32), Rect(w: i32) }

    pub fn area(k: Kind) -> i32 {
        return match k { Kind::Circle(r) => r * 3, Kind::Rect(w) => w * 2 }
    }
}

fn main() -> i32 {
    print(shape::area(shape::Kind::Circle(10)) + shape::area(shape::Kind::Rect(5)))
    return shape::area(shape::Kind::Rect(5)) * 4
}
```

- `mod name { ... }` blocks; `pub` on functions, structs, enums.
  Cross-module use is via `m::item` paths; referencing a private item
  is `E-PRIVATE`. Same-named private items in different modules do not
  collide. Top-level declarations and `mod` blocks mix freely in one
  file.
- Cross-file: `import "lib.klang"` merges files (duplicate `fn` names
  across the merge are an error). Only plain relative paths are
  accepted — absolute paths, `..` escapes, `~`, and NUL bytes are
  rejected without touching the filesystem.

## 9. Effects and structured concurrency

- `throws`: calling a `throws` function requires `throws` on the
  caller (`E-EFFECT-MISMATCH`, carrying the caller's name span).
  There is no `throw` statement — effects are propagation-checked
  annotations; a `throws` function that calls nothing throwing is
  legal (it declares a failure boundary).
- `async`: any function using `task_group`/`spawn`/`await` must
  declare it (`E-EFFECT-MISMATCH` otherwise).
- `cancel`: accepted in signatures but not propagation-checked.
- `task_group { let h = spawn f() ... return await h }`: `spawn` only
  directly bound by `let` inside a group (`E-SPAWN-OUTSIDE-GROUP`
  outside one, `E-SPAWN-POSITION` for nested/bare/escaping spawns);
  every handle awaited before group end (`E-TASK-CANCEL`); `await`
  outside a group is `E-AWAIT-OUTSIDE-GROUP`; awaiting an undefined
  handle is `E-UNDEFINED`.
- Runtime: each spawn runs on its own thread, `await` joins. One
  child failure returns unwrapped; several become `E-TASK-GROUP`
  with each failure under `related` (none dropped). The first failure
  cancels siblings (cooperative, at loop back-edges); cancellation is
  excluded from the group report.

## 10. Diagnostic catalog

Check-time codes first (all verified by the sample shown), then
run-time codes. Every HIR diagnostic below carries a `0,0` span except
where noted — see [limitations](limitations.md).

```klang
// @check-fail E-TYPE
fn main() -> i32 {
    return "hi" - 1
}
```

```klang
// @check-fail E-ARITY
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    return add(1)
}
```

```klang
// @check-fail E-UNDEFINED
fn main() -> i32 {
    return nope + 1
}
```

```klang
// @check-fail E-DUPLICATE
fn f(a: i32, a: i32) -> i32 {
    return a
}

fn main() -> i32 {
    return f(1, 2)
}
```

Duplicate top-level `fn` names never reach the checker: the
import-merge stage rejects the file first (`parse: FAIL` with a plain
`duplicate function` message). Duplicate parameters, struct fields,
enum variants, and type parameters fail at check time as above.

```klang
// @check-fail E-PARSE
fn main() -> i32 {
    return 1
```

```klang
// @check-fail E-LOOP
fn main() -> i32 {
    break
    return 0
}
```

```klang
// @check-fail E-PRIVATE
mod m {
    fn secret() -> i32 {
        return 1
    }
}

fn main() -> i32 {
    return m::secret()
}
```

```klang
// @check-fail E-EFFECT-MISMATCH
fn boom() -> i32 throws {
    return 1
}

fn main() -> i32 {
    return boom()
}
```

```klang
// @check-fail E-MATCH-EXHAUSTIVE
enum Opt { Some(x: i32), None }

fn f(v: Opt) -> i32 {
    return match v { Opt::Some(n) => n }
}

fn main() -> i32 {
    return 0
}
```

```klang
// @check-fail E-MATCH-DUPLICATE
enum Opt { Some(x: i32), None }

fn f(v: Opt) -> i32 {
    return match v { Opt::Some(n) => n, Opt::Some(m) => m, Opt::None => 0 }
}

fn main() -> i32 {
    return 0
}
```

```klang
// @check-fail E-MATCH-UNREACHABLE
enum Opt { Some(x: i32), None }

fn f(v: Opt) -> i32 {
    return match v { Opt::Some(n) => n, _ => 0, Opt::None => 1 }
}

fn main() -> i32 {
    return 0
}
```

```klang
// @check-fail E-TASK-CANCEL
fn fetch_a() -> i32 {
    return 1
}

fn bad() -> i32 async {
    task_group {
        let a = spawn fetch_a()
        return 1
    }
}

fn main() -> i32 {
    return 0
}
```

```klang
// @check-fail E-SPAWN-OUTSIDE-GROUP
fn f() -> i32 {
    return 1
}

fn main() -> i32 {
    let a = spawn f()
    return 0
}
```

```klang
// @check-fail E-SPAWN-POSITION
fn f() -> i32 {
    return 1
}

fn main() -> i32 async {
    task_group {
        let y = spawn f() + 1
        return await y
    }
}
```

```klang
// @check-fail E-AWAIT-OUTSIDE-GROUP
fn main() -> i32 {
    return await b
}
```

```klang
// @check-fail E-RESERVED-FIELD
struct S { __variant: i32 }

fn main() -> i32 {
    return 0
}
```

Run-time codes (check clean, fail at `run`):

```klang
// @run-fail E-RUNTIME
fn main() -> i32 {
    return 1 / 0
}
```

```klang
// @run-fail E-TASK-GROUP
fn fail_a() -> i32 {
    return 1 / 0
}

fn fail_b() -> i32 {
    assert(1 == 2)
    return 0
}

fn main() -> i32 async {
    task_group {
        let a = spawn fail_a()
        let b = spawn fail_b()
        let x = await a
        let y = await b
        return x + y
    }
}
```

A single child failure returns unwrapped (the `E-RUNTIME` above, not
`E-TASK-GROUP`); `E-CANCELLED` is the runtime-internal notice for a
sibling cancelled after another failed — it is excluded from group
aggregates and never surfaces as the top-level error.

Library-only codes (never emitted by `klang check`; no surface syntax
produces them): `E-CONTRACT` (executable `non-empty` preconditions via
the `contracts` API) and `E-OWNERSHIP-MODE` (non-`Managed` modes via
the `ownership` API — only `Managed` ships).

## 11. What is not in the language

No closures or function values; no trait bounds; no match guards / partial destructuring /
struct-style variants; no recursive struct fields that must be
inhabited without a dynamic producer; no `throw` statement; no
short-circuit `&&`/`||`; no const items; no string interpolation;
no machine-code backend for non-integer code (interpreter is the
reference; `--backend-jit` is integer-only); no registry/network
packages; no LSP server (JSON renderer only); no debugger/profiler.
See [limitations](limitations.md) for the honest subset that matters
most in practice.

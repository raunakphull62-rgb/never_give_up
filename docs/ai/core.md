# Klang for AI — core guide

Example-first. Every snippet was executed with the real compiler.
Run `klang check <file>` (human lines plus JSON diagnostics) or
`klang check --json <file>` (one `{"diagnostics":[...]}` object) and
`klang run [--sandbox] <file>` (only program output on stdout, diagnostics on stderr).
`klang run --sandbox` limits files to the temp dir (`E-SANDBOX-FILE`) and denies
spawn/run (`E-SANDBOX-SPAWN`) and network (`E-SANDBOX-NETWORK`); plain `klang run`
allows all relative and absolute paths (OS permissions only, `E-IO-*` on failure)
and spawn/network. `klang fmt --check <file>` / `--write` reports/rewrites
canonical form. `klang mcp` was removed (`E-CLI-REMOVED`); use the CLI above.
Loop: write file, run `klang check --json`, fix first diagnostic by its `code`
(see errors.md), re-check until clean, then `klang run`. Keep `fmt --check` clean.

## Toolchain

Minimal: check a file.

```klang run
fn main() -> i32 {
    print(1)
    return 0
}
```

```output
1
```

Realistic: check output is JSON lines; run prints only program output.

```klang run
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    print(add(20, 22))
    return 0
}
```

```output
42
```

Rules: `klang check <file>` prints `check: OK` or `check: FAIL` plus JSON lines; `klang check --json <file>` prints one single-line `{"diagnostics":[...]}` object (same `Diagnostic::to_json()` objects: `code`, `message`, `primary_span{file,start,end,line,column}`, `cause`, `fixes`, `rule`, `related`); clean is `{"diagnostics":[]}` exit 0, any diagnostic is exit 1; follows imports (diagnostic names the defining file), respects `--strict`/manifest, flag order flexible (`check --json f` == `check f --json`). Loop: write file, `check --json`, fix first diagnostic by `code` (see errors.md), re-check, then `klang run <file>` (program output only, exit code is return value). `klang run --sandbox` limits files to temp dir and denies spawn/network (see file section); `klang fmt [--write|--check] <file|dir>` prints/rewrites/checks canonical form (4-space indent, one statement per line, fully parenthesized ops, one trailing newline; `--check` exits 1 with `fmt: would change <path>` unless `fmt: clean`).

## Variables and assignment

Minimal: `let` binds, `=` rebinds same type.

```klang run
fn main() -> i32 {
    let x = 1
    x = 2
    print(x)
    return 0
}
```

```output
2
```

Realistic: `let` shadows with a new type; `=` checks the old type.

```klang run
fn main() -> i32 {
    let x = 1
    let x = "hi"
    print(x)
    print(len(x))
    return 0
}
```

```output
hi
2
```

Rules: `let x = e` binds (re-`let` shadows, new type ok); `x = e` rebinds (same type only); unbound use is `E-UNDEFINED`.

## Types — Int (64-bit checked)

Minimal: full `i64` range, overflow is loud.

```klang run
fn main() -> i32 {
    print(100 + 23)
    return 0
}
```

```output
123
```

Realistic: overflow never wraps.

```klang error E-OVERFLOW
fn main() -> i32 {
    print(9223372036854775807 + 1)
    return 0
}
```

Rules: `Int` is 64-bit checked; `+ - * / %` and unary `-` past range are `E-OVERFLOW`.

## Types — annotated boundaries (i32/u32/i64/u64/u8)

Minimal: `u8` accepts 0..255.

```klang run
fn take_u8(x: u8) -> i32 {
    return x
}

fn main() -> i32 {
    print(take_u8(255))
    return 0
}
```

```output
255
```

Realistic: out-of-range at call/return/assign is `E-RUNTIME`.

```klang error E-RUNTIME
fn f(x: i32) -> i32 {
    return x
}

fn main() -> i32 {
    print(f(2147483648))
    return 0
}
```

Rules: `i32`/`u32`/`u64`/`u8` enforce range at arg/return/rebind/struct boundaries (`E-RUNTIME`); `i64`/`int` accept full range.

## Types — Float

Minimal: `+` with any float gives float.

```klang run
fn main() -> i32 {
    print(1 + 2.5)
    return 0
}
```

```output
3.5
```

Realistic: float division by zero is loud, never inf/NaN.

```klang error E-RUNTIME
fn main() -> i32 {
    print(1.0 / 0.0)
    return 0
}
```

Rules: `int` coerces to `float`; `1.0 / 0.0`, `sqrt` of negative, `log` of non-positive, overflowing `pow` are `E-RUNTIME`.

## Types — Bool and truthiness

Minimal: lowercase `true`/`false` print as 1/0.

```klang run
fn main() -> i32 {
    print(true)
    print(false)
    return 0
}
```

```output
1
0
```

Realistic: `if`/`while` take `Bool`/`Int`; empty values are falsy at runtime but statically only `Bool`/`Int` pass.

```klang run
fn main() -> i32 {
    let x = 0
    if x == 0 {
        print(1)
    }
    print(!0)
    print(!5)
    return 0
}
```

```output
1
1
0
```

Rules: truthy is non-zero/non-empty; `!`/`&&`/`||` take `Bool`/`Int` and return `Bool` (prints 0/1); `if "hi"` is `E-TYPE`.

## Types — Str

Minimal: `+` concats anything.

```klang run
fn main() -> i32 {
    print("hello " + "klang")
    return 0
}
```

```output
hello klang
```

Realistic: strings are double-quoted with `\n \r \t \" \\ \uXXXX` escapes.

```klang run
fn main() -> i32 {
    let s = "a\nb"
    print(len(s))
    print(s.split("\n"))
    return 0
}
```

```output
3
[a, b]
```

Rules: `Str + anything` concats via rendering; `s[i]` is char-based; `len(s)` is bytes (see below).

## Types — arrays

Minimal: literals, index, element assign.

```klang run
fn main() -> i32 {
    let a = [10, 20]
    print(a[0])
    a[0] = 99
    print(a)
    return 0
}
```

```output
10
[99, 20]
```

Realistic: `push`/`pop`/`insert` mutate a variable and return length/elem/length.

```klang run
fn main() -> i32 {
    let a = [1, 2]
    print(push(a, 3))
    print(a)
    print(pop(a))
    print(a)
    return 0
}
```

```output
3
[1, 2, 3]
3
[1, 2]
```

Rules: index is 0-based; negative or OOB is `E-RUNTIME`; mutating a temporary (`[1].push(2)`) is `E-TYPE`.

## Types — maps

Minimal: `{"k": v}`, lookup, upsert.

```klang run
fn main() -> i32 {
    let m = {"a": 1}
    print(m["a"])
    m["b"] = 2
    print(len(m))
    return 0
}
```

```output
1
2
```

Realistic: missing keys fail at run time; int index gives i-th key.

```klang run
fn main() -> i32 {
    let m = {"a": 10, "b": 20}
    print(m[0])
    print(keys(m))
    print(m.contains("a"))
    return 0
}
```

```output
a
[a, b]
1
```

Rules: `m[k]` lookup, `m[k] = v` upsert; missing key is `E-RUNTIME`; `m.field` is `E-TYPE` (maps are not structs).

## Types — structs

Minimal: declare, construct with all fields, read.

```klang run
struct Point {
    x: i32,
    y: i32
}

fn main() -> i32 {
    let p = Point {
        x: 3,
        y: 4
    }
    print(p.x + p.y)
    return 0
}
```

```output
7
```

Realistic: fields are mutable; args are pass-by-value.

```klang run
struct Point {
    x: i32,
    y: i32
}

fn main() -> i32 {
    let p = Point {
        x: 1,
        y: 2
    }
    p.y = 10
    print(p.y)
    return 0
}
```

```output
10
```

Rules: literal names every field (`E-ARITY`/`E-UNDEFINED`/`E-TYPE` on mismatch); `p.f` on missing field is `E-UNDEFINED`; mutating a param never affects the caller.

## Types — enums

Minimal: declare, construct, match exhaustively.

```klang run
enum Opt {
    Some(x: i32),
    None
}

fn main() -> i32 {
    let v = Opt::Some(5)
    print(match v {
        Opt::Some(n) => n,
        Opt::None => 0
    })
    return 0
}
```

```output
5
```

Realistic: bare variants take `()`; guards and `_` wildcard work.

```klang run
enum Opt {
    Some(x: i32),
    None
}

fn classify(v: Opt) -> str {
    return match v {
        Opt::Some(n) if n > 10 => "big",
        Opt::Some(n) => "small",
        Opt::None => "none"
    }
}

fn main() -> i32 {
    print(classify(Opt::Some(20)))
    print(classify(Opt::None()))
    return 0
}
```

```output
big
none
```

Rules: arms comma-separated, exhaustive or `_` (`E-MATCH-EXHAUSTIVE`); duplicates (`E-MATCH-DUPLICATE`); after-`_` (`E-MATCH-UNREACHABLE`); binding count exact (`E-ARITY`).

## Types — unknown/optional values

Minimal: map lookups are `unknown` (never `E-TYPE` statically).

```klang run
fn main() -> i32 {
    let m = {"a": 1}
    let v = m["a"]
    print(v + 1)
    return 0
}
```

```output
2
```

Realistic: no `None`/`null`; use an enum or sentinel plus `is_eof()` for stdin.

```klang run
enum Maybe {
    Just(x: i32),
    Empty
}

fn main() -> i32 {
    let v = Maybe::Just(7)
    print(match v {
        Maybe::Just(n) => n,
        Maybe::Empty => 0 - 1
    })
    return 0
}
```

```output
7
```

Rules: `unknown` (map/array-dynamic/generic positions) skips static checks; missing map key is still `E-RUNTIME`; there is no null — unset env is `""`.

## Operators

Minimal: int division truncates; `%` is ints only.

```klang run
fn main() -> i32 {
    print(7 / 2)
    print(7 % 3)
    print(7.0 / 2.0)
    return 0
}
```

```output
3
1
3.5
```

Realistic: `&&`/`||` short-circuit and return 0/1 (not the operand).

```klang run
fn main() -> i32 {
    let a = [1]
    if 1 == 1 || a[5] == 1 {
        print(1)
    }
    print(1 && 2)
    print(0 || 5)
    return 0
}
```

```output
1
1
1
```

Rules: `+` is concat if either side is `Str`; `- * /` numbers only; `%` ints only; `==` same-type or int/float mix; `<` numbers or `Str`/`Str`; `&& || !` are `Bool`/`Int` in, `Bool` out, short-circuit.

## Control flow — if/else

Minimal: braces required, `else` pairs with `if`.

```klang run
fn sign(n: i32) -> str {
    if n > 0 {
        return "pos"
    } else {
        return "neg"
    }
}

fn main() -> i32 {
    print(sign(5))
    print(sign(0 - 3))
    return 0
}
```

```output
pos
neg
```

Realistic: conditions are `Bool`/`Int`; use explicit comparisons, not truthy strings.

```klang run
fn main() -> i32 {
    if len([1]) == 1 {
        print(1)
    } else {
        print(0)
    }
    return 0
}
```

```output
1
```

Rules: `if cond { } else { }`; no `elif`/`:`/`and` keywords; `if "hi"` is `E-TYPE`.

## Control flow — while

Minimal: `while` with `break`.

```klang run
fn main() -> i32 {
    let i = 0
    while i < 3 {
        i = i + 1
    }
    print(i)
    return 0
}
```

```output
3
```

Realistic: `continue` skips; `break` exits.

```klang run
fn main() -> i32 {
    let total = 0
    let i = 0
    while i < 10 {
        i = i + 1
        if i % 2 == 0 {
            continue
        }
        if i > 7 {
            break
        }
        total = total + i
    }
    print(total)
    return 0
}
```

```output
16
```

Rules: `while cond { }`; `break`/`continue` loop-only (`E-LOOP` outside).

## Control flow — for

Minimal: `..` range is exclusive upper.

```klang run
fn main() -> i32 {
    for i in 0..3 {
        print(i)
    }
    return 0
}
```

```output
0
1
2
```

Realistic: `for x in xs` works for array/map-keys/string-chars.

```klang run
fn main() -> i32 {
    for x in [10, 20] {
        print(x)
    }
    let m = {"a": 1, "b": 2}
    for k in m {
        print(k)
    }
    return 0
}
```

```output
10
20
a
b
```

Rules: `for i in a..b` needs ints; `for x in it` needs array/map/str; map yields keys; no `in` operator, no `for-else`.

## Control flow — break/continue, match patterns

Minimal: match on enum with wildcard.

```klang run
enum E {
    A,
    B
}

fn main() -> i32 {
    let v = E::A()
    print(match v {
        E::A => 1,
        _ => 0
    })
    return 0
}
```

```output
1
```

Realistic: nested patterns and guards fall through on mismatch/false.

```klang run
enum Shape {
    Circle(r: i32),
    Rect(w: i32),
    Point
}

fn area(s: Shape) -> i32 {
    return match s {
        Shape::Circle(r) => r * 3,
        Shape::Rect(w) => w * 2,
        Shape::Point => 0
    }
}

fn main() -> i32 {
    print(area(Shape::Rect(5)))
    return 0
}
```

```output
10
```

Rules: `break`/`continue` cannot cross `match`/`try` boundaries (`E-LOOP`); match arms are comma-separated expressions or `{ }` blocks.

## Functions (params, return, void main exit codes)

Minimal: exact arity and typed params.

```klang run
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    print(add(20, 22))
    return 0
}
```

```output
42
```

Realistic: omitted `->` means `void` (always exits 0); declared `Int` maps to exit code (wraps 8-bit).

```klang run
fn helper() -> void {
    print(7)
}

fn main() -> i32 {
    helper()
    return 0
}
```

```output
7
```

Rules: arity exact (`E-ARITY`); arg/return types checked (`E-TYPE`); `main -> i32` return is exit code (256 wraps to 0); `fn main()` with no arrow always exits 0; `exit(n)` exits immediately (`E-EXIT`, uncatchable).

## Closures

Minimal: literal captures by value.

```klang run
fn apply(f: fn(i32) -> i32, x: i32) -> i32 {
    return f(x)
}

fn main() -> i32 {
    let add1 = fn(x: i32) -> i32 {
        return x + 1
    }
    print(apply(add1, 41))
    return 0
}
```

```output
42
```

Realistic: captures snapshot at creation; later mutation does not affect the closure.

```klang run
fn apply(f: fn(i32) -> i32, x: i32) -> i32 {
    return f(x)
}

fn main() -> i32 {
    let n = 20
    let addn = fn(x: i32) -> i32 {
        return x + n
    }
    n = 999
    print(addn(2))
    return 0
}
```

```output
22
```

Rules: `fn(params) [-> ty] { }` value; by-value capture; call via binding; `spawn` of a closure is `E-TYPE`.

## Generics

Minimal: `fn f<T>` with per-site inference.

```klang run
fn same<T>(a: T, b: T) -> T {
    return a
}

fn main() -> i32 {
    print(same(1, 2))
    return 0
}
```

```output
1
```

Realistic: generic structs hold any `K`/`V`; conflicting sites are `E-TYPE`.

```klang run
struct Entry<K, V> {
    key: K,
    val: V
}

fn main() -> i32 {
    let e1 = Entry {
        key: "a",
        val: 1
    }
    let e2 = Entry {
        key: 2,
        val: "b"
    }
    print(e1.val + e2.key)
    return 0
}
```

```output
3
```

Rules: `fn f<T>`, `struct S<T>`, `enum E<T>`; no bounds; unconstrained becomes `unknown`; no trait system.

## Modules and imports

Minimal: `mod` with `pub` and `m::item`.

```klang run
mod shape {
    pub fn area(w: i32) -> i32 {
        return w * 2
    }
}

fn main() -> i32 {
    print(shape::area(21))
    return 0
}
```

```output
42
```

Realistic: cross-module enum plus function.

```klang run
mod shape {
    pub enum Kind {
        Circle(r: i32),
        Rect(w: i32)
    }
    pub fn area(k: Kind) -> i32 {
        return match k {
            Kind::Circle(r) => r * 3,
            Kind::Rect(w) => w * 2
        }
    }
}

fn main() -> i32 {
    print(shape::area(shape::Kind::Rect(21)))
    return 0
}
```

```output
42
```

Rules: `mod name { }`; `pub` needed for cross-module use (`E-PRIVATE` otherwise); `import "file.klang"` merges; `import "file.klang" as m` gives `m.fn()`; bare `import "pkg"` resolves via `klang.toml`+lock (local file wins with a stderr note); failures are `E-IO-NOT-FOUND`.

File imports (the helper file defines `fn double(x: i32) -> i32 { return x + x }`):

```klang run
import "helper.klang"

fn main() -> i32 {
    print(double(21))
    return 0
}
```

```output
42
```

```klang run
import "helper.klang" as h

fn main() -> i32 {
    print(h.double(21))
    return 0
}
```

```output
42
```

```klang error E-IO-NOT-FOUND
import "nope_pkg_xyz_123"

fn main() -> i32 {
    return 0
}
```

## Error handling (throws, try/catch, codes)

Minimal: `try`/`catch` binds `{code, message}` map.

```klang run
fn main() -> i32 {
    let n = 0
    try {
        n = parse_int("abc")
    } catch e {
        n = 99
    }
    print(n)
    return 0
}
```

```output
99
```

Realistic: `throws` propagates; caller needs `throws`; `e["code"]` names the failure.

```klang run
fn boom() -> i32 throws {
    return 7
}

fn main() -> i32 throws {
    print(boom())
    return 0
}
```

```output
7
```

```klang run
fn main() -> i32 {
    let n = 0
    try {
        n = parse_int("abc")
    } catch e {
        print(e["code"])
        n = 0 - 1
    }
    print(n)
    return 0
}
```

```output
E-PARSE-INT
-1
```

Rules: no `throw` statement; calling `throws` fn needs `throws` on caller (`E-EFFECT-MISMATCH`); `try { } catch e { }`; cancellation never caught.

## Strict mode

Minimal: default allows `Int` conditions.

```klang run
fn main() -> i32 {
    let x = 1
    if x == 1 {
        print(1)
    }
    return 0
}
```

```output
1
```

Realistic: `--strict` (or `strict = true` under `[project]` in `klang.toml`) narrows to `Bool` (dynamic `unknown` still ok).

```klang silent
fn main() -> i32 {
    let x = 1
    if x == 1 {
        print(1)
    }
    print(x && 1)
    return 0
}
```

Rules: default `if`/`while`/`&&`/`||`/`!` take `Bool`/`Int`; strict needs `Bool` (statically-known non-`Bool` is `E-TYPE`).

## String/array/map operations (len is bytes)

Minimal: `len()` counts bytes, not chars.

```klang run
fn main() -> i32 {
    print(len("é"))
    print("héllo"[1])
    return 0
}
```

```output
2
é
```

Realistic: for ASCII `0..len(s)` works; for non-ASCII use `chars()`.

```klang run
fn main() -> i32 {
    print("a,b,c".split(","))
    print("aaa".replace("a", "b"))
    print(["a", "b"].join(", "))
    print(sort([3, 1, 2]))
    print(slice("hello", 1, 4))
    return 0
}
```

```output
[a, b, c]
bbb
a, b
[1, 2, 3]
ell
```

Rules: `len(str)` bytes, `len(array)` elems, `len(map)` entries; `s[i]`/`chars()` char-based; `slice(x, lo, hi)` end-exclusive; `sort` returns a copy.

## File/process/env/time builtins

Minimal: env absence is `""`, never null.

```klang run
fn main() -> i32 {
    print(env("KLANG_NOPE_XYZ") == "")
    print(time_now() > 0.0)
    return 0
}
```

```output
1
1
```

Realistic: argv-array spawn, no shell; plain `run` allows relative `..`, sandbox denies with `E-SANDBOX-*`.

```klang run
fn main() -> i32 {
    let r = spawn("echo", ["hi"])
    print(r["status"])
    write_file("core_tmp.txt", "hi")
    print(read_file("core_tmp.txt"))
    remove_file("core_tmp.txt")
    return 0
}
```

```output
0
hi
```

Rules: only `klang run --sandbox` limits files (absolute only under temp dir, relative with no `..`; denials are `E-SANDBOX-FILE`). Plain `klang run` allows all relative and absolute paths resolved against CWD (so `chdir` + relative reads work), with OS failures as `E-IO-*`. Temp-dir files work in both modes. `run --sandbox` also denies `spawn`/`run`/`run_process` (`E-SANDBOX-SPAWN`) and `http_*` (`E-SANDBOX-NETWORK`); `task_group`/`spawn f()` (structured concurrency) still runs under sandbox. Missing file is `E-IO-NOT-FOUND`. `spawn`/`run_process`/`run` take `(cmd, args_array)`, no shell; `time_sleep`/`sleep_ms` bad durations are `E-TIME-INVALID`.

## Stdin, args, exit codes, formatting, comments

Minimal: `read_line()` is `""` at EOF; `args()` excludes prog name.

```klang run
fn main() -> i32 {
    print(read_line() == "")
    print(len(args()) == 0)
    return 0
}
```

```output
1
1
```

Realistic: `format(...)` joins with spaces; `sprintf(fmt, array)` does `{}`; `//` and `/* */` comments.

```klang run
fn main() -> i32 {
    // line comment
    /* block comment */
    print(format("age:", 42))
    print(sprintf("{} + {}", [1, 2]))
    print(parse_int("42"))
    return 0
}
```

```output
age: 42
1 + 2
42
```

Rules: `print(x)` one arg plus newline; `print_raw(x)` no newline (returns 1); `exit(n)` exits now; `main -> i32` return wraps 8-bit; `;` trailing is ignored; statements end at newline.

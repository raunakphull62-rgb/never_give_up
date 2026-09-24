# Language tour

A walkthrough of Klang by example. Every sample is a complete program
in the state shown — check or run it as-is. (The precise rules behind
each construct live in the [reference](reference.md).)

## Variables and functions

```klang
// @run prints: 42 | 42 ; return: 42
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    let x = add(20, 22)
    print(x)
    x = x + 1
    print(x - 1)
    return x - 1
}
```

`let` binds; plain `=` rebinds an already-bound name. Re-`let` of the
same name shadows (the new binding wins, with its own type).
Parameters are comma-separated `name: type`; the return type follows
`->`. Statements do not need trailing semicolons (a trailing `;` is
accepted and ignored).

## Control flow and loops

```klang
// @run prints: 3 | 55 | 6 | 7 ; return: 42
fn grade(s: i32) -> i32 {
    if s >= 90 {
        return 4
    } else if s >= 80 {
        return 3
    } else if s >= 70 {
        return 2
    } else {
        return 0
    }
}

fn main() -> i32 {
    print(grade(85))
    let total = 0
    let i = 1
    while i <= 10 {
        total = total + i
        i = i + 1
    }
    print(total)
    let s = 0
    for x in [1, 2, 3] {
        s = s + x
    }
    print(s)
    let t = 0
    for i in 0..5 {
        if i == 3 {
            continue
        }
        t = t + i
    }
    print(t)
    return total - 13
}
```

`if` conditions accept `bool` or `int`. `for i in a..b` needs integer
bounds (`i` is `i32`); `for x in xs` iterates arrays and strings
(element type is dynamic). `break`/`continue` work inside loops.

## Structs, arrays, maps, strings

```klang
// @run prints: 42 | 4 | yes ; return: 42
struct Point { x: i32, y: i32 }

fn main() -> i32 {
    let p = Point { x: 19, y: 20 }
    p.y = 23
    print(p.x + p.y)
    let nums = [1, 2, 3]
    push(nums, 4)
    print(len(nums))
    let cfg = {"host": "local", "port": 8080}
    cfg["port"] = 9090
    if cfg["host"] == "local" {
        print("yes")
    }
    return p.x + p.y
}
```

Struct literals must provide **every** declared field (a missing field
is an `E-ARITY` error, not a default). `push`/`pop` mutate a variable
in place — calling them on a temporary is an error. Map access with a
missing key fails at run time, so guard dynamic keys before use.

Strings support indexing (`s[i]` is character-based), byte-length
`len(s)`, and methods (`upper`, `lower`, `trim`, `split`, `contains`,
`starts_with`, `ends_with`, `replace`, `chars`, `len`):

```klang
// @run prints: HELLO WORLD | 5 | 2 ; return: 0
fn main() -> i32 {
    let words = "hello world".split(" ")
    print(words.join(" ").upper())
    print(len("klang"))
    print(len("é"))
    return 0
}
```

`len("é")` is 2 because `len` counts bytes (Rust `String::len`
convention) while `s[i]` and `s.chars()` are character-based — for
non-ASCII text, map character indices to byte offsets instead of
assuming `0..len(s)` lines up.

## Enums and match

```klang
// @run prints: 30 | 7 ; return: 0
enum Opt { Some(x: i32), None }

fn pick(v: Opt) -> i32 {
    return match v { Opt::Some(n) => n + 1, Opt::None => 7 }
}

fn main() -> i32 {
    print(pick(Opt::Some(29)))
    print(pick(Opt::None()))
    return 0
}
```

Variants carry tuple-style payloads (`Some(x: i32)`) or nothing
(bare variants still construct with `()`, as in `Opt::None()`).
`match` arms are comma-separated `Enum::Variant(bindings) => expr`;
`_ => ...` is the wildcard. Matches must be exhaustive — leaving a
variant uncovered with no wildcard is an `E-MATCH-EXHAUSTIVE` error
that names the missing variant. Payload bindings are typed and flow
into the arm body.

## Generics

```klang
// @run prints: yo | hi | 2 ; return: 42
fn identity<T>(x: T) -> T {
    return x
}

struct Box<T> { value: T }

enum Res<T> { Ok(v: T), Err(code: i32) }

fn expect<T>(r: Res, dflt: T) -> T {
    return match r { Res::Ok(v) => v, Res::Err(c) => dflt }
}

fn main() -> i32 {
    print(identity("yo"))
    let a = Box { value: 41 }
    let b = Box { value: "hi" }
    print(b.value)
    print(expect(Res::Ok(1), 0) + expect(Res::Err(9), 1))
    return a.value + 1
}
```

Type arguments are inferred per call/construction site from the
arguments, with optional explicit instantiation: annotations take
`<...>` (`Opt<i32>`, `a::Box<i32>`) and call sites take `<...>` before
the argument list (`count<T>(n)`, `count<i32>(5)`). A bare `<` without
the full generic shape stays a comparison, so `a < b` is unaffected.
`Box<i32>`
and `Box<str>` coexist without sharing state. Conflicting inference
in one site (`same(1, "hi")` for `fn same<T>(a: T, b: T)`) is an
`E-TYPE` error. There are no trait bounds.

## Modules

```klang
// @run prints: 42 ; return: 42
mod lexer {
    pub struct Token { kind: i32 }

    pub fn scan(x: i32) -> i32 {
        return x * 2
    }

    fn helper() -> i32 {
        return 1
    }
}

fn main() -> i32 {
    let t = lexer::Token { kind: 21 }
    print(lexer::scan(t.kind))
    return lexer::scan(t.kind)
}
```

Items are private by default; `pub` exposes them to qualified
`module::item` paths. Calling a private item from another module is
an `E-PRIVATE` error:

```klang
// @check-fail E-PRIVATE
mod lexer {
    fn helper() -> i32 {
        return 1
    }
}

fn main() -> i32 {
    return lexer::helper()
}
```

Same-named private items in different modules do not collide, and a
file can freely mix top-level declarations with `mod` blocks.
Splitting across files uses `import "other.klang"` (relative paths
only — absolute paths and `..` escapes are rejected); the entry file
is loaded with `klang check app.klang` and imports merge automatically.

## Effects: throws, async, cancel

A function that calls a `throws` function must itself declare
`throws` — the compiler rejects the chain otherwise:

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
// @run prints: 42 ; return: 42
fn boom() -> i32 throws {
    return 21
}

fn main() -> i32 throws {
    print(boom() * 2)
    return boom() * 2
}
```

Structured concurrency pairs `async` with `task_group`/`spawn`/`await`:

```klang
// @run prints: 42 ; return: 42
fn fetch(n: i32) -> i32 {
    return n * 2
}

fn main() -> i32 async {
    let total = 0
    task_group {
        let a = spawn fetch(20)
        let b = spawn fetch(1)
        total = await a + await b
    }
    print(total)
    return total
}
```

The rules, briefly: `spawn` only inside `task_group` (and only as a
direct `let h = spawn f()` binding); every handle must be `await`ed
before its group ends or it is an `E-TASK-CANCEL` error; using
`task_group`/`spawn`/`await` requires `async` on the function. `await`
of an unknown handle is `E-UNDEFINED`. The `cancel` effect is accepted
in signatures but, unlike `throws`, is not propagation-checked —
callers of a `cancel` function need not declare it.

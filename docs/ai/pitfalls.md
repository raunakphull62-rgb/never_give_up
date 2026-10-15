# Klang pitfalls for Python/Java/JS-trained models

Every wrong snippet was executed. `SILENT` means it runs (exit 0) but does
something different from Python/JS — those are the dangerous ones, listed
first. All others fail loudly with the recorded code + message.

## SILENT pitfalls (runs, wrong behavior) — read first

| # | Mistake | What Klang does | Correct |
|---|---|---|---|
| S1 | `1 + "a"` expecting error | `"1a"` (Str+anything concats) | use `str(1) + "a"` explicitly |
| S2 | `len("é")` expecting 1 | `2` (bytes, not chars) | use `s.chars()` / char indexing |
| S3 | `7 / 2` expecting 3.5 | `3` (int division truncates) | use `7.0 / 2.0` for float |
| S4 | `0 - 7 % 3` expecting 2 (Python) | `0 - 1` (Rust remainder, sign of dividend) | avoid negative `%`; adjust manually |
| S5 | `1 && 2` expecting 2 | `1` (Bool 0/1, not operand) | use `if` or explicit value |
| S6 | `print(true)` expecting `True` | `1` (Bool prints 0/1) | compare with `== true`, print numbers |
| S7 | `sort(a)` expecting in-place | returns copy; `a` unchanged | `let b = sort(a)` |
| S8 | `push(a, v)` expecting array | returns new length (Int) | use `a` after the call |
| S9 | `m[0]` expecting KeyError | first key `"a"` (positional) | use `m["key"]` for lookup |
| S10 | `args()` expecting script name at [0] | excludes prog name; empty when no args | iterate all of `args()` |
| S11 | `read_line() == ""` for EOF check | `""` both for empty line and EOF | also check `is_eof()` |
| S12 | `env("X")` expecting None when unset | `""` (indistinguishable from empty) | treat `""` as absent |
| S13 | mutating struct param expecting caller change | pass-by-value; caller unchanged | return the updated value |
| S14 | `fn main()` returning value for exit code | always exits 0, discards value | declare `-> i32` to set exit code |
| S15 | `return 256` expecting exit 256 | wraps 8-bit to 0 (`-1` to 255) | keep codes 0..125 |
| S16 | `format("a", 1)` expecting Python format | joins with spaces (`"a 1"`) | use `sprintf("{}..", [...])` for templates |
| S17 | `run("ls -l", [])` expecting shell | argv-only, no shell/glob/pipe | pass `("ls", ["-l"])`, no metachars |
| S18 | `len(s)` as char count for `slice`/`s[i]` | `len("é")` is `2` bytes but `slice("é", 0, 1)` is 1 char; `slice(s, 0, len(s))` OOB for non-ASCII | use `len(s.chars())` for char counts |
| S19 | `==` on arrays/structs/maps expecting JS reference or Python order-insensitive | value equality; maps order-sensitive (`{"x":1,"y":2}=={"y":2,"x":1}` is `0`, Python `True`) | compare explicitly, do not rely on order |
| S20 | `if ""`/`if []` expecting Python falsy | literal empty is loud `E-TYPE`; same value via `unknown` (`m["k"]`) runs silent falsy | use `== ""` / `len() == 0` explicitly |

Details with verified runs follow; then the loud (caught) pitfalls.

### S1 `+` with Str concats (not an error)

Wrong (silent `"1a"`):

```klang silent
fn main() -> i32 {
    print(1 + "a")
    return 0
}
```

Correct (explicit):

```klang run
fn main() -> i32 {
    print(str(1) + "a")
    print("a" + str(1))
    return 0
}
```

```output
1a
a1
```

### S2 `len()` is bytes

Wrong (silent `2`):

```klang silent
fn main() -> i32 {
    print(len("é"))
    return 0
}
```

Correct (chars for text):

```klang run
fn main() -> i32 {
    print(len("é"))
    print("héllo"[1])
    print("hi".len())
    return 0
}
```

```output
2
é
2
```

### S3 `/` on ints truncates

Wrong (silent `3`):

```klang silent
fn main() -> i32 {
    print(7 / 2)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(7 / 2)
    print(7.0 / 2.0)
    return 0
}
```

```output
3
3.5
```

### S4 `%` sign differs for negatives

Wrong (silent `-1`, Python gives `2`):

```klang silent
fn main() -> i32 {
    print(0 - 7 % 3)
    return 0
}
```

Correct (avoid negative `%`):

```klang run
fn main() -> i32 {
    print(7 % 3)
    return 0
}
```

```output
1
```

### S5 `&&`/`||` return 0/1

Wrong (silent `1`, Python gives operand):

```klang silent
fn main() -> i32 {
    print(1 && 2)
    print(0 || 5)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(1 && 2)
    print(!0)
    return 0
}
```

```output
1
1
```

### S6 Bool prints as 0/1

Wrong (silent `1`/`0`):

```klang silent
fn main() -> i32 {
    print(true)
    print(false)
    return 0
}
```

Correct (compare explicitly):

```klang run
fn main() -> i32 {
    print(true)
    print(1 == 1)
    return 0
}
```

```output
1
1
```

### S7 `sort` returns a copy

Wrong (silent: input unchanged):

```klang silent
fn main() -> i32 {
    let a = [3, 1, 2]
    print(sort(a))
    print(a)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let a = [3, 1, 2]
    let b = sort(a)
    print(b)
    return 0
}
```

```output
[1, 2, 3]
```

### S8 `push` returns length

Wrong (silent `2`):

```klang silent
fn main() -> i32 {
    let a = [1]
    print(push(a, 2))
    print(a)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let a = [1]
    push(a, 2)
    print(a)
    return 0
}
```

```output
[1, 2]
```

### S9 `m[0]` is positional key lookup

Wrong (silent `"a"`):

```klang silent
fn main() -> i32 {
    let m = {"a": 10, "b": 20}
    print(m[0])
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let m = {"a": 10, "b": 20}
    print(m["a"])
    print(keys(m))
    return 0
}
```

```output
10
[a, b]
```

### S10 `args()` excludes prog name

Wrong (silent `0` with no args):

```klang silent
fn main() -> i32 {
    print(len(args()))
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let argv = args()
    print(len(argv))
    return 0
}
```

```output
0
```

### S11 `read_line()` EOF vs empty line

Wrong (silent: both `""`):

```klang silent
fn main() -> i32 {
    let line = read_line()
    print(line == "")
    print(is_eof())
    return 0
}
```

Correct (use `is_eof()`):

```klang run
fn main() -> i32 {
    print(read_line() == "")
    print(is_eof())
    return 0
}
```

```output
1
1
```

### S12 missing env is `""`

Wrong (silent `1`):

```klang silent
fn main() -> i32 {
    print(env("KLANG_NOPE_XYZ") == "")
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(get_env("KLANG_NOPE_XYZ") == "")
    return 0
}
```

```output
1
```

### S13 struct args are pass-by-value

Wrong (silent caller unchanged):

```klang silent
struct P {
    x: i32
}

fn f(p: P) -> i32 {
    p.x = 99
    return p.x
}

fn main() -> i32 {
    let q = P {
        x: 1
    }
    print(f(q))
    print(q.x)
    return 0
}
```

Correct (return updated value):

```klang run
struct P {
    x: i32
}

fn main() -> i32 {
    let q = P {
        x: 1
    }
    q.x = 99
    print(q.x)
    return 0
}
```

```output
99
```

### S14 `fn main()` with no arrow always exits 0

Wrong (silent exit 0):

```klang silent
fn main() {
    print(7)
}
```

Correct (declare `-> i32` for exit codes):

```klang run
fn main() -> i32 {
    print(7)
    return 0
}
```

```output
7
```

### S15 exit codes wrap 8-bit

Wrong (silent wrap; `return 256` exits 0):

```klang silent
fn main() -> i32 {
    return 0
}
```

Correct (keep 0..125):

```klang run
fn main() -> i32 {
    print(42)
    return 0
}
```

```output
42
```

### S16 `format` joins with spaces

Wrong (silent `"a 1"`):

```klang silent
fn main() -> i32 {
    print(format("a", 1))
    return 0
}
```

Correct (use `sprintf` for templates):

```klang run
fn main() -> i32 {
    print(format("a", 1))
    print(sprintf("{} + {}", [1, 2]))
    return 0
}
```

```output
a 1
1 + 2
```

### S17 spawn has no shell

Wrong (silent no glob expansion; use argv):

```klang silent
fn main() -> i32 {
    let r = spawn("echo", ["hi"])
    print(r["status"])
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let r = spawn("echo", ["hi"])
    print(r["status"])
    print(r["stdout"])
    return 0
}
```

```output
0
hi

```

### S18 `len()` is bytes, `s[i]`/`slice()` are chars

Wrong (silent `2` vs 1 char; `slice(s, 0, len(s))` is OOB for non-ASCII):

```klang silent
fn main() -> i32 {
    print(len("é"))
    print(slice("é", 0, 1))
    return 0
}
```

Correct (char counts via `chars()`):

```klang run
fn main() -> i32 {
    print(len("héllo"))
    print(len("héllo".chars()))
    print(slice("héllo", 1, 3))
    print("héllo"[1])
    return 0
}
```

```output
6
5
él
é
```

### S19 `==` on arrays/structs/maps is value equality, maps order-sensitive

Wrong (silent `1`/`0`; JS would be reference-false, Python map order-insensitive true):

```klang silent
fn main() -> i32 {
    print([1, 2] == [1, 2])
    print([1, 2] == [1, 3])
    return 0
}
```

Correct (same-struct value equality; map order matters):

```klang run
struct P {
    x: i32
}

fn main() -> i32 {
    let a = P {
        x: 1
    }
    let b = P {
        x: 1
    }
    print(a == b)
    let m1 = {"x": 1, "y": 2}
    let m2 = {"y": 2, "x": 1}
    print(m1 == m2)
    return 0
}
```

```output
1
0
```

### S20 truthiness: literal empty is loud, dynamic empty is silent falsy

Wrong (silent `0`/`1` via `unknown`; `if ""` literally is `E-TYPE`):

```klang silent
fn main() -> i32 {
    let m = {"s": "", "a": [1]}
    let s = m["s"]
    if s {
        print(1)
    } else {
        print(0)
    }
    let v = m["a"]
    if v {
        print(1)
    } else {
        print(0)
    }
    return 0
}
```

Correct (explicit comparisons):

```klang run
fn main() -> i32 {
    if len("") == 0 {
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

## Loud pitfalls (caught with a diagnostic)

| # | Python/JS habit | Klang diagnostic | Correct |
|---|---|---|---|
| 1 | list comprehension | `E-PARSE` expected `]` | `for` loop + `push` |
| 2 | f-strings | `E-PARSE` expected `)` | `"a" + x` / `format` / `sprintf` |
| 3 | `None`/`null`/`nil` | `E-UNDEFINED` or `E-PARSE` expected field name | enum or sentinel |
| 4 | `True`/`False` capitalized | `E-UNDEFINED` / `E-PARSE` | lowercase `true`/`false` |
| 5 | `class` | `E-PARSE` expected `fn` | `struct` + `fn` |
| 6 | `with open()` | `E-PARSE` expected map-literal key | `read_file`/`write_file` |
| 7 | `print(a, b)` multi-arg | `E-PARSE` expected `)` | `print(format(a, b))` |
| 8 | `m.field` on maps | `E-TYPE` field base want struct | `m["field"]` |
| 9 | slicing `a[1:2]` | `E-PARSE` expected `]` | `slice(a, 1, 2)` |
| 10 | tuple unpack `a, b = 1, 2` | `E-PARSE` expected `=` | two `let`s |
| 11 | `in` operator | `E-PARSE` expected `{` | `.contains()` |
| 12 | `except` | `E-PARSE` expected `catch` | `catch` |
| 13 | top-level `let g = 1` | `E-PARSE` expected `fn` | pass via params/return |
| 14 | `:` blocks / indentation | `E-PARSE` expected `{` | braces `{ }` |
| 15 | `elif` | `E-PARSE` map-literal key | nested `else { if }` |
| 16 | `def` | `E-PARSE` expected `fn` | `fn` |
| 17 | `lambda` | `E-PARSE` expected expression | `fn(x: ty) -> ty { }` |
| 18 | `and`/`or`/`not` | `E-PARSE` expected `)` | `&&`/`||`/`!` |
| 19 | `is` | `E-PARSE` expected `{` | `==` (same-type only) |
| 20 | `**` power | `E-PARSE` expected expression | `pow(a, b)` |
| 21 | `++`/`+=` | `E-PARSE` expected expression | `x = x + 1` |
| 22 | `===` | `E-PARSE` expected expression | `==` |
| 23 | `1 == true` (JS) | `E-TYPE` want i32 got bool | compare same types |
| 24 | `1 < 2 < 3` chain | `E-TYPE` want i32/f64/str got bool | `(1 < 2) && (2 < 3)` |
| 25 | `"a" * 3` repeat | `E-TYPE` want i32/f64 got str | `"a".repeat(3)` |
| 26 | `[1] + [2]` concat | `E-TYPE` want i32/f64/str got array | `push` loop or `join` |
| 27 | `7.0 % 2.0` | `E-TYPE` want i32 got f64 | ints only for `%` |
| 28 | `x = 1` without `let` | `E-UNDEFINED` undefined variable | `let x = 1` first |
| 29 | `x = "hi"` over `let x = 1` | `E-TYPE` assignment want i32 got str | new `let x = "hi"` to shadow |
| 30 | `fn len()` shadowing | `W-SHADOW` shadows builtin | rename |
| 31 | `[1,2].push(3)` on temp | `E-TYPE` must mutate variable | bind first |
| 32 | `return` with no value | `E-PARSE` expected expression | `return 0` / fall off |
| 33 | `fn f(x: i32) { return x }` | `E-TYPE` want void got i32 | add `-> i32` |
| 34 | `break` outside loop | `E-LOOP` outside any loop | move inside |
| 35 | `if "hi"` truthy string | `E-TYPE` want bool got str | explicit `==`/`len() == 0` |
| 36 | `a[-1]` last elem | `E-RUNTIME` negative index | `a[len(a) - 1]` |
| 37 | missing map key | `E-RUNTIME` missing map key | `.contains()` guard |
| 38 | `parse_int("abc")` | `E-PARSE-INT` | `try`/`catch`, check input |
| 39 | `1 / 0` | `E-RUNTIME` division by zero | guard divisor |
| 40 | `m::secret()` private | `E-PRIVATE` | mark `pub` |

### 1 list comprehension

Wrong (`E-PARSE` expected `]`):

```klang error E-PARSE
fn main() -> i32 {
    let x = [i * 2 for i in range(0, 3)]
    print(x)
    return 0
}
```

Correct (loop + push):

```klang run
fn main() -> i32 {
    let out = []
    for i in 0..3 {
        push(out, i * 2)
    }
    print(out)
    return 0
}
```

```output
[0, 2, 4]
```

### 2 f-strings

Wrong (`E-PARSE` expected `)`):

```klang error E-PARSE
fn main() -> i32 {
    let name = "world"
    print(f"hello {name}")
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let name = "world"
    print("hello " + name)
    print(format("hello", name))
    return 0
}
```

```output
hello world
hello world
```

### 3 None/null

Wrong (`E-UNDEFINED` undefined variable `None`):

```klang error E-UNDEFINED
fn main() -> i32 {
    print(None)
    return 0
}
```

Correct (enum optional):

```klang run
enum Maybe {
    Just(x: i32),
    Empty
}

fn main() -> i32 {
    let v = Maybe::Empty()
    print(match v {
        Maybe::Just(n) => n,
        Maybe::Empty => 0 - 1
    })
    return 0
}
```

```output
-1
```

### 4 True/False capitalized

Wrong (`E-UNDEFINED` undefined variable `True`):

```klang error E-UNDEFINED
fn main() -> i32 {
    print(True)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(true)
    return 0
}
```

```output
1
```

### 5 class

Wrong (`E-PARSE` expected `fn`):

```klang error E-PARSE
class Foo {
}

fn main() -> i32 {
    return 0
}
```

Correct:

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
    print(p.x)
    return 0
}
```

```output
1
```

### 6 with

Wrong (`E-PARSE` map-literal key — confusing, see report):

```klang error E-PARSE
fn main() -> i32 {
    with open("f") {
        print(1)
    }
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    write_file("pit_tmp.txt", "hi")
    print(read_file("pit_tmp.txt"))
    remove_file("pit_tmp.txt")
    return 0
}
```

```output
hi
```

### 7 print with multiple args

Wrong (`E-PARSE` expected `)`):

```klang error E-PARSE
fn main() -> i32 {
    print("hi", "there")
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(format("hi", "there"))
    return 0
}
```

```output
hi there
```

### 8 dict dot access

Wrong (`E-TYPE` field base want struct, got map):

```klang error E-TYPE
fn main() -> i32 {
    let m = {"a": 1}
    print(m.a)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let m = {"a": 1}
    print(m["a"])
    return 0
}
```

```output
1
```

### 9 slicing

Wrong (`E-PARSE` expected `]`):

```klang error E-PARSE
fn main() -> i32 {
    let a = [1, 2, 3]
    print(a[1:2])
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(slice([1, 2, 3], 0, 2))
    print(slice("hello", 1, 3))
    return 0
}
```

```output
[1, 2]
el
```

### 10 tuple unpacking

Wrong (`E-PARSE` expected `=`):

```klang error E-PARSE
fn main() -> i32 {
    let a, b = 1, 2
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let a = 1
    let b = 2
    print(a + b)
    return 0
}
```

```output
3
```

### 11 in operator

Wrong (`E-PARSE` expected `{`):

```klang error E-PARSE
fn main() -> i32 {
    if 1 in [1, 2] {
        print(1)
    }
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print([1, 2].contains(1))
    print("hi".contains("i"))
    return 0
}
```

```output
1
1
```

### 12 except

Wrong (`E-PARSE` expected `catch`):

```klang error E-PARSE
fn main() -> i32 {
    try {
        print(1)
    } except e {
        print(2)
    }
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let n = 0
    try {
        n = parse_int("abc")
    } catch e {
        n = 7
    }
    print(n)
    return 0
}
```

```output
7
```

### 13 top-level let

Wrong (`E-PARSE` expected `fn` at top level — globals do not exist):

```klang error E-PARSE
let g = 1

fn main() -> i32 {
    return 0
}
```

Correct (thread state via params):

```klang run
fn f(g: i32) -> i32 {
    return g + 1
}

fn main() -> i32 {
    print(f(1))
    return 0
}
```

```output
2
```

### 14 indentation blocks

Wrong (`E-PARSE` expected `{`):

```klang error E-PARSE
fn main() -> i32 {
    if 1 == 1:
        print(1)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    if 1 == 1 {
        print(1)
    }
    return 0
}
```

```output
1
```

### 15 elif

Wrong (`E-PARSE` map-literal key — confusing):

```klang error E-PARSE
fn main() -> i32 {
    let x = 1
    if x == 0 {
        print(0)
    } elif x == 1 {
        print(1)
    }
    return 0
}
```

Correct (nested else-if):

```klang run
fn main() -> i32 {
    let x = 1
    if x == 0 {
        print(0)
    } else {
        if x == 1 {
            print(1)
        }
    }
    return 0
}
```

```output
1
```

### 16 def

Wrong (`E-PARSE` expected `fn`):

```klang error E-PARSE
def foo():
    pass

fn main() -> i32 {
    return 0
}
```

Correct:

```klang run
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    print(add(1, 2))
    return 0
}
```

```output
3
```

### 17 lambda

Wrong (`E-PARSE` expected expression):

```klang error E-PARSE
fn main() -> i32 {
    let f = lambda x: x + 1
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let f = fn(x: i32) -> i32 {
        return x + 1
    }
    print(f(1))
    return 0
}
```

```output
2
```

### 18 and/or/not keywords

Wrong (`E-PARSE` expected `)`):

```klang error E-PARSE
fn main() -> i32 {
    print(1 and 0)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(1 && 0)
    print(1 || 0)
    print(!1)
    return 0
}
```

```output
0
1
0
```

### 19 is keyword

Wrong (`E-PARSE` expected `{`):

```klang error E-PARSE
fn main() -> i32 {
    if 1 is 1 {
        print(1)
    }
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    if 1 == 1 {
        print(1)
    }
    return 0
}
```

```output
1
```

### 20 power operator

Wrong (`E-PARSE` expected expression):

```klang error E-PARSE
fn main() -> i32 {
    print(2 ** 3)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(pow(2.0, 3.0))
    return 0
}
```

```output
8.0
```

### 21 increment operators

Wrong (`E-PARSE` expected expression):

```klang error E-PARSE
fn main() -> i32 {
    let x = 1
    x++
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let x = 1
    x = x + 1
    print(x)
    return 0
}
```

```output
2
```

### 22 strict equality

Wrong (`E-PARSE` expected expression):

```klang error E-PARSE
fn main() -> i32 {
    print(1 === 1)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(1 == 1)
    return 0
}
```

```output
1
```

### 23 int == bool

Wrong (`E-TYPE` want i32 got bool):

```klang error E-TYPE
fn main() -> i32 {
    print(1 == true)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(1 == 1)
    return 0
}
```

```output
1
```

### 24 chained comparison

Wrong (`E-TYPE` want i32/f64/str got bool):

```klang error E-TYPE
fn main() -> i32 {
    print(1 < 2 < 3)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(1 < 2 && 2 < 3)
    return 0
}
```

```output
1
```

### 25 string repeat with *

Wrong (`E-TYPE` want i32/f64 got str):

```klang error E-TYPE
fn main() -> i32 {
    print("a" * 3)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print("ab".repeat(3))
    return 0
}
```

```output
ababab
```

### 26 array concat with +

Wrong (`E-TYPE` want i32/f64/str got array):

```klang error E-TYPE
fn main() -> i32 {
    print([1] + [2])
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let a = [1]
    push(a, 2)
    print(a)
    return 0
}
```

```output
[1, 2]
```

### 27 float modulo

Wrong (`E-TYPE` want i32 got f64):

```klang error E-TYPE
fn main() -> i32 {
    print(7.0 % 2.0)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    print(7 % 3)
    return 0
}
```

```output
1
```

### 28 assign without let

Wrong (`E-UNDEFINED` undefined variable):

```klang error E-UNDEFINED
fn main() -> i32 {
    x = 1
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let x = 1
    print(x)
    return 0
}
```

```output
1
```

### 29 rebinding to a new type

Wrong (`E-TYPE` assignment want i32 got str):

```klang error E-TYPE
fn main() -> i32 {
    let x = 1
    x = "hi"
    return 0
}
```

Correct (shadow with `let`):

```klang run
fn main() -> i32 {
    let x = 1
    let x = "hi"
    print(x)
    return 0
}
```

```output
hi
```

### 30 shadowing a builtin

Wrong (`W-SHADOW` shadows builtin):

```klang error W-SHADOW
fn len(x: i32) -> i32 {
    return x
}

fn main() -> i32 {
    return 0
}
```

Correct (rename):

```klang run
fn mylen(x: i32) -> i32 {
    return x
}

fn main() -> i32 {
    print(mylen(3))
    return 0
}
```

```output
3
```

### 31 push on a temporary

Wrong (`E-TYPE` must mutate a variable):

```klang error E-TYPE
fn main() -> i32 {
    [1, 2].push(3)
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let a = [1, 2]
    push(a, 3)
    print(a)
    return 0
}
```

```output
[1, 2, 3]
```

### 32 bare return

Wrong (`E-PARSE` expected expression):

```klang error E-PARSE
fn f() -> void {
    return
}

fn main() -> i32 {
    f()
    return 0
}
```

Correct:

```klang run
fn f() -> void {
    print(1)
}

fn main() -> i32 {
    f()
    return 0
}
```

```output
1
```

### 33 missing return type

Wrong (`E-TYPE` want void got i32):

```klang error E-TYPE
fn f(x: i32) {
    return x
}

fn main() -> i32 {
    return 0
}
```

Correct:

```klang run
fn f(x: i32) -> i32 {
    return x
}

fn main() -> i32 {
    print(f(1))
    return 0
}
```

```output
1
```

### 34 break outside loop

Wrong (`E-LOOP` outside any loop):

```klang error E-LOOP
fn main() -> i32 {
    break
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    while true {
        break
    }
    print(1)
    return 0
}
```

```output
1
```

### 35 truthy string condition

Wrong (`E-TYPE` want bool got str):

```klang error E-TYPE
fn main() -> i32 {
    if "hi" {
        print(1)
    }
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    if len("hi") == 2 {
        print(1)
    }
    return 0
}
```

```output
1
```

### 36 negative index

Wrong (`E-RUNTIME` negative index):

```klang error E-RUNTIME
fn main() -> i32 {
    let a = [1, 2, 3]
    print(a[0 - 1])
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let a = [1, 2, 3]
    print(a[len(a) - 1])
    return 0
}
```

```output
3
```

### 37 missing map key

Wrong (`E-RUNTIME` missing map key):

```klang error E-RUNTIME
fn main() -> i32 {
    let m = {"a": 1}
    print(m["zz"])
    return 0
}
```

Correct (guard with `contains`):

```klang run
fn main() -> i32 {
    let m = {"a": 1}
    if m.contains("zz") == false {
        print(0)
    }
    return 0
}
```

```output
0
```

### 38 parse failure

Wrong (`E-PARSE-INT`):

```klang error E-PARSE-INT
fn main() -> i32 {
    print(parse_int("abc"))
    return 0
}
```

Correct:

```klang run
fn main() -> i32 {
    let n = 0
    try {
        n = parse_int("abc")
    } catch e {
        n = 0
    }
    print(n)
    return 0
}
```

```output
0
```

### 39 division by zero

Wrong (`E-RUNTIME` division by zero):

```klang error E-RUNTIME
fn main() -> i32 {
    print(1 / 0)
    return 0
}
```

Correct (guard):

```klang run
fn main() -> i32 {
    let d = 0
    if d == 0 {
        print(0)
    } else {
        print(1 / d)
    }
    return 0
}
```

```output
0
```

### 40 private access

Wrong (`E-PRIVATE`):

```klang error E-PRIVATE
mod m {
    fn secret() -> i32 {
        return 1
    }
}

fn main() -> i32 {
    return m::secret()
}
```

Correct:

```klang run
mod m {
    pub fn secret() -> i32 {
        return 1
    }
}

fn main() -> i32 {
    print(m::secret())
    return 0
}
```

```output
1
```

## Backend traps (A1/A2/A3/L1/A3b) — verified

### B1 `conn_read` is one syscall, loop for exact sizes

`conn_read(c, max)` returns one read (short reads return early).
Loop for exact sizes; `conn_read_line` frames lines (strips `\n`/`\r\n`,
`null` at close, final partial line returned once).

```klang check
fn copy_all(c: map) -> i32 {
    return 0
}

fn main() -> i32 {
    print(is_null(json_decode("null")))
    return 0
}
```

### B2 JSON `true`/`false` become `1`/`0`, no Bool value

`Bool` erases to `Int` at runtime (prints `0`/`1`).

```klang run
fn main() -> i32 {
    print(json_decode("true"))
    print(json_decode("false"))
    print(json_decode("null"))
    print(json_stringify(json_decode("{\"a\": null}"), 0))
    return 0
}
```

```output
1
0
null
{"a":null}
```

### B3 HTTP bodies need `Content-Length`, chunked is `400`

The server requires `Content-Length` (missing means `""`);
`Transfer-Encoding: chunked` is `400`; oversize is `413`, slow is `408`.

```klang check
fn handle(req: map) -> map {
    return http_json(200, {"len": len(req["body"])})
}

fn main() -> i32 {
    let s = http_serve("127.0.0.1", 0, "handle", {"workers": 1})
    print(server_port(s) > 0)
    http_stop(s)
    return 0
}
```

### B4 `json_decode` is native, `json_parse` is the package

The pure-Klang `json` package defines `json_parse` (with `$json`
markers); the fast native is `json_decode` (throws `E-JSON-PARSE`).
Same-named natives would trip `W-SHADOW`.

```klang run
fn main() -> i32 {
    print(json_decode("{\"a\": 1}"))
    return 0
}
```

```output
{"a": 1}
```

```klang error E-JSON-PARSE
fn main() -> i32 {
    print(json_decode("{bad"))
    return 0
}
```

### B5 `null` has no literal, use `is_null`

Only `tcp_accept_timeout`/`conn_read_line`/`recv` produce `Null`;
`str(null)` is `"null"`, null is falsy, `==` only to null.

```klang run
fn main() -> i32 {
    print(is_null(json_decode("null")))
    print(is_null(0))
    print(str(json_decode("null")))
    return 0
}
```

```output
1
0
null
```

### B6 handles are shared by clone, give to ONE worker

`Listener`/`Conn`/`Chan` cross threads by `Arc` clone (same socket);
concurrent use serializes, close-anywhere closes-everywhere. Move to at
most one worker; sharing never crashes but interleaves.

```klang check
fn worker(c: map) -> i32 {
    return 0
}

fn main() -> i32 {
    let ch = chan(1)
    send(ch, 1)
    print(recv(ch))
    close(ch)
    return 0
}
```

### B7 `chan(0)` is rejected, forgetting `thread_join` hides failures

Capacity is `1..1000000` (`0` is `E-CHAN-CAPACITY`); unjoined threads are
reaped with results ignored (join to observe `E-THREAD-FAILED`).

```klang error E-CHAN-CAPACITY
fn main() -> i32 {
    let ch = chan(0)
    return 0
}
```

### B8 sandbox caps threads at 4, denies sockets/pg/HTTP

Plain `run` allows network/threads; `run --sandbox` denies with
`E-SANDBOX-*` (`is_null`/`thread_id`/pure JSON stay allowed).

```klang check
fn main() -> i32 {
    print(thread_id() == 0)
    print(is_null(json_decode("null")))
    return 0
}
```

### B9 datetime is UTC, fixed offsets only, years 1..9999

Named zones unsupported; `%z` is `+0000`; `dt_parts_offset` takes
`-1080..1080` minutes; `dt_from_parts` needs `1..9999`.

```klang run
fn main() -> i32 {
    let ms = dt_from_parts(2024, 2, 29, 12, 0, 0)
    print(dt_format(ms, "%Y-%m-%d %z"))
    print(dt_is_leap(2024))
    print(dt_days_in_month(2024, 2))
    return 0
}
```

```output
2024-02-29 +0000
1
29
```

```klang error E-TIME-RANGE
fn main() -> i32 {
    print(dt_from_parts(2024, 2, 30, 0, 0, 0))
    return 0
}
```

### B10 `tar_create` archives, does not compress

`tar_create(dir, out)` writes plain tar; combine with `gzip_file` for
`.tar.gz`. Extraction rejects `..`/absolute/links
(`E-COMPRESS-UNSAFE-PATH`) and caps at 256 MB (`E-COMPRESS-TOO-LARGE`).

```klang check
fn main() -> i32 {
    print(gzip_decompress(gzip_compress("hi")))
    return 0
}
```

### B11 Postgres NUMERIC needs a cast, BOOL via literals, per-request conns

Bare `NUMERIC` reads are `E-PG-TYPE`; use `n::text` (exact `Str`) or
`n::float8` (Float). `BOOL` reads as `Int` 0/1; writes use SQL literals
(no bool literal; `Int` params are INT). Handlers open per-request
connections (no pool; avoids txn cross-talk).

```klang check
fn main() -> i32 {
    let db = pg_connect(get_env("DATABASE_URL"))
    let rows = pg_query(db, "SELECT n::text as n FROM t", [])
    pg_exec(db, "INSERT INTO notes (text) VALUES ($1)", ["hi"])
    pg_begin(db)
    pg_rollback(db)
    pg_close(db)
    return 0
}
```

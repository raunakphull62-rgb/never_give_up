# Klang error codes — causes, repros, fixes

Every check-time code prints JSON with `code`, `message`, `primary_span`,
`cause`, `fixes`, `rule`. Run-time codes print `run: FAIL` plus JSON on
stderr. Each snippet below was executed. Library-only and v2-only codes
have no surface Klang repro (noted); all others show minimal failing
programs.

## Check-time codes

| Code | Cause | Fix |
|---|---|---|
| `E-PARSE` | syntax does not match grammar | add missing brace/paren/keyword |
| `E-ARITY` | wrong arg/binding/payload count | match declared counts |
| `E-TYPE` | operand/arg/return/condition mistyped | convert or change annotation |
| `E-UNDEFINED` | unknown variable/fn/variant | `let` first / check spelling |
| `E-DUPLICATE` | duplicate param/field/variant/alias | rename |
| `E-LOOP` | `break`/`continue` outside loop or across `match`/`try` | move inside loop |
| `E-PRIVATE` | cross-module use of non-`pub` item | mark `pub` |
| `E-EFFECT-MISMATCH` | calling `throws`/`async` fn without effect | add `throws`/`async` to caller |
| `E-MATCH-EXHAUSTIVE` | missing variant, no `_` | add arms or `_` |
| `E-MATCH-DUPLICATE` | same variant twice | remove duplicate |
| `E-MATCH-UNREACHABLE` | arm after `_` | move before `_` or drop |
| `E-SPAWN-OUTSIDE-GROUP` | `spawn` outside `task_group` | wrap in `task_group` |
| `E-SPAWN-POSITION` | `spawn` not directly `let`-bound | `let h = spawn f()` only |
| `E-AWAIT-OUTSIDE-GROUP` | `await` outside group | await inside group |
| `E-TASK-CANCEL` | spawned handle never awaited | `await` every handle |
| `E-RESERVED-FIELD` | struct field `__*`/`f<digits>` | rename field |
| `E-IMPORT` | selective import missing name / split `mod` | check available names / whole-file import |
| `E-IO-NOT-FOUND` | missing import/file (also bare-package miss) | create file / `klang add` |
| `W-SHADOW` | `fn` reuses builtin name (builtins win) | rename |
| `E-PARSE-INT` | `parse_int` bad text/range (run-time) | check input / `try` |
| `E-PARSE-FLOAT` | `parse_float` bad text (run-time) | check input / `try` |

```klang error E-PARSE
fn main() -> i32 {
    return 1
```

```klang error E-ARITY
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    return add(1)
}
```

```klang error E-TYPE
fn main() -> i32 {
    return "hi" - 1
}
```

```klang error E-UNDEFINED
fn main() -> i32 {
    return nope + 1
}
```

```klang error E-DUPLICATE
fn f(a: i32, a: i32) -> i32 {
    return a
}

fn main() -> i32 {
    return f(1, 2)
}
```

```klang error E-LOOP
fn main() -> i32 {
    break
    return 0
}
```

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

```klang error E-EFFECT-MISMATCH
fn boom() -> i32 throws {
    return 1
}

fn main() -> i32 {
    return boom()
}
```

```klang error E-MATCH-EXHAUSTIVE
enum Opt {
    Some(x: i32),
    None
}

fn f(v: Opt) -> i32 {
    return match v {
        Opt::Some(n) => n
    }
}

fn main() -> i32 {
    return 0
}
```

```klang error E-MATCH-DUPLICATE
enum Opt {
    Some(x: i32),
    None
}

fn f(v: Opt) -> i32 {
    return match v {
        Opt::Some(n) => n,
        Opt::Some(m) => m,
        Opt::None => 0
    }
}

fn main() -> i32 {
    return 0
}
```

```klang error E-MATCH-UNREACHABLE
enum Opt {
    Some(x: i32),
    None
}

fn f(v: Opt) -> i32 {
    return match v {
        Opt::Some(n) => n,
        _ => 0,
        Opt::None => 1
    }
}

fn main() -> i32 {
    return 0
}
```

```klang error E-SPAWN-OUTSIDE-GROUP
fn f() -> i32 {
    return 1
}

fn main() -> i32 {
    let a = spawn f()
    return 0
}
```

```klang error E-SPAWN-POSITION
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

```klang error E-AWAIT-OUTSIDE-GROUP
fn main() -> i32 {
    return await b
}
```

```klang error E-RESERVED-FIELD
struct S {
    __variant: i32
}

fn main() -> i32 {
    return 0
}
```

```klang error E-TASK-CANCEL
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

```klang error W-SHADOW
fn len(x: i32) -> i32 {
    return x
}

fn main() -> i32 {
    return 0
}
```

## Run-time codes

| Code | Cause | Fix |
|---|---|---|
| `E-RUNTIME` | division by zero, bad index, missing key, guard, OOB | guard inputs |
| `E-OVERFLOW` | `i64` arithmetic past range (never wraps) | smaller operands / bounds check |
| `E-PARSE-INT` | `parse_int` non-decimal or outside `i32` | validate / `try` |
| `E-PARSE-FLOAT` | `parse_float` non-finite decimal | validate / `try` |
| `E-IO-NOT-FOUND` | missing file / bad `chdir` / missing binary dir | create path |
| `E-IO-PERMISSION` | permission denied | fix perms |
| `E-IO-FAILED` | other OS file failure | check OS error |
| `E-PROCESS-NOT-FOUND` | missing binary for `spawn`/`run` | install / fix PATH |
| `E-PROCESS-FAILED` | OS refused spawn | check limits |
| `E-REGEX-INVALID-PATTERN` | bad regex | fix pattern |
| `E-REGEX-TOO-MANY` | >10k matches in `regex_find_all` | narrower pattern |
| `E-REGEX-TOO-LARGE` | replace output >1M bytes | smaller replacement |
| `E-TIME-INVALID` | negative/NaN/inf sleep | non-negative finite duration |
| `E-NET-INVALID-URL` | bad URL | fix URL |
| `E-NET-INVALID-HEADER` | bad header entry | `["Name: value"]` strings |
| `E-NET-UNREACHABLE` | transport failure | check network/host |
| `E-CHAR-INVALID` | `ord("")` / bad `chr` | non-empty str / valid codepoint |
| `E-STR-INVALID` | empty pad / negative width/count | non-empty pad, `>= 0` |
| `E-STR-TOO-LARGE` | pad/repeat >1M chars | smaller width/count |
| `E-FMT-INVALID` | lone `{`/`}` in `sprintf` | escape `{{`/`}}` |
| `E-FMT-ARG-MISMATCH` | more `{}` than args | add args |
| `E-RANDOM-BOUNDS` | `lo > hi` in seeded int | `lo <= hi` |
| `E-ENV-INVALID` | `set_env` name with `=`/NUL | plain NAME |
| `E-TASK-GROUP` | several spawns failed (each under `related`) | fix children |
| `E-TYPE` | run-time type mismatch (dynamic `unknown` misused) | check types |
| `E-SANDBOX-FILE` | file outside temp dir (or `..`) under `klang run --sandbox` only | run without `--sandbox` or use temp-dir paths |
| `E-SANDBOX-SPAWN` | `spawn`/`run`/`run_process` under `--sandbox` only | run without `--sandbox` |
| `E-SANDBOX-NETWORK` | `http_get`/`http_post` (sync/async) under `--sandbox` only | run without `--sandbox` |

Sandbox codes only trigger under `klang run --sandbox` (plain `run` uses
`E-RUNTIME`/`E-PROCESS-*`/`E-NET-*` for the same operations). Examples (shell,
not gated — the gate runs plain `run` only):

```sh
klang run --sandbox prog.klang
# read_file("../x") or read_file("/etc/hostname") -> E-SANDBOX-FILE
# spawn("echo", ["hi"]) -> E-SANDBOX-SPAWN
# http_get("http://127.0.0.1:1/") -> E-SANDBOX-NETWORK
# temp-dir absolute paths and task_group/spawn f() still run
```

```klang error E-RUNTIME
fn main() -> i32 {
    return 1 / 0
}
```

```klang error E-OVERFLOW
fn main() -> i32 {
    print(9223372036854775807 + 1)
    return 0
}
```

```klang error E-PARSE-INT
fn main() -> i32 {
    print(parse_int("abc"))
    return 0
}
```

```klang error E-PARSE-FLOAT
fn main() -> i32 {
    print(parse_float("abc"))
    return 0
}
```

```klang error E-IO-NOT-FOUND
fn main() -> i32 {
    print(read_file("errors_nope_xyz.txt"))
    return 0
}
```

```klang error E-PROCESS-NOT-FOUND
fn main() -> i32 {
    print(spawn("nope_bin_xyz_123", []))
    return 0
}
```

```klang error E-REGEX-INVALID-PATTERN
fn main() -> i32 {
    print(regex_is_match("[", "hi"))
    return 0
}
```

```klang error E-TIME-INVALID
fn main() -> i32 {
    sleep_ms(0 - 1)
    return 0
}
```

```klang error E-NET-INVALID-URL
fn main() -> i32 {
    print(http_get("not a url !!!"))
    return 0
}
```

```klang error E-CHAR-INVALID
fn main() -> i32 {
    print(ord(""))
    return 0
}
```

```klang error E-STR-INVALID
fn main() -> i32 {
    print("hi".repeat(0 - 1))
    return 0
}
```

```klang error E-FMT-INVALID
fn main() -> i32 {
    print(sprintf("{", [1]))
    return 0
}
```

```klang error E-FMT-ARG-MISMATCH
fn main() -> i32 {
    print(sprintf("{} {}", [1]))
    return 0
}
```

```klang error E-RANDOM-BOUNDS
fn main() -> i32 {
    print(random_seeded_int(10, 1))
    return 0
}
```

```klang error E-ENV-INVALID
fn main() -> i32 {
    set_env("a=b", "x")
    return 0
}
```

## Other codes (no surface Klang repro)

- `E-EXIT`: `exit(n)` request — exits with `n & 0xFF`, not a failure; uncatchable.
- `E-CANCELLED`: sibling cancelled after another failed; excluded from group reports, never top-level.
- `E-TASK-GROUP`: needs 2+ failing spawns; single failure returns unwrapped (see `E-RUNTIME` above).
- `E-CONTRACT`, `E-OWNERSHIP-MODE`: library-only (`contracts`/`ownership` APIs); `klang check` never emits them.
- `E-PARSE-V2`, `E-PARSE-ECHO`, `E-PARSE-FLOW`, `E-SCHEMA-INVALID`, `E-SCHEMA-NOT-FOUND`, `E-ECHO-FAILURE-LOST`, `E-ECHO-INVALID-LISTEN`, `E-ECHO-OUTLIVES-SCOPE`, `E-ECHO-UNLISTENED`, `E-FLOW-DUPLICATE-DEPENDENCY`, `E-FLOW-MUTABLE-CAPTURE`, `E-RESONANCE-MISMATCH`, `E-RESONANCE-UNPROVEN`: v2-track only (`.v2` files, `check --lang v2`); v1 `klang check` never emits them.
- `E-NET-INVALID-HEADER`, `E-NET-UNREACHABLE`, `E-IO-PERMISSION`, `E-IO-FAILED`, `E-PROCESS-FAILED`, `E-REGEX-TOO-MANY`, `E-REGEX-TOO-LARGE`, `E-STR-TOO-LARGE`: same family as above; trigger with oversized inputs, bad headers, or OS faults (no network needed for the invalid cases shown).
- `W-TYPE-NARROW`: retired since D3 (empty); `W-DUPLICATE-DEPENDENCY`, `W-MUTABLE-CAPTURE`: flow lints (v2).
- `E-CLI-REMOVED`: `klang mcp` was removed (exit 2); use `klang check --json`, `klang run --sandbox`, or `klang fmt` instead. No MCP server exists.
- `JIT-FAIL`: not a diagnostic code; CLI prints `run: JIT-FAIL` on stderr when `--backend-jit` rejects out-of-subset code (floats, strings, closures, builtins, `try`, etc.).

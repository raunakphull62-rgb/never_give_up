# S7 dogfood findings — three real programs

Programs: `examples/real/minishell.klang`, `examples/real/organize.klang`,
`examples/real/logsum.klang`. Fixtures: `examples/real/fixtures/sample.log`,
`shell_demo.txt`, `organize_sample/`. Gates: `tests/s7_real_programs.rs`
(15 tests: 4 run + 1 fmt per program).

All three run under the interpreter (`klang run`). None runs under the JIT
or v2 backends (see Backend compatibility). No language features were added;
workarounds are in the programs and noted below.

## Backend compatibility

- `minishell.klang`: interpreter OK (`printf 'echo hi\nfalse\nexit\n' |
  klang run examples/real/minishell.klang` prints `hi` and `exit: 1`).
  JIT: refused loudly:
  `run: JIT-FAIL` + `jit phase-1 supports int-only MIR; builtin 'cwd' unsupported in 'main'`.
  v2 (`klang run --lang v2 examples/real/minishell.klang`): refused loudly:
  `parse: FAIL` + `E-PARSE-V2` `expected expression`
  (`input does not match the v2 grammar`).
- `organize.klang`: interpreter OK (`klang run ... -- /tmp/d --dry-run`
  prints `would move ...` and `would move 5 files`; without the flag it
  moves and prints `moved 5 files` + per-extension counts).
  JIT: `jit phase-1 supports int-only MIR; builtin 'args' unsupported in 'main'`.
  v2: same `E-PARSE-V2` `expected expression` failure.
- `logsum.klang`: interpreter OK (`klang run ... -- /tmp/sample.log`
  prints `INFO: 4`, `WARN: 1`, `ERROR: 2`, `first: ...`, `last: ...`,
  `top:` + `request ok: 3` etc.).
  JIT: `builtin 'args' unsupported in 'main'`.
  v2: same `E-PARSE-V2` failure.

JIT and v2 results are expected (integer-only JIT, experimental v2 subset
without loops/stdlib) and recorded here as refusal, not breakage.

## Missing features that blocked or forced workarounds

### 1. No `chdir` / `set_cwd` — `cd` cannot affect children

There is `cwd()` but no setter. `minishell.klang` keeps a logical `cur`:

```klang
let cur = cwd()
if (cmd == "cd") {
    let dest = toks[1]
    let raw = ""
    if dest.starts_with("/") {
        raw = dest
    } else if cur.ends_with("/") {
        raw = (cur + dest)
    } else {
        raw = ((cur + "/") + dest)
    }
    let norm = normalize_path(raw)
    try {
        if is_dir(norm) {
            cur = norm
            print(cur)
        } else {
            print(("cd: no such directory: " + dest))
        }
    } catch e {
        print(("cd: no such directory: " + dest))
    }
    continue
}
```

`pwd` is intercepted to print `cur`:

```klang
if (cmd == "pwd") {
    if (len(toks) == 1) {
        print(cur)
        continue
    }
}
```

External commands still inherit the parent cwd, so after `cd /tmp`,
`pwd` (builtin) says `/tmp` but `run_process("pwd", [])` would say the
old dir. The program documents this and never claims real chdir.

### 2. `read_line()` cannot distinguish empty line from EOF

Reference: stdin line without newline (`""` at EOF). A bare newline also
yields `""`. There is no `is_eof()` / `peek()`.

```klang
let line = read_line()
if (line == "") {
    return 0
}
let t = line.trim()
if (t == "") {
    continue
}
```

A bare `Enter` quits the shell:

```sh
printf 'echo hi\n\necho bye\nexit\n' | klang run examples/real/minishell.klang
# $ /content/wt1
# hi
# $ /content/wt1
# (exits; `echo bye` never runs)
```

Whitespace-only lines (`"   "`) are distinguishable and `continue`;
only `""` is ambiguous. Any REPL needs an EOF probe.

### 3. No `print` without newline

`print(x)` always ends with a newline on the CLI (`println!("{line}")`).
The shell prompt lives on its own line:

```klang
print(("$ " + cur))
```

```sh
printf 'echo hi\nexit\n' | klang run examples/real/minishell.klang
# $ /content/wt1
# hi
# $ /content/wt1
```

Captured child output (`run_process`/`spawn` `stdout` ends with `\n`)
would double-newline if printed raw, so every print goes through:

```klang
fn strip_one_newline(s: str) -> str {
    if s.ends_with("\n") {
        let n = len(s.chars())
        return slice(s, 0, (n - 1))
    }
    return s
}
```

Note `len(s)` is bytes while `slice` is char-based; the helper uses
`len(s.chars())` for correctness on non-ASCII.

### 4. File guard: absolute only inside temp dir, no `..`

`reject_unsafe_path`: absolute must start with `temp_dir()` (`/tmp` on
Linux), relative must not contain `..`/`/`-root, else `E-RUNTIME`
(`unsafe path '../evil'`, `unsafe absolute path '/etc/passwd'`), distinct
from `E-IO-NOT-FOUND`. So `organize`/`logsum` cannot take repo-absolute
fixtures (`/content/wt1/examples/...`) nor `cd ..` via the builtin. The
shell normalizes `..`/`.` itself:

```klang
fn normalize_path(p: str) -> str {
    let is_abs = 0
    if p.starts_with("/") {
        is_abs = 1
    }
    let parts = p.split("/")
    let stack = []
    for part in parts {
        if (part == "") {
            continue
        }
        if (part == ".") {
            continue
        }
        if (part == "..") {
            if (len(stack) > 0) {
                pop(stack)
            }
            continue
        }
        push(stack, part)
    }
    let out = stack.join("/")
    if (is_abs == 1) {
        return ("/" + out)
    }
    if (out == "") {
        return "."
    }
    return out
}
```

Tests use `temp_dir()` copies (`/tmp/klang-s7-...`) or relative
`examples/real/fixtures/...`.

### 5. No `env` listing

`env(name)` reads one variable (`""` when unset); there is no list-all.
`env` with no args shells out:

```klang
if (cmd == "env") {
    if (len(toks) == 1) {
        try {
            let r = spawn("env", [])
            let so = r["stdout"]
            if (so != "") {
                print(strip_one_newline(so))
            }
        } catch e {
            print(e["code"])
        }
        continue
    }
    if (len(toks) == 2) {
        print(env(toks[1]))
        continue
    }
}
```

`NAME=value` (single token, valid name) uses `set_env`:

```klang
if (len(toks) == 1) {
    if toks[0].contains("=") {
        let eqparts = toks[0].split("=")
        let name = eqparts[0]
        if (is_valid_env_name(name) == 1) {
            let val = join_from(eqparts, 1, "=")
            try {
                set_env(name, val)
            } catch e {
                print(e["code"])
            }
            continue
        }
    }
}
```

`FOO=bar baz` (space after `=`) is not an assignment by design; it falls
through to `spawn` and reports `command not found`.

### 6. `args()` needs `--`

`klang run prog.klang foo` treats `foo` as the entry name, not argv.
Program argv is everything after `--`, else non-flag positionals after
path/entry. All three programs document `klang run ... -- <args>`:

```klang
let a = args()
if (len(a) == 0) {
    print("usage: logsum <file>")
    return 1
}
```

```sh
klang run examples/real/organize.klang -- /tmp/mydir --dry-run
klang run examples/real/logsum.klang -- /tmp/app.log
```

### 7. No `index_of` / `substring(search)` / `trim(chars)`

Parsing uses `split` + manual scan. `ext_of` (`organize.klang`):

```klang
fn ext_of(name: str) -> str {
    if !name.contains(".") {
        return "noext"
    }
    let parts = name.split(".")
    if (len(parts) == 2) {
        if (parts[0] == "") {
            return "noext"
        }
    }
    let last = parts[(len(parts) - 1)]
    if (last == "") {
        return "noext"
    }
    return last.lower()
}
```

`split_shell` (`minishell.klang`) loops `line.chars()` tracking
`in_single`/`in_double`; no backslash escapes (documented: `echo "a\"b"`
does not unescape).

### 8. No `sort_by` / map removal — top-5 is manual max-scan

`sort()` orders homogeneous values, not by count. `logsum.klang` scans:

```klang
let shown = {}
let n = 0
while (n < 5) {
    let best = ""
    let best_c = 0
    let found = 0
    for k in counts {
        if shown.contains(k) {
        } else {
            let c = counts[k]
            if (found == 0) {
                best = k
                best_c = c
                found = 1
            } else if (c > best_c) {
                best = k
                best_c = c
            }
        }
    }
    if (found == 0) {
        break
    }
    print((best + ": ") + str(best_c))
    shown[best] = 1
    n = (n + 1)
}
```

Ties resolve by insertion order (first-seen wins), deterministic because
`list_dir` is sorted and log order is file order.

### 9. Three process builtins with three shapes

`run_process(cmd, args)` → `{stdout, stderr, exit_code}`,
`spawn(cmd, args)` → `{status, stdout, stderr}`,
`run(cmd, args)` → `i32` with inherited stdio. The shell uses `spawn`
(captured, order-preserving via `print`). `run` (inherit) was rejected
for the shell: child output appears immediately while `print` output is
flushed at the end, so `print("before"); run("echo",["during"])`
shows `during` before `before` on the CLI.

### 10. No recursive walk / dir removal

`list_dir` lists one level (sorted, no `.`/`..`); `remove_file` deletes
files only. `organize` moves top-level files only, skips subdirs via:

```klang
if is_file(src) {
} else {
    continue
}
```

## Awkward spots (verbosity, error messages, API design)

- `fmt` fully parenthesizes: `a + b` → `(a + b)`,
  `if op == "+"` → `if (op == "+")`, `print(a + b)` → `print((a + b))`.
  Idempotent and behavior-preserving, but noisy for newcomers.
- `Bool` vs `Int`: `contains`/`starts_with`/`ends_with`/`is_dir`/`is_file`
  check as `Bool` yet render as `Int` 0/1 at runtime. `== 0`/`== 1`
  against them is `E-TYPE`:
  `if name.contains(".") == 0` fails with
  `E-TYPE` `` `==` operands: want bool, got i32 ``
  (`operand types do not match the operator`, `types/mismatch`).
  Use truthiness (`if name.contains(".")`) or negation
  (`if !name.contains(".")`); the latter parses as
  `!(name.contains("."))` and was verified.
- `len(str)` is bytes, `s[i]`/`s.chars()`/`slice(str)` are chars:
  `len("é")` is 2, `len("é".chars())` is 1. Trimming one trailing `\n`
  must use `len(s.chars())`, not `len(s)`.
- `break`/`continue` cannot cross `try`: `continue` inside `catch`
  targeting the outer `while` is `E-LOOP`
  (`break/continue cannot cross a 'try' boundary`,
  `break and continue only jump within their own 'try' region`,
  `control/loop`). The shell hit this once:
  ```klang
  try {
      return parse_int(toks[1])
  } catch e {
      print("exit: numeric argument required")
      continue
  }
  ```
  Fixed as:
  ```klang
  try {
      return parse_int(toks[1])
  } catch e {
      print("exit: numeric argument required")
  }
  continue
  ```
  (success returns; failure falls through to the outer `continue`).
  Same discipline forces an `ok` flag for `make_dirs`/`rename_file`
  inside `for` in `organize.klang`.
- `slice` on `unknown` arrays returns `Str` (see Bugs): forces a manual
  `push` loop to build `cargs` for `spawn`.
- v2 parse errors are generic: any `while`/`for` program fails as
  `E-PARSE-V2` `expected expression`
  (`input does not match the v2 grammar`,
  `check schema echo flow tune listen spelling`, `syntax/v2`) with no
  hint that loops are unsupported.
- Most diagnostics carry `0,0` spans (known limitation); only
  `E-PARSE`/`E-TASK-CANCEL`/`E-SPAWN-OUTSIDE-GROUP`/`E-EFFECT-MISMATCH`
  have real spans. Name-based attribution still worked.

## Bugs found (with a minimal repro each)

### B1. `slice()` on a dynamic array is typed `Str`, breaking `spawn`

`check_builtin_call`: `slice` returns `Array` only when the first arg is
statically `Array`; `Unknown` falls through to `Str`. At runtime a
dynamic array slices to an array, so the check is wrong.

Repro:

```klang
fn foo() -> array {
    let a = ["a", "b", "c"]
    return a
}

fn main() -> i32 {
    let t = foo()
    let u = slice(t, 1, 3)
    print(u.join(","))
    let r = spawn("echo", u)
    print(r["status"])
    return 0
}
```

`klang check` fails:

```json
{
  "code": "E-TYPE",
  "message": "`spawn()` args: want array, got str",
  "cause": "operand types do not match the operator",
  "rule": "types/mismatch"
}
```

Workaround used in `minishell.klang` (build `cargs` by hand):

```klang
let cargs = []
let k = 1
while (k < len(toks)) {
    push(cargs, toks[k])
    k = (k + 1)
}
```

`cargs` is statically `Array` (`[]`), so `spawn(cmd, cargs)` checks
clean. `split("...")` on a `Str` receiver returns static `Array`, so
only function-returned (`Unknown`) arrays trigger this.

### B2. `run()` (inherit) output is not ordered with `print()`

`run` writes directly to process stdio during execution; `print` lines
accumulate and flush after the program ends. Repro:

```klang
fn main() -> i32 {
    print("before")
    let c = run("echo", ["during"])
    print("after")
    return 0
}
```

CLI shows `during` first, then `before`/`after`:

```sh
$ klang run /tmp/probe_order.klang
during
before
after
```

The shell avoids `run` and uses `spawn` + `print(strip_one_newline(...))`
so prompts and child output stay ordered. Whether this is a bug or a
documented inherit-vs-capture tradeoff, the interleaving is surprising
and worth a note in the CLI docs.

### B3. v2 loop rejection gives no actionable message

```klang
fn main() -> i32 {
    let t = 0
    while (t < 3) {
        t = (t + 1)
    }
    print(t)
    return t
}
```

`klang run --lang v2` fails with the same generic diagnostic as any v2
syntax error (`E-PARSE-V2`, `expected expression`), naming no construct.
v2 explicitly has no loops/closures/floats/stdin/file builtins
(`limitations.md`), so the diagnostic could name the unsupported
construct (`while`/`for`/`read_line`/...) instead of `expected
expression`.

No existing code was changed for these; all three are recorded here.
The `slice` typing (B1) is the best small-fix candidate
(return `Unknown` for `Unknown` input instead of `Str`).

## Things that worked well

- `spawn` argv fidelity: `echo "hello world"` arrives as one arg;
  `printf '<%s>'` style tests already pin spaces/`;| $` as literal.
  The shell relies on this and quoting works first try.
- `list_dir` sorted determinism: dry-run and real-run outputs are
  byte-identical across runs (`README, a.txt, b.md, c.txt` order);
  per-extension counts (`txt: 2`, `md: 1`) are stable.
- Specific error codes with real causes:
  missing file → `E-IO-NOT-FOUND` (`read_file(...) failed: file does
  not exist`); unsafe path → `E-RUNTIME` (`unsafe path '../evil'`);
  missing binary → `E-PROCESS-NOT-FOUND`
  (`spawn(...) failed: command not found or not executable` with the OS
  `No such file...` cause). `try { ... } catch e { print(e["code"]) }`
  made the shell/organize/logsum resilient without string parsing.
- `split`/`join`/`trim`/`lower`/`contains`/`starts_with`/`ends_with`/
  `chars`/`replace` covered all parsing (extension extraction,
  `NAME=value` split on first `=`, timestamp/level/message split).
  `lower()` made `photo.JPG` → `jpg/` grouping trivial.
- `args()` + `--` is predictable once documented; `is_dir`/`is_file`/
  `make_dirs` (idempotent) / `rename_file` / `read_file` / `exists`
  composed into a real mover without new builtins.
- `map` + `contains` guard pattern (`if counts.contains(k)` before
  `counts[k]`) avoided the missing-key `E-RUNTIME` cleanly;
  `for k in map` yields keys in insertion order, so summaries are
  deterministic.
- `fmt --write` is idempotent and comment-preserving; all three
  programs are `fmt: clean` and `check: OK`.
- Integer/float/string `+` concat (`"moved " + src + " -> " + dst`,
  `"exit: " + str(code)`) and `str()` rendering removed all manual
  formatting.

## Top 10 gaps ranked by hurt

1. No `chdir` — `cd` is logical-only; children keep the old cwd.
   Breaks relative `ls`/`cat`/script paths after `cd`.
2. `read_line()` EOF (`""`) indistinguishable from empty line — bare
   `Enter` quits any REPL; no `is_eof()` to fix it.
3. No newline-free output — prompt on its own line; captured `stdout`
   needs manual one-`\n` trim to avoid blank lines.
4. File guard forbids absolute outside temp and any `..` — real paths
   (`/etc/...`, repo-absolute fixtures) and `cd ..` need manual
   `normalize_path` + temp copies.
5. `slice(unknown)` typed `Str` (B1) — dynamic arg slices cannot feed
   `spawn`/`join` without a hand copy loop.
6. `run()` inherit vs `print()` ordering (B2) — forces `spawn`+`print`
   for shells; inherit is unusable when order matters.
7. `Bool` (`contains`/`is_dir`/...) vs `Int` `== 0/1` is `E-TYPE` —
   forces truthiness idiom; message (`want bool, got i32`) confuses.
8. No `env` dump — `env` with no args must exec `/usr/bin/env`.
9. `args()` needs `--` — `klang run prog.klang foo` takes `foo` as entry.
10. `len` bytes vs `slice`/`[]` chars + no `index_of` + `continue`
    cannot cross `try` — each small alone, together they make every
    parser a `chars()` loop with an `ok`-flag after each `try`.

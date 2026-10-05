# Klang Builtins — tested reference (Step 0)

Source of truth: `src/hir.rs` (`is_builtin`, `check_builtin_call`,
`check_method_call`) + `src/runtime/mod.rs` (`exec_builtin`).
Every entry below was verified with a tiny `.klang` program via
`klang run` (klang 0.6.0). Do not guess — re-probe if the compiler changes.

Conventions: `i32` = Int, `f64` = Float. `unknown` (map lookups, dynamic
index) is a wildcard that skips type checks. Errors are JSON diagnostics
with `code`; all runtime failures are catchable with `try {} catch e {}`
except `E-CANCELLED`.

## Core

| Fn | Signature | Tested behavior |
|---|---|---|
| `len` | `(array \| str \| map) -> i32` | `len(str)` = **bytes** (`len("é") == 2`); array = elements; map = entries. |
| `push` | `(arr: array, v: any) -> i32` | Mutates caller's array binding; returns new length. |
| `pop` | `(arr: array) -> any` | Removes + returns last; empty array is `E-RUNTIME` (`pop() of empty array`). |
| `insert` | `(arr: array, idx: i32, v: any) -> i32` | `idx == len` appends; negative or `> len` is `E-RUNTIME` (`insert() index out of bounds`). Returns new length. Tested: `insert([10,20,30],1,99)` → len 4, `[10,99,20,30]`. |
| `keys` | `(m: map) -> array<str>` | Insertion order. Tested: `keys({"a":1,"b":2})` → `["a","b"]`. |
| `range` | `(lo: i32, hi: i32) -> array<i32>` | `[lo, hi)`; capped at 100000 elements (`range() too large`). |
| `str` | `(v: any) -> str` | `Value::render`: ints `42`, floats keep `.0` (`str(3.0) == "3.0"`), bools `1`/`0`. |
| `int` | `(num \| str) -> i32` | Trims + parses (`int("42") == 42`); float truncates (`int(3.9) == 3`); failure is generic `E-RUNTIME`. Range is `i64`. |
| `float` | `(num \| str) -> f64` | `float(3) == 3.0`; `float("3.5") == 3.5`; failure is generic `E-RUNTIME`. |
| `parse_int` | `(s: str) -> i32` | **Strict**: optional single `-`, ASCII digits only, `i32` range. `parse_int(" 12")`, `"+12"`, `"1_2"` → `E-PARSE-INT`. Tested: `parse_int("42") == 42`, `parse_int("-7") == -7`. |
| `parse_float` | `(s: str) -> f64` | Finite `f64` grammar (`"1e3"` → `1000.0`); no surrounding whitespace; `inf`/`NaN` → `E-PARSE-FLOAT`. |
| `format` | `(...any) -> str` | Variadic incl. zero args; renders each like `str()` joined with one space. Tested: `format("a",1,2.5) == "a 1 2.5"`. |
| `assert` | `(cond: bool \| int) -> i32` | Falsy (`false`/`0`) → `E-RUNTIME` (`assert failed`); returns 1. |
| `read_line` | `() -> str` | One stdin line without newline; `""` at EOF. |

## Files (`reject_unsafe_path`: `..`/absolute-outside-tmp blocked; `/tmp/...` OK)

| Fn | Signature | Tested behavior |
|---|---|---|
| `read_file` | `(path: str) -> str` | Whole file. Missing → `E-IO-NOT-FOUND` (`check the path exists first with exists()`). |
| `write_file` | `(path: str, content: str) -> i32` | Overwrites; returns **byte count**. Tested: `"hello"` → 5. |
| `append_file` | `(path: str, content: str) -> i32` | Appends (creates when absent); returns bytes appended. Tested: `"!!"` → 2. |
| `exists` | `(path: str) -> bool` | Never fails. Tested `1`/`0`. |
| `remove_file` | `(path: str) -> i32` | Returns 1. Missing → `E-IO-NOT-FOUND`. (Files only.) |

## System

| Fn | Signature | Tested behavior |
|---|---|---|
| `env` | `(name: str) -> str` | Value or `""` when unset. Tested: unset var prints empty line, exit 0. No `set_env` builtin. |
| `run_process` | `(cmd: str, args: array<str>) -> map` | No shell (argv array). Map: `{stdout: str, stderr: str, exit_code: i32}`. Non-zero exit is a **normal result** (tested `sh -c "exit 3"` → `exit_code == 3`, `stdout == ""`). Missing binary → `E-PROCESS-NOT-FOUND`. Tested: `run_process("echo",["hi","there"])` → `stdout == "hi there\n"`, keys `stdout,stderr,exit_code`. |

## Regex

| Fn | Signature | Tested behavior |
|---|---|---|
| `regex_is_match` | `(pat: str, text: str) -> bool` | Searches anywhere. Bad pattern → `E-REGEX-INVALID-PATTERN` (real crate message as cause; tested `" kleine"` → `unclosed character class`). |
| `regex_find` | `(pat: str, text: str) -> map` | Map: `{matched: int 0/1, match: str, groups: array<str> (1..n, "" for non-participating), named: map}`. No match is normal (`matched == 0`, `match == ""`, empty groups/named) — tested `regex_find("xyz","hello")`. Named groups tested: `(?P<user>\w+)@(?P<host>\w+)` on `"contact bob@example now"` → `match == "bob@example"`, `groups == ["bob","example"]`, `named["user"] == "bob"`. |

## Time (`time_now` = seconds since Unix epoch, `f64`)

| Fn | Signature | Tested behavior |
|---|---|---|
| `time_now` | `() -> f64` | Wall clock. Tested `1791198009.3681743` vs `date +%s` → `1791198009`. Fractional part = sub-second. |
| `time_sleep` | `(secs: int \| float) -> i32` | Blocking; fractional OK. Negative/NaN/inf/too-large → `E-TIME-INVALID`. Non-number → `E-TIME-INVALID`. Returns 1. |
| `time_elapsed` | `(since: int \| float) -> f64` | `time_now() - since` on same clock; future `since` → negative (no clamp). Tested `> 0.0` after `time_sleep(0.01)`. |

## HTTP (real client over `ureq`; 10s connect / 60s total)

Response map: `{status: int, body: str, headers: map (lowercased name → value, repeats joined with ", ")}`. 4xx/5xx are **normal results** (populate `status`); only transport/URL/header failures raise.

| Fn | Signature | Tested behavior |
|---|---|---|
| `http_get` | `(url: str) -> map` | Malformed URL → `E-NET-INVALID-URL` (tested `"not a url at all"` → `http: invalid uri character`). Unreachable host → `E-NET-UNREACHABLE`. |
| `http_post` | `(url: str, body: str, headers: array<str>) -> map` | Headers are `"Name: Value"` strings (same shape as argv). Bad header → `E-NET-INVALID-HEADER`. Same URL/transport codes as `get`. |
| `http_get_async` / `http_post_async` | same as sync | Identical exchange on bounded pool; callable only from `async` fns (`E-EFFECT-MISMATCH` otherwise). |

## String methods (receiver `str`)

`len` (bytes) · `upper`/`lower` (Unicode) · `trim` (whitespace both ends) ·
`chars` → `array<str>` (Unicode scalars; `len("abc".chars()) == 3`) ·
`split(delim)` literal (tested `"a,b,c".split(",")` → 3 elems) ·
`contains/starts_with/ends_with` literal (`"foobar".contains("oba") == 1`) ·
`replace(a,b)` literal (`"aab".replace("a","z") == "zzb"`).
No `slice/substring/ord/chr` (Phase 2). `s[i]` is char-indexed; `for i in
0..len(s)` is ASCII-only — use `chars()` for non-ASCII.

## Array methods / map methods

Array: `len/push/pop/contains/join/insert` (`join(sep)` renders via `str()`;
tested `[0,1,2].join(",") == "0,1,2"`). Map: `len/keys/contains`
(`m.contains("b") == 1`). `m[k]` missing key → `E-RUNTIME`; `m[k] = v`
upserts; `a[i] = v` in-bounds only.

## Not available (Phase 2 Batch B)

Float fns (`sqrt pow abs min max floor ceil sin cos tan atan2 log
exp`), `random_int`, `random_float`, `sha256`, `hmac_sha256`, sockets.

## System + data (Phase 2 Batch A)

Program arguments: `klang run prog.klang -- a b` puts `["a","b"]`
into `args()`; without `--`, non-flag positionals after path/entry
are used (second positional stays the entry name, historically).
Entry-param binding is untouched (`main()` + CLI args is not an arity
error). JIT and MCP paths see `[]` (builtins are interpreter-only).

| Fn | Signature | Tested behavior |
|---|---|---|
| `args` | `() -> array<str>` | Program argv after the `.klang` file; `[]` when none. |
| `exit` | `(code: i32) -> !` | Unwinds everything as `E-EXIT` (message `exit(N)`); **uncatchable** like `E-CANCELLED`. CLI exits with the low 8 bits. Never returns. |
| `cwd` | `() -> str` | Process working dir; unreachable CWD is `E-IO-FAILED`. |
| `set_env` | `(name: str, value: str) -> i32` | Returns 1. Names with `=`/NUL are `E-ENV-INVALID` (`set_var` would panic). |
| `list_dir` | `(path: str) -> array<str>` | Entry names (no `.`/`..`), **sorted** (raw `read_dir` order is OS-dependent). Missing → `E-IO-NOT-FOUND`. |
| `make_dir` | `(path: str) -> i32` | Single level; existing dir → `E-IO-FAILED` (use `make_dirs`). |
| `make_dirs` | `(path: str) -> i32` | Idempotent (`create_dir_all`). |
| `is_dir` / `is_file` | `(path: str) -> bool` | Never fail (mirror `exists`); all guarded by `reject_unsafe_path` → `E-RUNTIME` on `..`/foreign-absolute paths. |
| `rename_file` | `(from: str, to: str) -> i32` | Returns 1; missing source → `E-IO-NOT-FOUND`. Both paths guarded. |
| `copy_file` | `(from: str, to: str) -> i32` | Returns bytes copied. |
| `file_size` | `(path: str) -> i32` | Bytes. Missing → `E-IO-NOT-FOUND`. Huge sizes flow as values but cannot enter checked `i32` arithmetic (same as oversized `int()`). |
| `ord` | `(s: str) -> i32` | First char's codepoint. Empty → `E-CHAR-INVALID`; non-string → `E-CHAR-INVALID`. |
| `chr` | `(n: i32) -> str` | Scalar → 1-char string. Surrogates/negatives/`>0x10FFFF` → `E-CHAR-INVALID`. |
| `slice` | `(str, lo: i32, hi: i32) -> str` / `(array, lo, hi) -> array` | Char-based `[lo, hi)`. Negative, `hi < lo`, or past-the-end → `E-RUNTIME` (same loudness as `s[i]`). |
| `sort` | `(list: array) -> array` | Homogeneous ints/floats (`total_cmp`, NaN-safe)/strings only; mixed (incl. int+float) → `E-TYPE`. Empty sorts to empty. |

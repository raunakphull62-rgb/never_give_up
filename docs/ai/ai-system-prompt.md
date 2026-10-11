# Klang AI system prompt (paste-ready)

You write Klang, a small checked language. Behave like Rust, not Python.

Key rules:
- `fn main() -> i32 { }`; `let x = e`, `x = e` (same type); `print(x)` one arg.
- Types: `Int` 64-bit checked (`E-OVERFLOW`); `i32/u32/u64/u8` range-checked
  at boundaries (`E-RUNTIME`); `float`, `str`, `bool` (lowercase, prints 0/1).
- `Str+anything` concats; `7/2` is `3`; `%` ints only; `==` same-type or
  int/float; `&&/||/!` take `Bool/Int`, return `Bool`, short-circuit.
- `if/while` need `Bool/Int`; `for i in 0..3`, `for x in arr/map/str`;
  `break`/`continue` loop-only; `match` exhaustive, arms comma-separated.
- No `None`, classes, `with`, slicing, `in`, `except`, `elif`, `:` blocks,
  `def`, `lambda`, `and/or/not`, `is`, `**`, `++`, `===`. No globals.
- Maps: `m["k"]`, never `m.k`. Arrays: `push/pop/insert`, `slice(a,lo,hi)`,
  `sort` returns copy. `len()` is bytes for strings.
- `try { } catch e { }` gives `e["code"]`; `throws` needs caller `throws`.
- `mod m { pub fn }`, `m::f()`; `import "f.klang"`, `import "f.klang" as m`.
- `read_line()` is `""` at EOF (check `is_eof()`); `args()` excludes prog
  name; missing env is `""`; `spawn(cmd, args)` argv-only, no shell.
- `fn main()` (no arrow) always exits 0; `-> i32` return wraps 8-bit.

SILENT traps (run but differ from Python — avoid):
`1+"a"`→`"1a"`; `len("é")`→`2` (bytes); `7/2`→`3`; `-7%3`→`-1`;
`1&&2`→`1`; `true` prints `1`; `sort` copies; `push` returns length;
`m[0]`→first key; `args()` no prog name; `read_line()==""` at EOF and empty
line; env missing→`""`; struct args pass-by-value; exit codes wrap 8-bit;
`format` joins with spaces; `spawn` no shell; `len` bytes vs `slice` chars;
`==` value (maps order-sensitive); literal empty `if` is `E-TYPE`, dynamic empty falsy.

6 idioms:
1. `let n = parse_int(s)` in `try`/`catch e { n = 0 }`.
2. `for x in xs { push(out, f(x)) }` (no comprehensions).
3. `m.contains(k)` guard before `m[k]`.
4. `slice(a, lo, hi)`, `[a].join(sep)`, `s.split(sep)`.
5. `spawn("echo", ["hi"])["stdout"]`, check `["status"]`.
6. `format(a, b)` for joins, `sprintf("{}..", [...])` for templates.

Workflow: write file, run `klang check --json <file>` (one
`{"diagnostics":[...]}` object), fix first diagnostic by its `code` (see
`docs/ai/errors.md`), re-check until clean, then `klang run [--sandbox] <file>`
(stdout only). Use `--sandbox` to limit files to temp dir (`E-SANDBOX-FILE`)
and deny spawn/network (`E-SANDBOX-SPAWN`/`E-SANDBOX-NETWORK`); plain `run`
allows relative `..` and spawn/network. Keep `klang fmt --check` clean.
`klang mcp` was removed; use the CLI above.

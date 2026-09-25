# Getting started

## Build from source

Klang is a Rust binary with no runtime dependencies beyond the
toolchain itself. Database-backed `cargo` access is needed once to
fetch crates (`salsa`, `cranelift-*`); after that the build is offline.

```sh
git clone <repo> klang && cd klang
cargo build
```

The binary is `target/debug/klang`. If your checkout lives on a
noexec mount (some container setups), point the target dir elsewhere:

```sh
CARGO_TARGET_DIR=/tmp/klang-target cargo build
```

## First program

Save this as `hello.klang`:

```klang
// @run prints: 14 ; return: 42
fn main() -> i32 {
    let x = 2 + 3 * 4
    print(x)
    return 40 + 2
}
```

Check it (parse + type-check, prints diagnostics as JSON on failure):

```sh
cargo run -- check hello.klang
# parse: OK (1 functions, 0 structs, 0 enums)
# check: OK (0 diagnostics)
```

Run it (check + lower + execute, entry defaults to `main`):

```sh
cargo run -- run hello.klang
# print: 14
# run main() = 42
```

`print(x)` writes to stdout and the entry function's integer return
becomes the result line. A program that fails to check never runs:
`run` stops after the diagnostics.

## CLI subcommands

```sh
cargo run -- --version                # print version and exit
cargo run -- check <file>               # parse + type-check only
cargo run -- run <file> [entry]         # parse + check + run (entry defaults to main)
cargo run -- build <file>               # parse + check + print MIR listing
cargo run -- fmt <file> [--write]       # print canonical source (or rewrite in place)
cargo run -- repair <file> [entry] [flags...]  # diagnostic-guided LLM repair
```

Backcompat shorthand: `cargo run -- <file.klang> [entry]` means `run`.

What each one actually does:

- **`check`** parses (with `import` merging — see the tour) and runs
  the full HIR suite: effects, names, types, enums, generics,
  structured concurrency. Exit 0 with `check: OK`, exit 1 with one
  JSON diagnostic object per error on stdout.
- **`run`** does everything `check` does, prints the MIR listing,
  then executes `entry` (default `main`) in the interpreter and
  prints `print:` lines plus `run <entry>() = <int>`. `--backend-jit`
  switches execution to the Cranelift JIT, which only supports
  integer code — the interpreter is the default and the reference.
- **`build`** does everything `check` does and prints the MIR
  listing, then stops. (`--backend-jit` is accepted but noted as
  run-only; `build` never emits machine code.)
- **`fmt`** prints the canonical form: 4-space indent, one statement
  per line, binary operators fully parenthesized. Formatting is a
  fixpoint (`fmt(fmt(x)) == fmt(x)`) and formatted code re-parses to
  the same program. Note: **comments are not preserved** — `fmt`
  renders the AST, which has no comment nodes, so `--write` strips
  `//` and `/* */` comments. Keep your commented source and treat
  `fmt` output as a canonical snapshot.
- **`repair`** is covered in its own [guide](repair.md).

Two runnable examples ship in the repo: `examples/simple.klang`
(minimal) and `examples/full.klang` (language tour), plus
`examples/eval.klang`, a small expression evaluator combining
generics, enums, modules, and effects. All three check clean and run:

```klang
// @run prints: 55 | 45 | 42 | klang | KLANG | hi klang ; return: 42
fn main() -> i32 {
    let total = 0
    let i = 1
    while i <= 10 {
        total = total + i
        i = i + 1
    }
    print(total)
    print(sum_to(10))
    print(total - 13)
    print("klang")
    print("klang".upper())
    print("hi " + "klang")
    return total - 13
}

fn sum_to(n: i32) -> i32 {
    let s = 0
    for i in 0..n {
        s = s + i
    }
    return s
}
```

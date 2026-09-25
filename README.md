# Klang

**Klang is a small compiled language and toolchain for programs that need clear, repeatable verification.** Its Rust compiler parses and type-checks `.klang` source, reports structured diagnostics, and can run checked programs in an interpreter. A standard-I/O Model Context Protocol (MCP) server exposes those same checks and formatting tools to an AI coding harness. Klang does not make model or network calls itself.

The current language includes functions, control flow, structs, arrays, maps, strings, enums and exhaustive pattern matching, generics, modules, imports, and effect annotations. Its `async` task groups enforce that spawned tasks are awaited before the group ends. See the [language tour][1] and [language reference][2] for syntax and precise behavior.

## What you can do with Klang

- **Check source:** parse and type-check a program before it runs. Errors are reported as structured diagnostics with codes, messages, causes, rules, and suggested fixes where available.
- **Run programs:** execute checked source in the reference interpreter. The optional Cranelift JIT backend is available for integer-focused programs.
- **Inspect the compiler pipeline:** lower checked programs to an intermediate representation (MIR) and print a readable instruction listing.
- **Format source:** render a canonical form with consistent indentation and parenthesized operators.
- **Integrate with coding agents:** serve compiler tools over MCP/stdio so an external harness can check, run, format, and plan a focused repair.
- **Explore a separate v2 track:** explicitly check or run v2 programs using resonance types, schemas, flows, and echoes. V2 uses separate commands or an explicit language flag; `.klang` files do not silently switch modes.

## Quick start

You need a stable Rust toolchain. From the project directory, build the binary:

```sh
cargo build
```

Create `hello.klang`:

```klang
fn double(n: i32) -> i32 {
    return n * 2
}

fn main() -> i32 {
    let answer = double(21)
    print(answer)
    return answer
}
```

Check the program, then run it:

```sh
cargo run -- check hello.klang
cargo run -- run hello.klang
```

A successful run prints the program output (`42`) and the entry function’s return value (`run main() = 42`). The `run` command also prints the MIR listing before execution. The entry point defaults to `main`; pass another function name after the file to choose a different entry point.

## Command-line reference

| Command | Behavior |
|---|---|
| `cargo run -- check <file.klang>` | Parse and type-check source; print diagnostics on failure. |
| `cargo run -- run <file.klang> [entry]` | Check, lower to MIR, then execute with the interpreter. |
| `cargo run -- run <file.klang> [entry] --backend-jit` | Execute with the optional JIT backend; its supported surface is integer-only. |
| `cargo run -- build <file.klang>` | Check and print MIR; this command does not emit a native executable. |
| `cargo run -- fmt <file.klang>` | Print canonical formatting. |
| `cargo run -- fmt <file.klang> --write` | Rewrite the file with canonical formatting. **Comments are not preserved by the formatter.** |
| `cargo run -- repair <file.klang> --dry-run` | Show the diagnostics and planned repair prompt without calling a model. |
| `cargo run -- mcp` | Start the MCP server over standard input and output. |
| `cargo run -- check-v2 <file.v2>` | Check a v2 program. `check --lang v2 <file.v2>` is an equivalent explicit form. |
| `cargo run -- run-v2 <file.v2> [entry]` | Run a v2 program. `run --lang v2 <file.v2> [entry]` is an equivalent explicit form. |

For v1 programs, passing a source file without a subcommand remains a shorthand for `run`:

```sh
cargo run -- examples/simple.klang
```

## Language at a glance

Klang uses newline-terminated statements and a small, familiar syntax:

```klang
struct Point { x: i32, y: i32 }

enum Result<T> { Ok(value: T), Err(code: i32) }

fn describe(point: Point) -> str {
    return "point: " + str(point.x) + ", " + str(point.y)
}

fn sum_to(limit: i32) -> i32 {
    let total = 0
    for i in 0..limit {
        total = total + i
    }
    return total
}

fn main() -> i32 {
    let point = Point { x: 20, y: 22 }
    print(describe(point))
    print(match Result::Ok(sum_to(3)) {
        Result::Ok(value) => value,
        Result::Err(code) => code
    })
    return point.x + point.y
}
```

The checker enforces function argument and return types, enum exhaustiveness, generic inference at call sites, and module visibility. Arrays and maps are dynamically typed. The language also provides built-ins for common collection operations, conversions, assertions, file access, and environment-variable access. Consult the [reference][2] for exact rules and diagnostics.

## Structured concurrency and effects

Functions declare effects in their signatures. Calling a `throws` function requires the caller to declare `throws`. Functions that use `task_group`, `spawn`, or `await` must be marked `async`.

A spawned task must be bound directly to a variable inside a `task_group`, and its handle must be awaited before leaving that group. The checker reports violations before execution. At runtime, spawned tasks run on threads and `await` joins them. When a child fails, Klang cancels siblings cooperatively and retains the group’s failures in its diagnostics.

## MCP integration for coding harnesses

Start Klang as an MCP server over stdio:

```sh
cargo run -- mcp
```

The server handles JSON-RPC messages and offers these tools:

| Tool | Purpose |
|---|---|
| `klang_check` | Check source and return compiler diagnostics as JSON. |
| `klang_run` | Check, then run source with a bounded execution time. |
| `klang_fmt` | Return canonical source formatting. |
| `klang_scope_plan` | Use check diagnostics to identify a focused function-level edit scope or recommend file scope. |
| `klang_v2_check` | Check source written for the separate v2 language track. |

A repair loop can therefore remain with the harness: check the source, send the diagnostics to its model, apply a proposed edit, and check again. Klang supplies deterministic compiler feedback; it does not choose or call a model. `repair --dry-run` is available to inspect the planned first prompt without making model calls. See the [repair guide][3].

## Build, test, and verify examples

Build and run the Rust test suite with Cargo:

```sh
cargo build
cargo test
```

The repository also includes documentation examples with machine-check directives. After building, verify those examples against the real binary:

```sh
scripts/verify_docs.py --bin target/debug/klang
```

The test suite covers the parser, type checker, runtime, task groups, enums, generics, modules, package helpers, MCP tools, repair planning, security checks, and v2 components. Runnable examples are in [`examples/`](examples/); user documentation is in [`docs/`](docs/).

## Project structure

| Path | Contents |
|---|---|
| `src/lexer/`, `src/parser/` | Lexers and parsers for the main language and the separate v2 track. |
| `src/hir.rs`, `src/sema/` | Typed high-level representation and semantic checks. |
| `src/mir/`, `src/runtime/` | Intermediate representation and interpreter runtimes. |
| `src/jit.rs`, `src/codegen.rs` | Optional JIT execution and readable MIR listing. |
| `src/diagnostics.rs`, `src/ai_safety/` | Structured diagnostics and input-safety helpers. |
| `src/mcp.rs`, `src/repair/` | MCP tools and model-agnostic repair planning. |
| `src/package/`, `src/db.rs` | Manifest/lockfile helpers and the incremental database. |
| `docs/` | Getting started, language tour, reference, repair guide, and limitations. |
| `examples/`, `tests/` | Example programs and unit/integration gate tests. |

## Current limitations

Klang is actively developed, and some features are intentionally narrower than in mature general-purpose languages:

- `&&` and `||` evaluate both operands; they do not short-circuit.
- The interpreter is the reference backend. The optional JIT supports integer code only, and `build` stops after printing MIR.
- `fmt` formats the parsed syntax tree and drops comments from its output.
- There are no closures, function values, trait bounds, match guards, or `throw` statement. `cancel` is accepted as an effect annotation but is not propagation-checked.
- Package helpers provide manifest and content-hash lockfile support, not a package registry or network dependency fetching.
- The LSP module converts diagnostics to LSP-shaped data; the CLI does not run a language-server process.
- The newer v2 language is selected explicitly and has its own syntax and commands.

See [known limitations][4] for additional detail and current caveats.

## Documentation

- [Getting started][5]
- [Language tour][1]
- [Language reference][2]
- [Repair and MCP guide][3]
- [Known limitations][4]

## References

[1]: ./docs/tour.md "Klang language tour"
[2]: ./docs/reference.md "Klang language reference"
[3]: ./docs/repair.md "Klang repair and MCP guide"
[4]: ./docs/limitations.md "Klang known limitations"
[5]: ./docs/getting-started.md "Klang getting started"

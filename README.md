# Klang

A small compiled language with structured diagnostics, built to be
called by AI coding harnesses — not to be one.

Klang itself never calls a model. Your harness (Cline, OpenCode,
Claude Code, ...) owns the model connection; Klang exposes
verification as MCP tools the harness calls inside its own agent
loop: `klang_check`, `klang_run`, `klang_fmt`, `klang_scope_plan`.

```sh
cargo build
cargo run -- mcp        # MCP server on stdio (initialize/tools/list/tools/call)
cargo run -- check prog.klang
cargo run -- run prog.klang
cargo run -- repair prog.klang --dry-run   # inspect the repair prompt
```

- `docs/` — user documentation: getting started, language tour,
  reference (every diagnostic code), repair/MCP guide, limitations.
  Every code sample is machine-verified (`scripts/verify_docs.py`).
- `SPEC.md` — the language and toolchain spec, code-verified.
- `AUDIT.md` — the running source of truth: what was audited, found,
  and fixed, with test evidence.
- `examples/` — runnable programs, including `eval.klang` (lexer /
  parser / evaluator combining generics, enums, modules, effects).
- `bench/run_live.sh` — harness-signal check: diagnostic family +
  planned scope per corpus task, zero model calls.

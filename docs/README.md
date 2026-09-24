# Klang Documentation

User documentation for the Klang programming language and toolchain,
sourced from what the compiler actually does. Every `.klang` sample in
these pages is a complete program verified against the real
`klang check` / `klang run` binary (see the verification note below),
not invented syntax.

- [Getting started](getting-started.md) — build from source, first
  program, CLI subcommands.
- [Language tour](tour.md) — guided walkthrough with runnable examples.
- [Language reference](reference.md) — precise per-construct rules and
  every diagnostic code with a minimal triggering example.
- [`klang repair` guide](repair.md) — the repair loop harnesses drive:
  `klang mcp` tools, `--dry-run` prompt inspector, scope model.
- [Known limitations](limitations.md) — what Klang does not do yet,
  pulled from the audit's triaged findings.

Related internal docs (design/verification history, not user docs):
`../SPEC.md`, `../architecture.md`, `../AUDIT.md`.

## Sample-verification discipline

Code blocks tagged ` ```klang ` carry a machine-checked directive on
their first line:

- `// @check-ok` — must pass `klang check` with zero diagnostics.
- `// @run prints: a | b ; return: N` — must run; stdout lines and the
  entry return value must match exactly.
- `// @check-fail E-CODE` — must fail `klang check` with `E-CODE` among
  the diagnostics.

`scripts/verify_docs.py` extracts every block and enforces this. Zero
samples are marked unverified.

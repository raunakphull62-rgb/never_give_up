> merged into shared docs in S8.

# Wave1 S2 notes (not shared docs — staged text for reference.md)

## Diagnostic classes fixed (real file + byte start/end + 1-based line/column)

All JSON keeps existing `file`/`start`/`end` fields and ADDS `line`/`column`
inside `primary_span` (1-based; `0,0` means the explicit no-location form).
`Span::no_location(file)`, `Diagnostic::error_with_location`,
`Diagnostic::with_location`, `diagnostics::line_col_for` are the helpers.

- `E-PARSE` (v1 parser + lexer, incl. imported-file parse errors via the
  loader passthrough): `Parser` stores its source text; `err()` and
  `lex_with_comments` build via `error_with_location` (real file from
  `new_with_file`, byte span from tokens, line/col from source).
- `E-IO-NOT-FOUND` (failed imports: unreadable file, bare-package
  unresolved/missing-lock/missing-entry, vendor fallback miss): `ImportError`
  now carries `file/start/end/line/column`; `import_span(importer, raw)`
  points at the quoted `"raw"` literal in the importing file
  (`line_col_for` for line/col). Entry-itself-missing uses file=entry path
  with line 0 (no source to point at).
- `E-IMPORT` (unsafe `..`/absolute/`~` paths, missing selective name,
  `mod`-as-import, alias-unknown): same `with_import_span` at the import
  literal in the importer.
- `E-DUPLICATE` (loader whole/alias duplicates via `insert_fn` def span;
  alias re-alias and alias/`let` collisions via import literal; HIR
  function/param/struct/enum/variant/field/type-param duplicates via
  `duplicate()` token search): real file from `decl.file` (per-item
  `eff_file`) + first `` `name` `` occurrence in that file's text.
- `E-UNDEFINED` (HIR variable + modules undefined): real caller file +
  variable-name span (`with_token_span`).
- `E-ARITY` (HIR call `arity_mismatch` preferring the call-site `name(`
  spelling; `method_arity`, enum-payload, struct-missing, match-binding):
  real file + callee/method/variant/struct token.
- `E-TYPE` (all `type_mismatch` sites incl. operators, conditions, match,
  builtins, generics via `generic_mismatch`): real file + backticked token
  from `what` (or first word for `if`/`while` conditions); `* return`
  errors prefer the `return` keyword span; `* arg` errors prefer the call
  site. D2 strict (`--strict`/`[project] strict`) flows through the same
  `type_mismatch` path, so strict `E-TYPE` carries the same real spans
  (`if`/`while`/`&&`/`||`/`!` keywords).
- `E-MATCH-EXHAUSTIVE` (points at `match`), `E-MATCH-DUPLICATE`,
  `E-MATCH-UNREACHABLE`, `E-RESERVED-FIELD`, `W-SHADOW`: real file + token.
- Already-real spans now also carry line/col (filled by the fixup when
  `start/end` exist but line is 0): `E-EFFECT-MISMATCH` (caller `name_span`),
  `E-TASK-CANCEL`/`E-SPAWN-OUTSIDE-GROUP` (`let` span), `E-PRIVATE`
  (module head). The HIR `fixup_no_location_spans` preserves existing
  `start/end` (e.g. `return` statement spans) and only fills line/col from
  the file text; it searches for a token only when `start==end==0`.
- D3 boundary `E-RUNTIME` (param/return/assignment/field/construction via
  `check_int_boundary_in_file`): `MirFunction.file` threads
  `FunctionDecl.file` through lowering (closures inherit the enclosing
  file); the error names that file and spans the ctx's backticked token
  (call-site `name(` preferred for `arg`).
- MCP `klang_check`: new optional `file` arg (schema: `source` required,
  `file` optional). With `file`, checks via
  `load_program_with_entry_source(path, source)` (local/alias/package
  imports through `klang.toml`/`klang.lock`/`.klang_pkgs`, same strict rule
  as CLI) and returns per-file diagnostics; unresolved imports become one
  `E-IO-NOT-FOUND` with the import-literal span. Without `file`, single-file
  behavior unchanged. Never runs program code.

## Remaining without a location (explicit `line 0, column 0`, with reasons)

- In-memory HIR checks with file `input.warden` (e.g. `Parser::new` +
  `TypedHIR::check` without a real path): the checker receives a `Program`
  without source text and `input.warden` exists on no disk, so token search
  has nothing to read. Callers that want spans must parse with
  `new_with_file(src, real_path)` (file on disk) or load via imports.
  (Parser `E-PARSE` for the same buffers DOES carry line/col, since the
  parser holds its source.)
- Entry file itself missing (CLI `klang check/run <missing>`): file is the
  given entry path, `0,0,line0`. No source text exists to point into.
- Most runtime failures (`E-RUNTIME` division-by-zero/overflow/index-OOB/
  missing-key, `E-OVERFLOW`, dynamic `E-TYPE` from builtins, `E-TASK-GROUP`
  aggregates, `E-SPAWN-POSITION` at runtime, jump validation): file
  `runtime`, line 0. The interpreter tracks values/MIR, not source
  positions; MIR `origin`s are `NodeId`s without byte offsets, and no
  source map survives lowering. Only D3 boundaries (above) recovered a file
  via `MirFunction.file`.
- `E-CANCELLED`, `E-EXIT` (cancellation/exit control flow): file `runtime`,
  line 0. Not source errors by construction.
- Library-only codes never from surface syntax: `E-CONTRACT`,
  `E-OWNERSHIP-MODE` (file `input.warden`, line 0).
- v2 paths (`input.v2`, flow/echo/schema diagnostics with `0,0`): v2 lexer/
  checker not re-threaded in S2 (separate language track).
- `insert_renamed` hash-collision duplicate (unreachable `@` internal
  names): file `import`, line 0. No source spelling exists.
- Salsa `db.rs` file-not-found (`parse_error(name,0,0)`): file is the
  missing path, line 0. No text to span.
- `klang fetch`/registry resolution failures in `main.rs` (`E-IMPORT`,
  file `import`, line 0): no single import statement owns the failure.

## Exact text to add to reference.md (staged; do NOT edit reference.md here)

Replace the §10 preamble line:

> Check-time codes first (all verified by the sample shown), then
> run-time codes. Every HIR diagnostic below carries a `0,0` span except
> where noted — see [limitations](limitations.md).

with:

> Check-time codes first (all verified by the sample shown), then
> run-time codes. Every diagnostic carries `primary_span.file` (the
> defining file; for merged imports, the file that defines the offending
> item — not always the entry file), byte `start`/`end`, and 1-based
> `line`/`column` for the offending token (import errors point at the
> import statement's string literal in the importing file). `line 0,
> column 0` is the explicit no-location form for errors with no source
> position (missing command-line file, runtime position lost,
> library-only codes) — never a fake `0,0` masquerading as position.
> See [limitations](limitations.md) for the remaining no-location list.

And in §10 after the catalog, add:

> Span contract (S2): `start < end`, `end <= file bytes`, `(line, column)
> == line_col(file_text, start)`, and `file_text[start..end]` contains the
> offending token (function/variable/keyword/literal). `klang_check` with
> `file` follows imports and reports each diagnostic in its own file.

## Exact text to add to limitations.md F7 (staged)

Replace the F7 paragraph with:

> Spans are real (S2): check-time diagnostics (parse, import, type,
> strict, match, duplicate, D3 boundaries via MIR files, and MCP
> `klang_check` with `file`) carry the defining file, byte offsets, and
> 1-based line/column verified against the file text. Explicit
> no-location (`line 0, column 0`) remains for: in-memory checks without a
> real path, a missing command-line file, most runtime failures where the
> position was lost in lowering, `E-CANCELLED`/`E-EXIT`, library-only
> codes, v2 paths, and registry-fetch failures. See
> `docs/progress/S2-notes.md` for the per-code list.

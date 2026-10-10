# S3 notes — formatter and language server (Wave1 S3)

Branch: `wave1-s3`. Target dir: `CARGO_TARGET_DIR=/tmp/target-s3`
(official); iteration used `/tmp/target-s3-quick`.
Baseline: `cargo test --locked` → **803 passed, 0 failed**.

S3b needed no product change: `klang lsp` already implements the full S3b
contract (stdio `Content-Length` framing, hand-rolled JSON via `src/mcp.rs`,
full-sync diagnostics with real spans, loud `MethodNotFound` for
hover/definition). S3b work here is a new end-to-end gate plus editor docs.
S3a needed real work: the old `fmt` deleted every comment (the lexer
discarded them) and the CLI only handled one file with `--write`.

## 1. Canonical style (as implemented by `klang::fmt::fmt_source`)

- 4-space indent. Opening braces (`fn`, `mod`, `task_group`, `if`/`else`,
  `while`, `for`, `try`/`catch`, closures) indent the body one level; the
  closing brace sits at the parent indent.
- One statement per line. Single-expression match arms stay inline
  (`Pat => expr,`); multi-statement arms use a `{ ... }` block whose last
  line is the trailing value expression.
- Binary operators fully parenthesized (`(a + b)`); re-parsing drops the
  parens and re-emitting restores them, so this is stable.
- Single spaces around operators and after `,`/`:`; canonical signature
  shape `fn name(a: T, b: U) -> R throws async {`.
- Item order: plain imports, `as`-alias imports, selective imports, blank
  line; structs, enums, mods; blank line; functions separated by blank
  lines. (Source order across kinds is NOT preserved: the AST groups by
  kind. Within one item and within one function body, order is preserved.)
- Strings re-escaped canonically (`\"`, `\\`, `\n`, `\t`); whole floats
  keep `.0`; `;` statement terminators are dropped (accepted, ignored).
- Normalizing spellings (same AST, pinned by gates): omitted `->` prints
  explicit `-> void`; explicit tuple payload `V(f0: T)` prints `V(T)`;
  empty-arg `m::f()` / `E::V()` prints `m::f` / `E::V` (both spellings parse
  to the same `EnumCtor` with no args).
- Comments (never deleted — see §2):
  - Standalone (only whitespace before the comment on its source line) goes
    on its own line(s) before its anchor code line, at the deeper of the
    two neighboring code indents. Multi-line block bodies are verbatim
    (ASCII art survives); `//` lines are stripped of trailing spaces/tabs
    and `\r`.
  - Trailing (code before the comment on its source line) moves to the end
    of its anchor code line with exactly one space. (A comment that shared
    a line with code that the formatter splits across lines becomes a
    standalone line — still attached to the same anchor.)
  - Comments after the last token (footers) go to file end at top-level
    indent; a comment-only file formats to its comments in order.
- Every non-empty formatted file ends with exactly one `\n` (empty input
  formats to empty output).
- Guarantees: `fmt(fmt(x)) == fmt(x)`; `parse(fmt(x)) == parse(x)` ignoring
  spans/ids/omitted-arrow; runnable files print identical output and return
  identical results before/after formatting.

## 2. How comments survive (lexer trivia + segmented alignment)

- The lexer (`src/parser/mod.rs::tokenize`) now collects `Comment { start,
  end }` trivia in the same string-aware scan ( Offsets are byte offsets;
  `//` inside `"..."` is still a string, never a comment). `Parser` behavior
  is unchanged (it ignores the trivia); new public API
  `lex_with_comments(source, file) -> (tokens, comments, lex_error)`.
- `fmt_source(source, file)` parses, renders the AST with the unchanged
  `fmt_program`, then re-attaches every comment. Mapping is a segmented
  token alignment (`src/fmt.rs`): both streams are split into top-level
  items (and mod members) at brace depth 0, matched by key (`fn:name`,
  `mod:name`, `import:path`, …; k-th occurrence on collision), and mapped
  greedily within each pair with confirmed lookahead for the four known
  fmt edits (`;` drop, added parens, added `-> void`, collapsed `fK: T`,
  dropped empty `()`). Unmappable comments fall back to file end in order.
  The mapping is a fixpoint, which is what makes second-format output
  byte-identical. Comment-free sources return byte-identical
  `fmt_program` output (fast path; all pre-existing `fmt_program` tests
  and the `klang_fmt` MCP test are untouched).

## 3. CLI (`klang fmt [--write | --check] <file|dir> [paths...]`)

- No flag, one file: print formatted source to stdout (backcompat,
  including printing already-formatted files verbatim).
- `--write`: rewrite every selected file that would change; prints
  `fmt: wrote <path>` per rewritten file; unchanged files are silent.
- `--check`: never writes; prints `fmt: would change <path>` per file
  that would change and exits 1 when any would (prints `fmt: clean` and
  exits 0 otherwise).
- Directories expand recursively to `*.klang` in sorted order (`.git`
  skipped) and require `--write` or `--check`; several paths may be given.
- Bad syntax: `parse: FAIL` + the diagnostic JSON on stdout, exit 1, file
  untouched (nothing is ever written before a successful format).
- Exit codes: 0 ok/clean, 1 would-change/parse/IO error, 2 usage error
  (no paths, `--write` + `--check` together, unknown `--...` flag,
  directory without a mode, several files without a mode).

## 4. Exact text proposed for `docs/reference.md` (NOT applied — shared doc)

```md
## 12. Formatter and language server

`klang fmt [--write | --check] <file|dir> [paths...]` prints or rewrites
the canonical form: 4-space indent, one statement per line, binary
operators fully parenthesized, exactly one trailing newline.
`fmt(fmt(x)) == fmt(x)`; formatted code re-parses to the same AST
(ignoring spans) and runs identically. Comments are preserved (standalone
comments keep their anchor at the deeper neighboring indent, trailing
comments move to end of line with one space). `--check` prints
`fmt: would change <path>` per file that would change and exits 1 when
any would (`fmt: clean`, 0, otherwise). Bad syntax prints `parse: FAIL`
plus the diagnostic JSON and leaves the file untouched.

`klang lsp` is a stdio language server (`Content-Length` framing):
diagnostics-as-you-type with the exact `klang check` pipeline and real
line/character positions (UTF-16 units). Full-sync only (`didOpen`,
`didChange` with full text, `didClose`); `publishDiagnostics` carries the
check diagnostic JSON in each `data` field. `textDocument/hover` and
`textDocument/definition` answer `MethodNotFound` (-32601, planned v2);
unknown methods never hang or crash the server.
```

## 5. Editor setup

All three run the same server (`klang lsp` on stdio, `Content-Length`
framing, `textDocumentSync: 1` full). Klang files use the `.klang`
extension. Hover and go-to-definition are planned v2 (the server answers
`MethodNotFound`, so clients just show "unsupported").

VS Code (generic LSP client, e.g. `llvm-vs-code-extensions.vscode-clangd`
-style generic client or `vscode-languageclient`): command `klang lsp`,
document selector `language: klang` / `scheme: file`, no initialization
options. Minimal `clientOptions`: `documentSelector: [{ scheme: 'file',
language: 'klang' }]`, `synchronize` off (full sync is push-only).

Neovim (built-in LSP, no plugins):
```lua
vim.api.nvim_create_autocmd('FileType', {
  pattern = 'klang',
  callback = function(args)
    vim.lsp.start({
      name = 'klang',
      cmd = { 'klang', 'lsp' },
      root_dir = vim.fs.dirname(vim.api.nvim_buf_get_name(args.buf)),
    })
  end,
})
vim.filetype.add({ extension = { klang = 'klang' } })
```
Diagnostics appear as-you-type via `publishDiagnostics`; `:lua
vim.diagnostic.open_float()` shows the checker message (the full `klang
check` JSON is in `data`).

Helix (`~/.config/helix/languages.toml`):
```toml
[[language]]
name = "klang"
scope = "source.klang"
file-types = ["klang"]
comment-token = "//"
language-servers = ["klang-lsp"]

[language-server.klang-lsp]
command = "klang"
args = ["lsp"]
```
Then `:lsp-diagnostics` / `]d` shows the published diagnostics.

## 6. Verification notes (for the final report)

- Baseline: 803 passed, 0 failed (`/tmp/s3-baseline.log`).
- New: `tests/s3_gates.rs`, 11 tests (9 fmt + 1 run-equivalence + 1 LSP
  e2e); all use unique temp dirs (pid + atomic counter).
- Corpus run-equivalence skips files with `import`, without `fn main`,
  failing check, or touching network/files/stdin/sleep/process
  (`http_*`, `write_file`, `append_file`, `remove_file`, `read_line`,
  `run_process`, `time_sleep`); skipped files are counted, never failed.
  Runnable outcomes compare `(ok, value-render-or-error-code, stdout)`.
- Two honest-attempt search notes: greedy token alignment alone cannot
  survive `fmt_program`'s kind-grouping (fixed with segmented alignment,
  first attempt); `m::f()` → `m::f` is a pre-existing spelling
  normalization, verified meaning-preserving by the AST gates (not
  changed).
- BLOCKED: none. Everything committed; per-commit clean-checkout results
  in `docs/progress/S3.md`.
```

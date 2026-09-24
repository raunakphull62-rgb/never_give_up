# Repair guide: the loop harnesses drive

`klang repair` used to close the loop itself: given failing source, it
called a model, spliced the answer back, and re-checked. That assumed
Klang needed its own model connection — wrong. You already run a
harness (Cline, OpenCode, Claude Code, ...) that owns the model,
provider config, and keys. So Klang stopped dialing out and instead
exposes the verification mechanism for your harness to call. The
mechanism is unchanged — check → structured diagnostics → scoped
splice → re-check — only who calls it changed.

## The loop, as your harness runs it

1. Send the failing source to `klang_check` (see `klang mcp` below).
   Back comes `Diagnostic[]` as JSON — empty means clean.
2. Feed source + diagnostics to `klang_scope_plan`. Back comes the
   minimal edit scope: named functions, or file scope with the reason.
3. Your model regenerates just that scope; you splice it back and
   re-check. Repeat until clean or the budget is gone.

Every signal in that loop is compiler output, so it is deterministic
given the same source: no clock, no randomness, no hidden state.

## `klang mcp`: the tools

One JSON-RPC message per line on stdin, one per line on stdout:

```sh
cargo run -- mcp
```

| Tool | Input | Output |
|---|---|---|
| `klang_check` | source string | `diagnostics` array — the exact `Diagnostic::to_json()` objects (empty = clean) |
| `klang_run` | source, optional entry fn | `stdout` + `return_value`, or the check diagnostics first if it fails |
| `klang_fmt` | source | canonical source, or the parse diagnostic |
| `klang_scope_plan` | source + diagnostics (exactly as `klang_check` returned them) | `function` scope with names, or `file` scope with the reason |

Deliberately absent: any tool that takes a task description and calls
a model. That is the harness's job, using these tools as its feedback
signal — the same way it would use any other compiler's diagnostics.

`klang_run` executes with a 30-second bound: a non-terminating program
yields a timeout error, never a hung server.

## `klang repair --dry-run`: the prompt inspector

The `repair` subcommand no longer performs repairs — it shows the
exact signal a harness would send. Given `broken.klang`:

```klang
// @check-fail E-TYPE
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    return add("hi", 1)
}
```

```sh
cargo run -- repair broken.klang --dry-run
# repair: DRY-RUN (no model calls)
# scope: function(s): add
# --- system prompt ---
# ...
```

Flags: `--max-iters N` (1–10, default 5) and `--scope
function|file` shape the planned prompt; `klang.toml [repair]` may
set the same two keys (`max_iters`, `scope`) — the retired
`endpoint`/`model`/`api_key`/`timeout` keys are ignored. Anything but
`--dry-run` on a failing file errors with a pointer to `klang mcp`;
a clean file exits 0 with `check: OK (0 diagnostics), nothing to
repair`.

## The scope model

Repair edits target the smallest unit that can express the fix:

- **Function scope** (default): diagnostics attributed to ≤2 functions
  mean regenerating just those `fn` blocks, spliced back
  byte-identically around everything untouched. Attribution layers
  real spans (where they exist), name/use-site analysis, and `match/*`
  diagnostics preferring the function that contains the `match`.
- **File scope**: used when diagnostics span more than 2 functions,
  when attribution fails, or when file scope is forced.
- **Declaration-level diagnostics always take file scope**: rules
  `names/duplicate`, `modules/visibility`, and `ownership/mode` can
  only be fixed by editing declarations (`enum`/`struct`/`mod`/privacy
  markers), which function-scoped splicing cannot express — so the
  planner escalates instead of guessing. A response containing
  declaration text is accepted only as a complete file (every original
  function *and* declaration present); an incomplete one fails loudly
  rather than silently dropping the declaration edit.

After every splice the **full program** is re-checked, not just the
regenerated scope — a fix that cures one diagnostic while breaking a
caller is correctly reported as still failing, and attempt history
(what was already tried) feeds into later prompts so the same mistake
is not repeated.

## What the harness sees

Each attempt's user prompt contains the scope's current source, every
diagnostic verbatim as `Diagnostic::to_json()` plus `FixIt` hint
labels, the full file for context, and prior-failure summaries.
`--dry-run` prints exactly this with zero model calls — audit what
your harness would send before it sends it.

## Transparency and bounds

- Hard iteration ceiling: 10 regardless of configuration.
- Nothing in Klang makes network calls. There are no endpoints to
  configure and no keys to leak — that entire surface was removed,
  not secured.
- Every `run_repair` outcome carries the full attempt history
  (diagnostics in, prompts sent, raw response, diagnostics out);
  `write_log` persists it as `.klang-repair-log.json` for harness and
  library users who want it on disk.
- `check`/`run`/`build`/`fmt` behavior is unchanged by any of this; a
  file that already checks clean exits 0 with `check: OK (0
  diagnostics), nothing to repair`.

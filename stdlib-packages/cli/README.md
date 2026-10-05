# cli

Spec-driven argument parsing over an argv array (long options only).
Pure parsing — `cli_parse` takes the array, so tests never need a
process; `cli_args()` reads the live program arguments.

## API

- `cli_spec(prog)`, `cli_flag(spec, name, desc)`,
  `cli_option(spec, name, desc)`, `cli_command(spec, name, desc)` —
  value semantics, return the updated spec.
- `cli_parse(spec, argv) -> map` — `{ok, flags: map, options: map,
  command: str, positionals: array, help: 0/1, error: str}`.
  Grammar: `--flag`, `--opt value`, `--opt=value`, positionals, `--`
  (rest is positional). `--help` sets `help=1`. A `-x` token is an
  error (long options only; pass values with `=` when they start with
  `-`, e.g. `--n=-1`). Lone `-` is positional. When subcommands are
  declared the first positional must name one, else `ok=false`.
- `cli_help(spec) -> str` — auto `Usage:` text.
- `cli_args() -> array` — live `args()`.
- `cli_get_flag(parsed, name) -> i32` (missing → 0),
  `cli_get_option(parsed, name, default) -> str`,
  `cli_positionals(parsed) -> array`, `cli_command_of(parsed) -> str`.

## Test

```sh
klang run cli_test.klang
```

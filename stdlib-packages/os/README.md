# os

Environment and process helpers over the `env` / `run_process`
builtins (see `../BUILTINS.md`).

## API

- `os_env(name) -> str` — value or `""` when unset.
- `os_env_or(name, default) -> str` — `default` when unset or empty
  (unset and empty are indistinguishable — same as the builtin).
- `os_run(cmd, args) -> map` — `{stdout, stderr, exit_code}`.
- `os_run_ok(cmd, args) -> bool` — `exit_code == 0`.
- `os_which(name) -> str` — absolute path via POSIX `sh -c
  "command -v <name>"`, or `""` when not found, when `sh` is missing,
  or when the name contains shell metacharacters (space `; & | $`) or
  is empty. POSIX-only: on non-POSIX hosts it returns `""`.
- `os_args() -> array<str>` — program arguments after the `.klang`
  file (`klang run prog.klang -- a b` → `["a","b"]`; `[]` otherwise,
  including under MCP/test hosts).
- `os_exit(code) -> i32` — terminates the whole program with `code`
  (low 8 bits at the OS); uncatchable, never returns. Deliberately
  NOT exercised in the self-test (it would kill the runner) — covered
  by `tests/sysdata_gates.rs`.
- `os_cwd() -> str` — current working directory.
- `os_set_env(name, value) -> i32` — sets a variable for this process
  and its future children (`E-ENV-INVALID` on `=`/NUL names).

## Failure convention

Spawn failures (`E-PROCESS-NOT-FOUND` / `E-PROCESS-FAILED`) propagate
from `os_run` / `os_run_ok`. A non-zero exit is a normal result, never
an error. `os_which` never fails.

## Test

```sh
klang run os_test.klang
```

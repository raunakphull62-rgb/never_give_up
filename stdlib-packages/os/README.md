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

## Failure convention

Spawn failures (`E-PROCESS-NOT-FOUND` / `E-PROCESS-FAILED`) propagate
from `os_run` / `os_run_ok`. A non-zero exit is a normal result, never
an error. `os_which` never fails.

## Test

```sh
klang run os_test.klang
```

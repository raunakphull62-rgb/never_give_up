# S6 notes — process, time and environment builtins (Wave1 S6)

## Exact shape

`spawn(cmd, args)` — argv-array spawn, no shell. Returns a map with
exactly three keys (in this order):

```klang
let r = spawn("echo", ["hi"])
r["status"]  // Int: exit code (e.g. 0; -1 when killed by signal/unknown)
r["stdout"]  // Str: captured stdout (lossy UTF-8)
r["stderr"]  // Str: captured stderr (lossy UTF-8)
```

Example (`spawn("echo", ["hello world", "it's"])`):

```
status = 0
stdout = "hello world it's\n"
stderr = ""
```

`run(cmd, args)` — argv-array spawn with stdin/stdout/stderr inherited
from the parent. Returns only the exit code as Int (`0` for
`run("true", [])`, `1` for `run("false", [])`, `3` for
`run("sh", ["-c", "exit 3"])`, `-1` on signal). Captured output is
always empty (nothing is piped).

`now_ms()` — Int millis since the Unix epoch (e.g. `1791608514435`).
Far exceeds the `i32` checked range (like `file_size` past 2 GiB):
flows as a value, `E-OVERFLOW` on arithmetic.

`now_iso()` — Str UTC RFC 3339 (`YYYY-MM-DDTHH:MM:SS.mmmZ`, e.g.
`2026-10-10T05:01:54.435Z`, always 24 chars, trailing `Z`).

`sleep_ms(n)` — Int `1` on success. `n` (Int widens, Float allowed for
dynamic values) must be finite and `>= 0`; negatives/`NaN`/infinite
are `E-TIME-INVALID` (e.g. `sleep_ms(0 - 5)` →
``sleep_ms failed: sleep duration -5ms is negative``).

`get_env(name)` — Str value, or `""` when unset (no null in the
language; same absence convention as `env`). Never an error. Reads
only the named variable; there is no list/dump API.

Missing binaries (`spawn`/`run`) are `E-PROCESS-NOT-FOUND` (permission
denied likewise; other OS refusals are `E-PROCESS-FAILED`), catchable
with `try`/`catch` (`e["code"]` is the code). Non-zero exits are normal
`Ok` results, never errors. Parser note: `spawn(cmd, args)` with 0/1/2/3+
args parses as the builtin Call (arity checked); only `spawn (f())`
(single Call inside) stays the task-group `Spawn`. `fn spawn` cannot
parse (keyword-reserved, stronger than `W-SHADOW`); `fn run`/`fn now_ms`
etc. are `W-SHADOW`.

JIT/v2: all six are interpreter-only. The JIT rejects each loudly via
the existing `is_builtin` arm (``builtin `spawn` unsupported`` etc.);
v2 has no such builtins (`E-UNDEFINED`/`E-PARSE-V2`). No allowlist change
was needed (no parity-corpus program uses them).

## Security — exact limits and caller responsibilities

Limits (what S6 does NOT do):

- No shell, ever: `Command::new(cmd).args(args)` (argv array). No
  `sh -c` string building, no interpolation, no globbing (`*`/`?` are
  literal), no `~`/env expansion, no `PATH` searching beyond the OS
  default. To run shell syntax you must explicitly spawn `sh` with
  `["-c", "..."]` — the danger is then visible at the call site.
- No timeout: `spawn`/`run` block until the child exits; `sleep_ms`
  blocks until done. A hung child hangs the caller (same as
  `run_process`/`time_sleep`). Callers needing bounds must wrap at a
  higher layer (see D7 `timeout_ms` recommendation).
- No input feeding: `spawn` captures via `output()` (stdin closed);
  `run` inherits stdio (no piping). There is no `stdin:` argument.
- No output caps: `spawn` buffers all of stdout/stderr into memory
  (lossy UTF-8). A child printing gigabytes exhausts memory — same as
  `run_process`. Callers must only spawn trusted, bounded-output
  programs or add their own cap/timeout layer.
- No env listing: `get_env` reads one named variable; `env` likewise.
  There is no dump-all API. `set_env` still validates `=`/NUL
  (`E-ENV-INVALID`).
- `now_ms`/`now_iso` are wall-clock (`SystemTime`); not monotonic. Do
  not use for ordering across clock adjustments; `sleep_ms` duration
  is still honored (thread sleep).
- `run` inherits stdio: child output interleaves with the parent's
  (noisy under `cargo test`). Prefer `spawn` when you need capture.

Caller responsibilities:

1. Pass argv as a list, never build a shell string (`spawn("grep",
   [pat])`, not `spawn("sh", ["-c", "grep " + pat])`).
2. Treat `status`/`run()` codes as untrusted input (check non-zero;
   `-1` means signal).
3. Never pass secrets as argv (visible in `ps`); prefer env files or
   stdin (not yet piped — stage via files for now).
4. Only spawn bounded-output programs (or accept OOM on hostile output).
5. Only read env vars you need; unset is `""` — distinguish "empty" vs
   "missing" at a higher layer if it matters (prefix/sentinel).
6. Expect `E-PROCESS-NOT-FOUND` on missing binaries (catch it; do not
   let it escape as a crash — it never panics).

## Exact text for reference.md (to merge)

In the free-functions table (§5), after the `run_process` row, add:

| `spawn(cmd, args)` | 2 | argv-array spawn, no shell; returns map `status`/`stdout`/`stderr` (`status` is Int exit code, `-1` on signal; non-zero is a normal result, missing binary is `E-PROCESS-NOT-FOUND` catchable via try/catch) |
| `run(cmd, args)` | 2 | argv-array spawn with stdio inherited; returns Int exit code only (`-1` on signal; missing binary is `E-PROCESS-NOT-FOUND`) |
| `now_ms()` / `now_iso()` / `sleep_ms(n)` | 0 / 0 / 1 | millis since epoch (Int, past-i32 like `file_size`); UTC RFC 3339 Str (`YYYY-MM-DDTHH:MM:SS.mmmZ`); blocking ms sleep returning 1, negative/NaN/inf is `E-TIME-INVALID` |
| `get_env(n)` | 1 | env var or `""` when unset (same absence convention as `env`; never an error) |

Parser note (§3/§9): `spawn(cmd, args)` (0/1/2/3+ parenthesized args)
is the process builtin Call; only `spawn (f())` (single Call inside)
is task-group spawn. `fn spawn` does not parse (keyword).

## Exact text for limitations.md (to merge)

Add under Design-scope cuts (or a new `S6 process/time/env` bullet):

- **Process/time/env (S6)**: `spawn`/`run` are argv-only (no shell, no
  glob, no timeout, no stdin piping); `spawn` buffers all output in
  memory (no cap — hostile gigabytes OOM like `run_process`); `run`
  inherits stdio. `now_ms` exceeds `i32` (flows as Int, `E-OVERFLOW` on
  arithmetic); `now_iso` is wall-clock UTC. `get_env`/`env` absence is
  `""` (no null). All six are interpreter-only (JIT loud-rejects,
  v2 `E-UNDEFINED`).

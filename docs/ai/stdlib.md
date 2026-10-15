# Klang stdlib — every builtin (generated from code)

Source: `src/hir.rs:is_builtin` (107 flat) + `check_method_call`/`exec_method`
(14 str + 6 array + 3 map methods). If a builtin exists in code but not
here, the gate fails. Each entry lists exact arity, return type (checker
`Ty`), and error codes (checker `E-ARITY`/`E-TYPE` plus runtime codes).
Examples are executed; non-deterministic results use bounds/len checks.

## Flat builtins

| Name | Signature | Returns | Errors |
|---|---|---|---|
| `len` | `(x: str\|array\|map)` | `int` | `E-ARITY`, `E-TYPE` |
| `push` | `(a: array var, v: unknown)` | `int` (new len) | `E-ARITY`, `E-TYPE` (temp), `E-RUNTIME` |
| `pop` | `(a: array var)` | `unknown` (elem) | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (empty) |
| `range` | `(lo: int, hi: int)` | `array<int>` | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (too large) |
| `str` | `(x: unknown)` | `str` | `E-ARITY` |
| `int` | `(x: int\|float\|str)` | `int` | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (parse) |
| `float` | `(x: int\|float\|str)` | `float` | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (parse) |
| `keys` | `(m: map)` | `array<str>` | `E-ARITY`, `E-TYPE` |
| `assert` | `(c: bool\|int)` | `int` (1) | `E-ARITY`, `E-RUNTIME` (failed) |
| `read_file` | `(p: str)` | `str` | `E-ARITY`, `E-TYPE`, `E-IO-NOT-FOUND`, `E-SANDBOX-FILE` (sandbox only) |
| `write_file` | `(p: str, c: str)` | `int` (bytes) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `append_file` | `(p: str, c: str)` | `int` (bytes) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `exists` | `(p: str)` | `bool` | `E-ARITY`, `E-TYPE`, `E-SANDBOX-FILE` (sandbox only) |
| `remove_file` | `(p: str)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-IO-NOT-FOUND`, `E-SANDBOX-FILE` (sandbox only) |
| `env` | `(n: str)` | `str` (`""` when unset) | `E-ARITY`, `E-TYPE` |
| `get_env` | `(n: str)` | `str` (`""` when unset) | `E-ARITY`, `E-TYPE` |
| `set_env` | `(n: str, v: str)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-ENV-INVALID` |
| `run_process` | `(cmd: str, args: array)` | `map{stdout,str;stderr,str;exit_code,int}` | `E-ARITY`, `E-TYPE`, `E-PROCESS-NOT-FOUND` |
| `spawn` | `(cmd: str, args: array)` | `map{status,int;stdout,str;stderr,str}` | `E-ARITY`, `E-TYPE`, `E-PROCESS-NOT-FOUND` |
| `run` | `(cmd: str, args: array)` | `int` (exit code) | `E-ARITY`, `E-TYPE`, `E-PROCESS-NOT-FOUND` |
| `regex_is_match` | `(pat: str, text: str)` | `bool` | `E-ARITY`, `E-TYPE`, `E-REGEX-INVALID-PATTERN` |
| `regex_find` | `(pat: str, text: str)` | `map{matched,bool;match,str;groups,array;named,map}` | `E-ARITY`, `E-TYPE`, `E-REGEX-INVALID-PATTERN` |
| `regex_find_all` | `(pat: str, text: str)` | `array<str>` | `E-ARITY`, `E-TYPE`, `E-REGEX-INVALID-PATTERN`, `E-REGEX-TOO-MANY` |
| `regex_replace` | `(pat: str, text: str, repl: str)` | `str` | `E-ARITY`, `E-TYPE`, `E-REGEX-INVALID-PATTERN`, `E-REGEX-TOO-LARGE` |
| `time_sleep` | `(s: int\|float)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-TIME-INVALID` |
| `time_now` | `()` | `float` (epoch secs) | `E-ARITY` |
| `time_elapsed` | `(since: int\|float)` | `float` | `E-ARITY`, `E-TYPE`, `E-TIME-INVALID` |
| `now_ms` | `()` | `int` (millis) | `E-ARITY` |
| `now_iso` | `()` | `str` (RFC3339) | `E-ARITY` |
| `sleep_ms` | `(ms: int\|float)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-TIME-INVALID` |
| `http_get` | `(url: str)` | `map{status,int;body,str;headers,map}` | `E-ARITY`, `E-TYPE`, `E-NET-INVALID-URL`, `E-NET-UNREACHABLE` |
| `http_post` | `(url: str, body: str, headers: array)` | `map` | `E-ARITY`, `E-TYPE`, `E-NET-*` |
| `http_get_async` | `(url: str)` | `map` | `E-ARITY`, `E-TYPE`, `E-EFFECT-MISMATCH` (needs `async`), `E-NET-*` |
| `http_post_async` | `(url: str, body: str, headers: array)` | `map` | `E-ARITY`, `E-TYPE`, `E-EFFECT-MISMATCH`, `E-NET-*` |
| `read_line` | `()` | `str` (`""` at EOF) | `E-ARITY` |
| `is_eof` | `()` | `bool` | `E-ARITY` |
| `print_raw` | `(x: unknown)` | `int` (1) | `E-ARITY` |
| `chdir` | `(p: str)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-IO-NOT-FOUND`, `E-IO-PERMISSION`, `E-IO-FAILED` |
| `sort_by` | `(a: array, key: fn(1) -> int\|str)` | `array` (copy) | `E-ARITY`, `E-TYPE` |
| `parse_int` | `(s: str)` | `int` (i32 range) | `E-ARITY`, `E-TYPE`, `E-PARSE-INT` |
| `parse_float` | `(s: str)` | `float` | `E-ARITY`, `E-TYPE`, `E-PARSE-FLOAT` |
| `format` | `(...: unknown)` | `str` (space-joined) | — (variadic, infallible) |
| `sprintf` | `(fmt: str, args: array)` | `str` (`{}`, `{{`/`}}`) | `E-ARITY`, `E-TYPE`, `E-FMT-INVALID`, `E-FMT-ARG-MISMATCH` |
| `insert` | `(a: array var, i: int, v: unknown)` | `int` (new len) | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (OOB) |
| `args` | `()` | `array<str>` (no prog name) | `E-ARITY` |
| `exit` | `(code: int)` | — (exits, `E-EXIT`) | `E-ARITY` |
| `cwd` | `()` | `str` | `E-ARITY` |
| `list_dir` | `(p: str)` | `array<str>` | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `make_dir` | `(p: str)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `make_dirs` | `(p: str)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `is_dir` | `(p: str)` | `bool` | `E-ARITY`, `E-TYPE`, `E-SANDBOX-FILE` (sandbox only) |
| `is_file` | `(p: str)` | `bool` | `E-ARITY`, `E-TYPE`, `E-SANDBOX-FILE` (sandbox only) |
| `rename_file` | `(from: str, to: str)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `copy_file` | `(from: str, to: str)` | `int` (bytes) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `file_size` | `(p: str)` | `int` (bytes) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `file_mtime` | `(p: str)` | `float` (epoch secs) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-SANDBOX-FILE` (sandbox only) |
| `path_absolute` | `(p: str)` | `str` (CWD-joined, infallible) | `E-ARITY`, `E-TYPE` |
| `ord` | `(s: str)` | `int` (first char code) | `E-ARITY`, `E-TYPE`, `E-CHAR-INVALID` |
| `chr` | `(n: int)` | `str` (1 char) | `E-ARITY`, `E-TYPE`, `E-CHAR-INVALID` |
| `slice` | `(x: str\|array, lo: int, hi: int)` | `str`/`array`/`unknown` | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (bounds) |
| `sort` | `(a: array)` | `array` (copy) | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (mixed) |
| `sqrt` | `(x: int\|float)` | `float` | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (negative) |
| `pow` | `(a: num, b: num)` | `float` | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (NaN/inf) |
| `abs` | `(x: int\|float)` | `int`/`float` | `E-ARITY`, `E-TYPE`, `E-OVERFLOW` (min) |
| `min` | `(a: num, b: num)` | `int` (both int) else `float` | `E-ARITY`, `E-TYPE` |
| `max` | `(a: num, b: num)` | `int`/`float` | `E-ARITY`, `E-TYPE` |
| `floor` | `(x: num)` | `float` | `E-ARITY`, `E-TYPE` |
| `ceil` | `(x: num)` | `float` | `E-ARITY`, `E-TYPE` |
| `sin` | `(x: num)` | `float` | `E-ARITY`, `E-TYPE` |
| `cos` | `(x: num)` | `float` | `E-ARITY`, `E-TYPE` |
| `tan` | `(x: num)` | `float` | `E-ARITY`, `E-TYPE` |
| `atan2` | `(y: num, x: num)` | `float` | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (NaN) |
| `log` | `(x: num)` | `float` (ln) | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (non-positive) |
| `exp` | `(x: num)` | `float` | `E-ARITY`, `E-TYPE` |
| `random_int` | `(lo: int, hi: int)` | `int` inclusive | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (lo>hi) |
| `random_float` | `()` | `float` in `[0,1)` | `E-ARITY` |
| `random_seed` | `(seed: int)` | `int` (1) | `E-ARITY`, `E-TYPE` |
| `random_seeded_int` | `(lo: int, hi: int)` | `int` inclusive | `E-ARITY`, `E-TYPE`, `E-RANDOM-BOUNDS` |
| `random_seeded_float` | `()` | `float` in `[0,1)` | `E-ARITY` |
| `sha256` | `(s: str)` | `str` (64 hex) | `E-ARITY`, `E-TYPE` |
| `hmac_sha256` | `(key: str, msg: str)` | `str` (64 hex) | `E-ARITY`, `E-TYPE`, `E-RUNTIME` (key) |
| `uuid_v4` | `()` | `str` (36, `4xxx`) | `E-ARITY` |
| `tcp_listen` | `(host: str, port: int)` | `unknown` (Listener; port 0 = any) | `E-ARITY`, `E-TYPE`, `E-NET-ADDR-INVALID`, `E-NET-ADDR-IN-USE`, `E-SANDBOX-NETWORK` |
| `listener_port` | `(l: listener)` | `int` | `E-ARITY`, `E-TYPE`, `E-NET-CLOSED`, `E-SANDBOX-NETWORK` |
| `tcp_accept` | `(l: listener)` | `unknown` (Conn, blocking) | `E-ARITY`, `E-TYPE`, `E-NET-CLOSED`, `E-NET-IO`, `E-SANDBOX-NETWORK` |
| `tcp_accept_timeout` | `(l: listener, ms: int)` | `unknown` (Conn or null) | `E-ARITY`, `E-TYPE`, `E-NET-CLOSED`, `E-NET-IO`, `E-SANDBOX-NETWORK` |
| `tcp_connect` | `(host: str, port: int, timeout_ms: int)` | `unknown` (Conn) | `E-ARITY`, `E-TYPE`, `E-NET-ADDR-INVALID`, `E-NET-DNS`, `E-NET-REFUSED`, `E-NET-TIMEOUT`, `E-SANDBOX-NETWORK` |
| `conn_read` | `(c: conn, max: int)` | `str` (lossy UTF-8; `""` at peer close) | `E-ARITY`, `E-TYPE`, `E-NET-CLOSED`, `E-NET-TIMEOUT`, `E-NET-IO`, `E-SANDBOX-NETWORK` |
| `conn_read_line` | `(c: conn)` | `unknown` (str without newline, or null at close) | `E-ARITY`, `E-TYPE`, `E-NET-CLOSED`, `E-NET-TIMEOUT`, `E-NET-IO`, `E-SANDBOX-NETWORK` |
| `conn_read_hex` | `(c: conn, max: int)` | `str` (lowercase hex; `""` at close) | `E-ARITY`, `E-TYPE`, `E-NET-*`, `E-SANDBOX-NETWORK` |
| `conn_write` | `(c: conn, s: str)` | `int` (bytes) | `E-ARITY`, `E-TYPE`, `E-NET-CLOSED`, `E-NET-TIMEOUT`, `E-NET-IO`, `E-SANDBOX-NETWORK` |
| `conn_write_hex` | `(c: conn, hex: str)` | `int` (bytes) | `E-ARITY`, `E-TYPE`, `E-NET-IO`, `E-SANDBOX-NETWORK` |
| `conn_close` | `(h: conn\|listener\|udpsock)` | `int` (1, idempotent) | `E-ARITY`, `E-TYPE`, `E-SANDBOX-NETWORK` |
| `conn_set_timeout` | `(c: conn, ms: int)` | `int` (1; 0 clears) | `E-ARITY`, `E-TYPE`, `E-NET-CLOSED`, `E-NET-IO`, `E-SANDBOX-NETWORK` |
| `conn_peer` | `(c: conn)` | `str` (`"ip:port"`) | `E-ARITY`, `E-TYPE`, `E-NET-CLOSED`, `E-SANDBOX-NETWORK` |
| `udp_bind` | `(host: str, port: int)` | `unknown` (UdpSock; port 0 = any) | `E-ARITY`, `E-TYPE`, `E-NET-*`, `E-SANDBOX-NETWORK` |
| `udp_send_to` | `(s: udpsock, host: str, port: int, data: str)` | `int` (bytes) | `E-ARITY`, `E-TYPE`, `E-NET-*`, `E-SANDBOX-NETWORK` |
| `udp_recv_from` | `(s: udpsock, max: int, timeout: int)` | `map{data,str;host,str;port,int}` | `E-ARITY`, `E-TYPE`, `E-NET-TIMEOUT`, `E-NET-*`, `E-SANDBOX-NETWORK` |
| `dns_lookup` | `(host: str)` | `array<str>` (sorted deduped IPs) | `E-ARITY`, `E-TYPE`, `E-NET-DNS`, `E-SANDBOX-NETWORK` |
| `is_null` | `(x: unknown)` | `bool` (`1` for null) | `E-ARITY` |
| `thread_spawn` | `(fn: str, args: array)` | `unknown` (Thread) | `E-ARITY`, `E-TYPE`, `E-THREAD-CLOSURE`, `E-THREAD-UNKNOWN`, `E-THREAD-ARITY`, `E-THREAD-LIMIT`, `E-SANDBOX-THREADS` |
| `thread_join` | `(t: thread)` | `unknown` (value or `E-THREAD-FAILED`) | `E-ARITY`, `E-TYPE`, `E-THREAD-FAILED` |
| `thread_id` | `()` | `int` (0 on main) | `E-ARITY` |
| `chan` | `(cap: int)` | `unknown` (Chan; cap 1..1000000) | `E-ARITY`, `E-TYPE`, `E-CHAN-CAPACITY`, `E-SANDBOX-THREADS` |
| `send` | `(ch: chan, v: unknown)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-CHAN-CLOSED`, `E-SANDBOX-THREADS` |
| `recv` | `(ch: chan)` | `unknown` (value or null when closed-empty) | `E-ARITY`, `E-TYPE`, `E-SANDBOX-THREADS` |
| `recv_timeout` | `(ch: chan, ms: int)` | `unknown` (value or null on timeout/closed) | `E-ARITY`, `E-TYPE`, `E-CHAN-TIMEOUT`, `E-SANDBOX-THREADS` |
| `try_recv` | `(ch: chan)` | `unknown` (value or null when empty) | `E-ARITY`, `E-TYPE`, `E-SANDBOX-THREADS` |
| `close` | `(ch: chan)` | `int` (1, idempotent) | `E-ARITY`, `E-TYPE`, `E-SANDBOX-THREADS` |
| `signal_watch` | `(name: str)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-SIGNAL-UNKNOWN`, `E-SIGNAL-UNSUPPORTED`, `E-SANDBOX-SIGNAL` |
| `signal_pending` | `(name: str)` | `bool` (`1` once, clears) | `E-ARITY`, `E-TYPE`, `E-SIGNAL-UNKNOWN`, `E-SIGNAL-UNSUPPORTED`, `E-SANDBOX-SIGNAL` |
| `http_serve` | `(host: str, port: int, handler: str, opts: map)` | `unknown` (Server; port 0 = any) | `E-ARITY`, `E-TYPE`, `E-HTTP-HANDLER`, `E-HTTP-OPTIONS`, `E-NET-*`, `E-SANDBOX-NETWORK` |
| `server_port` | `(s: server)` | `int` | `E-ARITY`, `E-TYPE`, `E-HTTP-CLOSED`, `E-SANDBOX-NETWORK` |
| `http_stop` | `(s: server)` | `int` (1, idempotent) | `E-ARITY`, `E-TYPE`, `E-SANDBOX-NETWORK` |
| `http_request` | `(method: str, url: str, headers: map, body: str, timeout: int[, opts: map])` | `map{status,int;body,str;headers,map}` | `E-ARITY`, `E-TYPE`, `E-HTTP-*`, `E-SANDBOX-NETWORK` |
| `http_json` | `(status: int, v: unknown)` | `map` (response, `Null`->`null`) | `E-ARITY`, `E-TYPE`, `E-HTTP-STATUS`, `E-JSON-STRINGIFY` |
| `http_text` | `(status: int, body: str)` | `map` (response) | `E-ARITY`, `E-TYPE`, `E-HTTP-STATUS` |
| `http_redirect` | `(url: str)` | `map` (302 + location) | `E-ARITY`, `E-TYPE`, `E-HTTP-URL` |
| `route_match` | `(pat: str, path: str)` | `unknown` (map or null) | `E-ARITY`, `E-TYPE`, `E-HTTP-ROUTE` |
| `json_decode` | `(s: str)` | `unknown` (`null`->null) | `E-ARITY`, `E-TYPE`, `E-JSON-PARSE` |
| `json_stringify` | `(v: unknown, pretty: int)` | `str` (`null`->`null`) | `E-ARITY`, `E-TYPE`, `E-JSON-STRINGIFY` |
| `dt_now` | `()` | `int` (epoch millis) | `E-ARITY` |
| `dt_parse_iso` | `(s: str)` | `int` (epoch millis) | `E-ARITY`, `E-TYPE`, `E-TIME-PARSE` |
| `dt_format` | `(ms: int, pattern: str)` | `str` (UTC; `%Y %m %d %H %M %S %f %z %a %A %b %B %j %p`, `%%`) | `E-ARITY`, `E-TYPE`, `E-TIME-RANGE`, `E-TIME-PATTERN` |
| `dt_add` | `(ms: int, delta: map)` | `int` (`{days,hours,minutes,seconds,ms}`) | `E-ARITY`, `E-TYPE`, `E-TIME-RANGE` |
| `dt_diff_ms` | `(a: int, b: int)` | `int` (`a - b`) | `E-ARITY`, `E-TYPE`, `E-TIME-RANGE` |
| `dt_parts` | `(ms: int)` | `map{year,month,day,hour,minute,second,millis,weekday,yday}` (UTC) | `E-ARITY`, `E-TYPE`, `E-TIME-RANGE` |
| `dt_parts_offset` | `(ms: int, off: int)` | `map` (wall clock at fixed offset `-1080..1080`) | `E-ARITY`, `E-TYPE`, `E-TIME-RANGE` |
| `dt_from_parts` | `(y: int, mo: int, d: int, h: int, mi: int, s: int)` | `int` (UTC; year 1..9999) | `E-ARITY`, `E-TYPE`, `E-TIME-RANGE` |
| `dt_local_offset_minutes` | `()` | `int` (host offset) | `E-ARITY` |
| `dt_is_leap` | `(year: int)` | `bool` | `E-ARITY`, `E-TYPE` |
| `dt_days_in_month` | `(year: int, month: int)` | `int` (28..31) | `E-ARITY`, `E-TYPE`, `E-TIME-RANGE` |
| `gzip_compress` | `(s: str)` | `str` (lowercase hex) | `E-ARITY`, `E-TYPE`, `E-COMPRESS-TOO-LARGE` |
| `gzip_decompress` | `(hex: str)` | `str` | `E-ARITY`, `E-TYPE`, `E-COMPRESS-CORRUPT`, `E-COMPRESS-TOO-LARGE` |
| `gzip_file` | `(src: str, dst: str)` | `int` (output bytes) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-COMPRESS-*`, `E-SANDBOX-FILE` (sandbox only) |
| `gunzip_file` | `(src: str, dst: str)` | `int` (output bytes) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-COMPRESS-*`, `E-SANDBOX-FILE` (sandbox only) |
| `zlib_compress` | `(s: str)` | `str` (lowercase hex) | `E-ARITY`, `E-TYPE`, `E-COMPRESS-TOO-LARGE` |
| `zlib_decompress` | `(hex: str)` | `str` | `E-ARITY`, `E-TYPE`, `E-COMPRESS-CORRUPT`, `E-COMPRESS-TOO-LARGE` |
| `zlib_file` | `(src: str, dst: str)` | `int` (output bytes) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-COMPRESS-*`, `E-SANDBOX-FILE` (sandbox only) |
| `unzlib_file` | `(src: str, dst: str)` | `int` (output bytes) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-COMPRESS-*`, `E-SANDBOX-FILE` (sandbox only) |
| `tar_create` | `(dir: str, out: str)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-COMPRESS-CORRUPT`, `E-SANDBOX-FILE` (sandbox only) |
| `tar_extract` | `(tar: str, dir: str)` | `int` (1; 256 MB cap) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-COMPRESS-*`, `E-SANDBOX-FILE` (sandbox only) |
| `tar_extract_limit` | `(tar: str, dir: str, max: int)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-IO-*`, `E-COMPRESS-*`, `E-SANDBOX-FILE` (sandbox only) |
| `pg_connect` | `(url: str)` | `unknown` (Pg) | `E-ARITY`, `E-TYPE`, `E-PG-CONNECT`, `E-PG-AUTH`, `E-SANDBOX-NETWORK` |
| `pg_exec` | `(pg: pg, sql: str, params: array)` | `map{changes,int}` | `E-ARITY`, `E-TYPE`, `E-PG-SQL`, `E-PG-CLOSED`, `E-SANDBOX-NETWORK` |
| `pg_query` | `(pg: pg, sql: str, params: array)` | `array<map>` | `E-ARITY`, `E-TYPE`, `E-PG-SQL`, `E-PG-CLOSED`, `E-SANDBOX-NETWORK` |
| `pg_begin` | `(pg: pg)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-PG-TX`, `E-PG-CLOSED`, `E-SANDBOX-NETWORK` |
| `pg_commit` | `(pg: pg)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-PG-TX`, `E-PG-CLOSED`, `E-SANDBOX-NETWORK` |
| `pg_rollback` | `(pg: pg)` | `int` (1) | `E-ARITY`, `E-TYPE`, `E-PG-TX`, `E-PG-CLOSED`, `E-SANDBOX-NETWORK` |
| `pg_close` | `(pg: pg)` | `int` (1, idempotent) | `E-ARITY`, `E-TYPE`, `E-SANDBOX-NETWORK` |

## Methods

| Receiver | Method | Signature | Returns | Errors |
|---|---|---|---|---|
| `str` | `len` | `()` | `int` (bytes) | `E-ARITY` |
| `str` | `upper` | `()` | `str` | `E-ARITY` |
| `str` | `lower` | `()` | `str` | `E-ARITY` |
| `str` | `trim` | `()` | `str` | `E-ARITY` |
| `str` | `chars` | `()` | `array<str>` | `E-ARITY` |
| `str` | `contains` | `(sub: str)` | `bool` | `E-ARITY` |
| `str` | `starts_with` | `(pre: str)` | `bool` | `E-ARITY` |
| `str` | `ends_with` | `(suf: str)` | `bool` | `E-ARITY` |
| `str` | `split` | `(sep: str)` | `array<str>` | `E-ARITY` |
| `str` | `replace` | `(old: str, new: str)` | `str` | `E-ARITY` |
| `str` | `pad_start` | `(w: int, pad: str)` | `str` | `E-ARITY`, `E-STR-INVALID`, `E-STR-TOO-LARGE` |
| `str` | `pad_end` | `(w: int, pad: str)` | `str` | `E-ARITY`, `E-STR-INVALID`, `E-STR-TOO-LARGE` |
| `str` | `repeat` | `(n: int)` | `str` | `E-ARITY`, `E-STR-INVALID`, `E-STR-TOO-LARGE` |
| `str` | `lines` | `()` | `array<str>` | `E-ARITY` |
| `array` | `len` | `()` | `int` | `E-ARITY` |
| `array` | `push` | `(v: unknown)` | `int` | `E-ARITY`, `E-TYPE`, `E-RUNTIME` |
| `array` | `pop` | `()` | `unknown` | `E-ARITY`, `E-RUNTIME` (empty) |
| `array` | `insert` | `(i: int, v: unknown)` | `int` | `E-ARITY`, `E-RUNTIME` (OOB) |
| `array` | `contains` | `(v: unknown)` | `bool` | `E-ARITY` |
| `array` | `join` | `(sep: str)` | `str` | `E-ARITY` |
| `map` | `len` | `()` | `int` | `E-ARITY` |
| `map` | `keys` | `()` | `array<str>` | `E-ARITY` |
| `map` | `contains` | `(k: str)` | `bool` | `E-ARITY` |

## Verified examples (grouped, deterministic)

Core sequence and strings:

```klang run
fn main() -> i32 {
    print(len("hi"))
    print(len([1, 2]))
    print(len({"a": 1}))
    print(str(42))
    print(int("42"))
    print(float("3.5"))
    print(parse_int("42"))
    print(parse_float("3.5"))
    return 0
}
```

```output
2
2
1
42
42
3.5
42
3.5
```

Arrays, maps, slice, sort:

```klang run
fn main() -> i32 {
    let a = [3, 1, 2]
    print(sort(a))
    print(slice(a, 0, 2))
    print(range(0, 3))
    print(keys({"a": 1, "b": 2}))
    print("a,b".split(","))
    print(["a", "b"].join(", "))
    return 0
}
```

```output
[1, 2, 3]
[3, 1]
[0, 1, 2]
[a, b]
[a, b]
a, b
```

Math and hashes (deterministic):

```klang run
fn main() -> i32 {
    print(sqrt(9.0))
    print(pow(2.0, 3.0))
    print(abs(0 - 5))
    print(min(3, 7))
    print(max(3, 7))
    print(floor(2.7))
    print(len(sha256("x")) == 64)
    return 0
}
```

```output
3.0
8.0
5
3
7
2.0
1
```

String methods and b1 methods:

```klang run
fn main() -> i32 {
    print("hi".upper())
    print("  hi  ".trim())
    print("aaa".replace("a", "b"))
    print("hi".pad_start(5, "0"))
    print("hi".pad_end(5, "0"))
    print("hi".repeat(2))
    print("a\nb".lines())
    return 0
}
```

```output
HI
hi
bbb
000hi
hi000
hihi
[a, b]
```

Files, env, time, paths (temp-dir safe, deterministic checks):

```klang run
fn main() -> i32 {
    write_file("stdlib_tmp.txt", "hi")
    print(read_file("stdlib_tmp.txt"))
    print(exists("stdlib_tmp.txt"))
    print(file_size("stdlib_tmp.txt"))
    remove_file("stdlib_tmp.txt")
    print(env("KLANG_NOPE_XYZ") == "")
    print(time_now() > 0.0)
    print(path_absolute("a") != "")
    return 0
}
```

```output
hi
1
2
1
1
1
```

Processes, regex, uuid, seeded random:

```klang run
fn main() -> i32 {
    let r = spawn("echo", ["hi"])
    print(r["status"])
    print(regex_is_match("a+", "aaa"))
    print(regex_find_all("a", "banana"))
    print(regex_replace("a", "banana", "o"))
    print(len(uuid_v4()) == 36)
    random_seed(7)
    print(random_seeded_int(1, 10) >= 1)
    print(random_seeded_float() >= 0.0)
    print(random_seeded_float() < 1.0)
    print(ord("A"))
    print(chr(65))
    return 0
}
```

```output
0
1
[a, a, a]
bonono
1
1
1
1
65
A
```

Misc (args, cwd, chdir, format, sprintf, assert, print_raw, sort_by, dirs):

```klang run
fn main() -> i32 {
    print(len(args()) == 0)
    print(len(cwd()) > 0)
    print(chdir("."))
    print(format("a", 1))
    print(sprintf("{}!", ["hi"]))
    print(assert(1))
    print(is_eof() == true || is_eof() == false)
    print(read_line() == "")
    return 0
}
```

```output
1
1
1
a 1
hi!
1
1
1
```

S9 print_raw (no newline, returns 1) and sort_by (stable copy by key, input unchanged):

```klang run
fn main() -> i32 {
    print(print_raw("hi"))
    let words = ["pear", "fig", "apple"]
    let bylen = fn(s: str) -> i32 {
    return len(s)
}
    print(sort_by(words, bylen))
    print(words)
    return 0
}
```

```output
hi1
[fig, pear, apple]
[pear, fig, apple]
```

```klang run
fn main() -> i32 {
    make_dirs("stdlib_d/a")
    print(is_dir("stdlib_d/a"))
    write_file("stdlib_d/a/f.txt", "hi")
    print(is_file("stdlib_d/a/f.txt"))
    print(list_dir("stdlib_d/a"))
    copy_file("stdlib_d/a/f.txt", "stdlib_d/a/g.txt")
    rename_file("stdlib_d/a/g.txt", "stdlib_d/a/h.txt")
    print(exists("stdlib_d/a/h.txt"))
    remove_file("stdlib_d/a/f.txt")
    remove_file("stdlib_d/a/h.txt")
    print(file_mtime("stdlib_d/a") > 0.0)
    return 0
}
```

```output
1
1
[f.txt]
1
1
```

Error cases (no network needed):

```klang error E-NET-INVALID-URL
fn main() -> i32 {
    print(http_get("not a url !!!"))
    return 0
}
```

```klang error E-IO-NOT-FOUND
fn main() -> i32 {
    print(read_file("stdlib_nope_xyz.txt"))
    return 0
}
```

```klang error E-PARSE-INT
fn main() -> i32 {
    print(parse_int("abc"))
    return 0
}
```

```klang error E-TIME-INVALID
fn main() -> i32 {
    sleep_ms(0 - 1)
    return 0
}
```

Backend builtins (A1/A2/A3/L1/A3b) — verified

Sockets + null (check only — needs loopback at run time; pure `is_null`
runs):

```klang check
fn main() -> i32 {
    let l = tcp_listen("127.0.0.1", 0)
    print(listener_port(l))
    let c = tcp_accept_timeout(l, 0)
    print(is_null(c))
    let s = tcp_connect("127.0.0.1", 0, 0)
    print(conn_peer(s))
    conn_set_timeout(s, 0)
    print(conn_write(s, "hi"))
    print(conn_read(s, 10))
    print(conn_read_line(s))
    print(conn_read_hex(s, 10))
    print(conn_write_hex(s, "hi"))
    conn_close(s)
    conn_close(l)
    let u = udp_bind("127.0.0.1", 0)
    print(udp_send_to(u, "127.0.0.1", 0, "hi"))
    print(udp_recv_from(u, 100, 0))
    print(dns_lookup("localhost"))
    print(is_null(c))
    return 0
}
```

```klang run
fn main() -> i32 {
    print(is_null(json_decode("null")))
    print(is_null(0))
    return 0
}
```

```output
1
0
```

Threads, channels, signals (runs — no network):

```klang run
fn double(n: i32) -> i32 {
    return n * 2
}

fn main() -> i32 {
    print(thread_id() == 0)
    let ch = chan(2)
    send(ch, 5)
    print(recv(ch))
    print(recv_timeout(ch, 0))
    print(try_recv(ch))
    close(ch)
    print(close(ch))
    let t = thread_spawn("double", [21])
    print(thread_join(t))
    signal_watch("INT")
    print(signal_pending("INT") == false)
    return 0
}
```

```output
1
5
null
null
1
42
1
```

HTTP + JSON + routing (pure parts run; serve/request check only):

```klang run
fn main() -> i32 {
    print(http_json(200, {"a": 1})["status"])
    print(http_text(200, "hi")["status"])
    print(http_redirect("http://x/")["status"])
    print(route_match("/notes/:id", "/notes/7")["id"])
    print(is_null(route_match("/a", "/b")))
    print(json_decode("{\"a\": null}"))
    print(json_stringify({"a": json_decode("null")}, 0))
    return 0
}
```

```output
200
200
302
7
1
{"a": null}
{"a":null}
```

```klang check
fn h(req: map) -> map {
    return http_json(200, {"ok": 1})
}

fn main() -> i32 {
    let s = http_serve("127.0.0.1", 0, "h", {"workers": 1})
    print(server_port(s))
    print(http_request("GET", "http://127.0.0.1/", {}, "", 1000)["status"])
    http_stop(s)
    print(json_decode("null"))
    return 0
}
```

Datetime (runs, UTC):

```klang run
fn main() -> i32 {
    print(dt_now() > 0)
    let ms = dt_from_parts(2024, 2, 29, 12, 34, 56)
    print(dt_format(ms, "%Y-%m-%d"))
    print(dt_format(ms, "%H:%M:%S"))
    print(dt_diff_ms(dt_add(ms, {"days": 1}), ms))
    print(dt_parts(ms)["day"])
    print(dt_parts_offset(ms, 60)["hour"])
    print(dt_local_offset_minutes() <= 1080)
    print(dt_is_leap(2024))
    print(dt_days_in_month(2024, 2))
    print(dt_parse_iso("2024-02-29T12:34:56Z") == ms)
    return 0
}
```

```output
1
2024-02-29
12:34:56
86400000
29
13
1
1
29
1
```

Compression strings (runs, pure):

```klang run
fn main() -> i32 {
    let g = gzip_compress("hello")
    print(gzip_decompress(g))
    print(zlib_decompress(zlib_compress("hello")))
    return 0
}
```

```output
hello
hello
```

Compression files + tar (check only — needs temp files at run time;
gates cover live round trips):

```klang check
fn main() -> i32 {
    print(gzip_file("a.txt", "a.gz"))
    print(gunzip_file("a.gz", "b.txt"))
    print(zlib_file("a.txt", "a.z"))
    print(unzlib_file("a.z", "c.txt"))
    print(tar_create("mydir", "out.tar"))
    print(tar_extract("out.tar", "dest"))
    print(tar_extract_limit("out.tar", "dest", 1000))
    return 0
}
```

Postgres (check only — needs DATABASE_URL at run time):

```klang check
fn main() -> i32 {
    let db = pg_connect(get_env("DATABASE_URL"))
    print(pg_exec(db, "SELECT 1", [])["changes"])
    print(len(pg_query(db, "SELECT 1 as v", [])))
    pg_begin(db)
    pg_commit(db)
    pg_begin(db)
    pg_rollback(db)
    pg_close(db)
    return 0
}
```

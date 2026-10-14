# A3 notes — proposed text for shared docs (do NOT apply until L1-docs)

## reference.md — HTTP server + client (new section)

- `http_serve(host: str, port: int, handler: str, options: map) -> Server`:
  `handler(req) -> {status, headers, body}` with
  `req = {method, path, query, headers (lower-case), body, remote}`.
  Binds `host` (`"0.0.0.0"` on Render, `port` from `get_env("PORT")`, `0` =
  any); `options`: `workers` (default 4), `max_body_bytes` (default 1MB),
  `read_timeout_ms` (default 10000), `max_header_bytes` (default 64KB).
  Worker pool on A1 sockets + A2 threads/channels; handler errors are `500`
  `{"error":"internal server error"}` (logged to stderr, server keeps
  running); `400` malformed/chunked, `413` oversize, `408` slow;
  HTTP/1.1 keep-alive + `Connection: close`, always `Content-Length`.
- `server_port(s) -> Int` (`E-HTTP-CLOSED` when stopped),
  `http_stop(s) -> 1` (graceful, idempotent). Poll
  `signal_pending("TERM")`/`"INT"` then `http_stop` for Render shutdown.
  Forgotten servers stop at exit.
- `http_json(status, v)`, `http_text(status, s)`, `http_redirect(url)`
  (pure, sandbox-allowed). `Null` is JSON `null` everywhere.
- `http_request(method, url, headers_map, body, timeout_ms[, options]) ->
  {status, headers, body}`: `GET POST PUT DELETE PATCH HEAD OPTIONS`,
  HTTPS via bundled WebPki (no second stack), redirects up to 5
  (`{"follow_redirects": 0}` disables), `E-HTTP-*`. Keeps
  `http_get`/`http_post` unchanged.
- `route_match(pattern, path)` (`"/notes/:id"` -> `{"id": ...}`, `Null`
  on no-match), `json_decode(s)` (`null`->`Null`, throws `E-JSON-PARSE`),
  `json_stringify(v, pretty)` (`Null`->`null`).

## limitations.md — A3 limits

- Bodies are UTF-8 `Str`; `Content-Length` required (`""` when missing);
  `chunked` is `400`; `DELETE/OPTIONS/GET/HEAD` bodies ignored.
- JSON `true`/`false` decode to `1`/`0` (no `Bool` value); every `Int`
  encodes as a number (bools do not round-trip; `Null` does).
- Fixed worker pool (`1..32`): 200 concurrent queue but all succeed.
- `PORT_FILE` is test-only; Render uses `PORT` + `0.0.0.0` + `SIGTERM`.

## errors.md — new codes

`E-HTTP-URL`, `E-HTTP-METHOD`, `E-HTTP-HEADER`, `E-HTTP-TIMEOUT`,
`E-HTTP-CONNECT`, `E-HTTP-REDIRECT`, `E-HTTP-OPTIONS`, `E-HTTP-HANDLER`,
`E-HTTP-CLOSED`, `E-HTTP-STATUS`, `E-HTTP-ROUTE`, `E-JSON-PARSE`,
`E-JSON-STRINGIFY` (all `runtime`, `0,0`, never panic).
Statuses `400/408/413/500` are HTTP responses, not diagnostics.

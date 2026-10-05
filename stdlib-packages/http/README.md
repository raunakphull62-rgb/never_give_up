# http

HTTP request/response builders plus live transfer wrappers over the
`http_get` / `http_post` builtins (real client; see
`../BUILTINS.md`). Depends on `io` + `string` (resolved transitively).

## API

Builders (pure, offline):

- `HttpReq { method, url, body }` — `http_get_req`, `http_post_req`, `http_req_to_string`
- `HttpResp { code, body }` — `http_resp`, `http_status_ok`
- `http_save_resp(r, path) -> i32`

Live transfers (need network; transport failures raise `E-NET-*`, HTTP
error statuses are normal results — use `http_ok` to check):

- `http_fetch(url) -> map`, `http_fetch_body(url) -> str`
- `http_fetch_async(url) -> map async`
- `http_post_text(url, body, headers) -> map`,
  `http_post_async_text(url, body, headers) -> map async`
- `http_post_form(url, fields) -> map` (form-encoded),
  `http_post_json(url, body_string) -> map` (`Content-Type: application/json`)
- `http_ok(resp) -> bool`, `http_status(resp) -> i32`, `http_body(resp) -> str`
- `http_save_body(resp, path) -> i32`

Pure helpers (offline):

- `http_header(name, value) -> str` (`"Name: Value"`),
  `http_headers(map) -> array`
- `http_query_string(map) -> str` (`k=v&...`, insertion order; values
  render via `str()`; no percent-encoding — that is Phase 2 `url`)
- `http_form_body(map) -> str` (same encoding as query strings)

## Test

```sh
klang run http_test.klang            # offline only, deterministic
cargo test http_pkg_                 # wrappers vs loopback 127.0.0.1 server
KLANG_LIVE_TESTS=1 sh scripts/live-tests.sh   # real internet (optional)
```

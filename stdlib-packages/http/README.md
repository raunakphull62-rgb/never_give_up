# http

HTTP request/response builders. Pure value construction; live client
exchange is a documented Phase 2 addition. Depends on `io` + `string`
(resolved transitively).

## API

- `HttpReq { method, url, body }` — `http_get_req`, `http_post_req`, `http_req_to_string`
- `HttpResp { code, body }` — `http_resp`, `http_status_ok`
- `http_save_resp(r, path) -> i32`

## Test

```sh
klang run http_test.klang
```

# url

URL parsing, building, and percent-encoding. Pure, no network.
Absolute URLs with a scheme only — relative refs report an error.

## API

- `url_parse(s) -> map` — `{ok, scheme, user, host, port, path,
  query, fragment, error}`. Scheme/host lowercased; port as string
  (`""` when absent); missing path becomes `"/"`. Bracketed IPv6
  (`http://[::1]:9000/x`) supported; non-digit ports and empty hosts
  are errors.
- `url_build(map) -> str` — inverse (emits stored parts verbatim).
- `url_encode(s) -> str` — percent-encoding (`%20` for space;
  unreserved `[A-Za-z0-9-_.~]` pass; UTF-8 bytes for the rest).
- `url_decode(s) -> map` — `{ok, value, error}` (`+` decodes to space,
  form-compatible; bad escapes and bad UTF-8 incl. surrogates are
  errors, never aborts).
- `url_query_parse(q) -> array` — `[{key, value}]` (decoded; valueless
  keys get `""`); `url_query_build(pairs) -> str` (encoded).

## Test

```sh
klang run url_test.klang
```

# base64

Standard base64 (RFC 4648 alphabet with `=` padding) over UTF-8
bytes. Pure and self-contained (the UTF-8 helpers twin
`hex_utf8_encode/decode` under `base64_` prefixes — duplicated, not
imported, so no registry dependency and no `E-DUPLICATE` when both
packages load). Decoding never aborts.

## API

- `base64_encode(s) -> str` — padded (`"f"` → `"Zg=="`).
- `base64_decode(s) -> map` — `{ok, value, error}` (`"length not a
  multiple of 4"`, `"bad base64 character"`, `"bad padding"`,
  `"padding must end input"`, or a UTF-8 error).

## Test

```sh
klang run base64_test.klang
```

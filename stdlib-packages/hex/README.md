# hex

Lowercase hex encoding over UTF-8 bytes. Pure; decoding never aborts
(bad input comes back in the result map).

## API

- `hex_encode(s) -> str` — lowercase hex of the UTF-8 bytes.
- `hex_decode(s) -> map` — `{ok, value, error}` (`"odd hex length"`,
  `"bad hex digit"`, or a UTF-8 error).
- `hex_utf8_encode(s) -> array<i32>` / `hex_utf8_decode(bytes) -> map`
  — also twinned as `base64_utf8_*` in the `base64` package
  (duplicated, not imported, so both stay self-contained).
  The decoder is lenient about overlong forms but
  rejects surrogates, truncations, and bad continuation bytes.

## Test

```sh
klang run hex_test.klang
```

# json

JSON parser, accessors, pretty-printer, plus the original encoders
(`encode.klang`: `json_quote`, `json_int/str/bool`, `json_obj1/2/3`,
`json_arr2`, `json_is_obj` — all kept).

## API

Parsing (never aborts on untrusted input):

- `json_parse(text) -> map` — `{"ok": bool, "value": v, "error": e}`
  with `e` like `"line 1, col 6: expected ':'"` (`""` on success).
  Handles objects, arrays, strings (all escapes incl. ASCII `\uXXXX`),
  numbers (out-of-range ints fall back to float), `true/false/null`,
  whitespace, nesting. Duplicate object keys: last wins.
- `json_parse_or_assert(text) -> value` — prints the same `line/col`
  message then asserts, for scripts.

Values: objects → map, arrays → array, strings → str, numbers →
i32/f64, `null` → `{"$json": "null"}`, booleans →
`{"$json": "true"/"false"}` (the runtime has no bool distinct from
int, so markers keep `json_type` honest).

`"$json"` collision (documented): a user object containing exactly the
key `"$json"` with value `"null"`/`"true"`/`"false"` is
indistinguishable from a marker — `json_type` reports
`null`/`bool` and `json_is_null/true/false` match. Any other value
under `"$json"` (numbers, objects, other strings) still reports
`"object"` and round-trips normally. Avoid `"$json"` as a data key
when the distinction matters.

- `json_get(value, key)`, `json_at(value, index)` — raw values; a
  missing key or bad index is `E-RUNTIME` (programmer error, like
  normal indexing).
- `json_type(value) -> str` — `null/bool/int/float/str/array/object`.
- `json_is_null(v)`, `json_is_true(v)`, `json_is_false(v) -> bool`.
- `json_pretty(value, indent) -> str` — indented rendering (`indent`
  spaces per level; `{}`/`[]` stay compact).

Limits: `\u` escapes decode via `chr()` (surrogate halves report an
error); nesting depth is bounded by the interpreter call-depth
cap — keep it under ~100.

## Test

```sh
klang run json_test.klang
```

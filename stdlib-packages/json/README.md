# json

JSON encoders (no decoder in Phase 1 — decode via `regex_find` maps
or manual `split`).

## API

- `json_quote(s)`, `json_int/str/bool(key, v)`
- `json_obj1/2/3(...)`, `json_arr2(a, b)`, `json_is_obj(s)`

## Test

```sh
klang run json_test.klang
```

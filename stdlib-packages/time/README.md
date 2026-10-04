# time

Duration arithmetic and formatting. Pure (no syscalls); wall-clock
helpers are a documented Phase 2 addition.

## API

- `Duration { secs: i32 }` — `dur_new`, `dur_from_mins`, `dur_from_hours`
- `dur_add(a, b)`, `dur_secs(d)`, `dur_mins(d)`, `dur_to_string(d)`, `dur_is_zero(d)`

## Test

```sh
klang run time_test.klang
```

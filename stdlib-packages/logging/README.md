# logging

Leveled logger. Levels `0 DEBUG / 1 INFO / 2 WARN / 3 ERROR`; records
below the logger level are dropped. Depends on `io` + `time`.

## API

- `Logger { name, level }` — `log_new`
- `log_level_name`, `log_should(l, level)`, `log_format(l, level, msg)`
- `log_record(l, level, msg) -> i32` (prints when enabled)
- `log_timed(l, level, msg, d: Duration) -> i32`

## Test

```sh
klang run logging_test.klang
```

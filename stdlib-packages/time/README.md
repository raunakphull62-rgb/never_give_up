# time

Duration arithmetic plus UTC civil datetime math on top of `time_now`
(see `../BUILTINS.md`). Pure integer/float code; the only syscalls are
the `time_now` reads inside user code.

## API

Duration (unchanged):

- `Duration { secs: i32 }` — `dur_new`, `dur_from_mins`, `dur_from_hours`
- `dur_add(a, b)`, `dur_secs(d)`, `dur_mins(d)`, `dur_to_string(d)`, `dur_is_zero(d)`

Civil datetime (UTC, `ts >= 0` i.e. 1970-01-01 onward; negative inputs
assert — civil-from-days algorithm):

- `time_to_civil(ts) -> map` — `{year, month, day, hour, minute, second}`
- `time_from_civil(y, mo, d, h, mi, s) -> f64` — asserts valid ranges
- `time_iso8601(ts) -> str` (`"2024-01-01T00:00:00Z"`),
  `time_parse_iso8601(s) -> map` (`{ok, year..second, error}` with
  `"line 1, col N: ..."` errors; never aborts on untrusted input)
- `time_weekday(ts) -> i32` (0=Sunday), `time_weekday_name(ts) -> str`
- `time_is_leap_year(y) -> bool`, `time_days_in_month(y, m) -> i32`
- `time_add_days(ts, n)`, `time_add_hours(ts, n) -> f64`
- `time_format_duration(secs) -> str` (`"1d 1h 1m 1s"`, `"0s"`; skips
  zero units)

## Test

```sh
klang run time_test.klang
```

# testing

Assert helpers and tiny test-framework hooks. A failing `test_check` /
`test_eq_*` prints `FAIL <name>` and aborts loudly via `assert(false)`;
passing checks print `PASS <name>` and contribute 0 to the failure
count. `test_run` never aborts — it returns 0/1 for counting.

## API

- `test_check(cond, name) -> i32`
- `test_eq_int(a, b, name) -> i32`
- `test_eq_str(a, b, name) -> i32`
- `test_eq_float(a, b, eps, name) -> i32` — passes when `|a-b| <= eps`.
- `test_summary(failures) -> i32` — 0 when clean (kept for compatibility).
- `test_suite(name) -> i32` — prints `SUITE <name>`, returns a fresh
  failure count (0).
- `test_run(name, passed) -> i32` — prints `PASS`/`FAIL`, returns 0/1.
- `test_report(passed, failed) -> i32` — prints
  `TESTS passed=X failed=Y` (plus `ALL TESTS PASSED` when clean) and
  returns `failed` (nonzero on failure).

## Test

```sh
klang run testing_test.klang
```

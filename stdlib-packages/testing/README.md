# testing

Assert helpers and tiny test-framework hooks. A failing check prints
`FAIL <name>` and aborts loudly via `assert(false)`; passing checks
print `PASS <name>` and contribute 0 to the failure count.

## API

- `test_check(cond, name) -> i32`
- `test_eq_int(a, b, name) -> i32`
- `test_eq_str(a, b, name) -> i32`
- `test_summary(failures) -> i32` — 0 when clean

## Test

```sh
klang run testing_test.klang
```

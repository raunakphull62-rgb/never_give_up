# math

Math constants and integer/float helpers (1.1.0 adds the float
wrappers over the compiler builtins; `math_pi`/`math_e` are now full
`f64` precision).

## API

- `math_pi() -> f64`, `math_e() -> f64`
- `math_abs(n)`, `math_max(a, b)`, `math_min(a, b)`, `math_clamp(v, lo, hi)`
- `math_pow(base, exp) -> i32`, `math_fact(n) -> i32`
- `math_circle_area(r: f64) -> f64`
- Float wrappers (delegate to the builtins; wrong types are `E-TYPE`,
  `sqrt` of a negative / `log` of `<= 0` are loud domain errors):
  `math_sqrt`, `math_powf`, `math_absf`, `math_minf`, `math_maxf`,
  `math_floor`, `math_ceil`, `math_sin`, `math_cos`, `math_tan`,
  `math_atan2`, `math_log`, `math_exp`

## Test

```sh
klang run math_test.klang
```

# string

StringBuilder and small string utilities.

## API

- `StringBuilder { parts: array }` — `str_builder_new()`, `str_builder_push(b, s)`, `str_builder_build(b)`
- `str_is_empty(s) -> bool`
- `str_repeat(s, n) -> str`
- `str_lines(s) -> array` / `str_line_count(s) -> i32`
- `str_contains_all(s, a, b) -> bool`
- `str_shout(s) -> str` — uppercased plus `!`

## Example

```klang
import "string/lib.klang"

fn main() -> i32 {
    let b = str_builder_new()
    b = str_builder_push(b, "klang")
    print(str_builder_build(b))
    return 0
}
```

## Test

```sh
klang run string_test.klang
```

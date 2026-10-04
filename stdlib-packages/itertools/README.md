# itertools

Iterator utilities over plain arrays. All functions are pure: inputs are
pass-by-value, results are new values.

## API

- `iter_sum(nums: array) -> i32` — sum of elements (0 for empty)
- `iter_count(nums: array) -> i32` — element count
- `iter_max(nums: array) -> i32` — maximum (0 for empty)
- `iter_min(nums: array) -> i32` — minimum (0 for empty)
- `iter_filter_pos(nums: array) -> array` — elements `> 0`
- `iter_add_all(nums: array, k: i32) -> array` — each element plus `k`

## Example

```klang
import "itertools/lib.klang"

fn main() -> i32 {
    let total = iter_sum([20, 22])
    print(total)
    return 0
}
```

## Test

```sh
klang run itertools_test.klang
```

# collections

List, Stack, Queue, and map helpers. Value semantics: mutating
operations return the updated collection. Depends on `itertools`
(`list_sum` / `list_max`).

Popping is two-step (pass-by-value): `stack_pop` returns
`{"value", "items"}` — rebuild with the items via a fresh struct or
keep the value.

## API

- `List { items }` — `list_new/empty/len/push/get/sum/max`
- `Stack { items }` — `stack_new/push/pop/len`
- `Queue { items, head }` — `queue_new/push/peek/pop/len`
- `map_put(m, k, v)`, `map_get_or(m, k, dflt)`

## Test

```sh
klang run collections_test.klang
```

# collections

List, Stack, Queue, Heap, Deque, OrderedMap, set and map helpers. Value
semantics: mutating operations return the updated collection. Depends
on `itertools` (`list_sum` / `list_max`).

Popping is two-step (pass-by-value): `stack_pop` returns
`{"value", "items"}` — rebuild with the items via a fresh struct or
keep the value. Same shape for `heap_pop` (`{"value", "heap"}`) and
`deque_pop_front/back` (`{"value", "deque"}`).

## API

- `List { items }` — `list_new/empty/len/push/get/sum/max`
- `Stack { items }` — `stack_new/push/pop/len`
- `Queue { items, head }` — `queue_new/push/peek/pop/len`
- `map_put(m, k, v)`, `map_get_or(m, k, dflt)`
- `Heap { items, is_max }` — `heap_new_min/heap_new_max`, `heap_len`,
  `heap_peek`, `heap_push`, `heap_pop`, `heap_sorted` (drains ascending
  for min, descending for max). Empty peek/pop assert.
- `Deque { front, back }` — `deque_new`, `deque_push_back/front`,
  `deque_peek_front/back`, `deque_pop_front/back`, `deque_len`.
  Empty peek/pop assert.
- `OrderedMap { keys, vals }` — `omap_new/put/get/get_or/contains/
  len/keys/remove` (insertion order; re-put keeps the original
  position; `omap_get` on a missing key asserts).
- `set_union(a, b)`, `set_intersection(a, b)`, `set_difference(a, b)` —
  order-preserving, deduplicated, i32 elements.

## Test

```sh
klang run collections_test.klang
```

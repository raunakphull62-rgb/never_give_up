# sync

Mutex and channel types as single-threaded value logic. `Mutex`
flips a flag; `Channel` wraps an array queue (LIFO `chan_recv`
returns `{"value", "items"}` — rebuild with `chan_new`).

## API

- `Mutex { locked, value }` — `mutex_new/lock/unlock/is_locked/get/set`
- `Channel { items }` — `chan_new(seed)`, `chan_send`, `chan_len`, `chan_recv`

## Test

```sh
klang run sync_test.klang
```

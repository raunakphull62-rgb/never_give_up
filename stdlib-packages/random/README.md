# random

Deterministic PRNG with explicit state. Park-Miller minimal standard
(a=16807, m=2^31-1) via Schrage's method, so every intermediate fits
Klang's checked i32 arithmetic. Klang has no bitwise operators, so this
arithmetic formulation replaces the usual xorshift64. Deterministic for
a given seed. NOT cryptographic — for real key material wait for the
Phase 2 OS-seeded builtins.

## API

State is `{"state": s}`; every draw returns `{"value": v, "state": s2}`
(or `{"list", "state"}`) — thread the state through:

- `rand_new(seed) -> map` — any i32 seed normalizes to `1..m-1`.
- `rand_next(state) -> map` — raw draw in `1..2147483646`.
- `rand_int(state, lo, hi) -> map` — asserts `hi >= lo`; `hi - lo`
  must fit i32.
- `rand_float(state) -> map` — in `[0.0, 1.0)`.
- `rand_choice(list, state) -> map` — asserts non-empty.
- `rand_shuffle(list, state) -> map` — Fisher-Yates, returns a new list.
- `rand_state(state) -> i32`, `rand_seed_from_time() -> map`
  (from `time_now`; distinct runs differ, same millisecond may repeat).

## Test

```sh
klang run random_test.klang
```

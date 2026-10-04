# crypto

Demo checksum and comparison helpers. **Not cryptographic**: real
archive integrity is enforced by the registry client (SHA-256).

## API

- `crypto_hash(s) -> i32` — deterministic checksum (`""` → 0)
- `crypto_eq_const(a, b) -> bool` — full-walk string equality
- `crypto_pin4(n) -> str` — zero-padded 4-digit string

## Test

```sh
klang run crypto_test.klang
```

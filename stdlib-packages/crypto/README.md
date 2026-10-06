# crypto

Checksum, hashing, and comparison helpers (1.1.0 adds real SHA-256 /
HMAC-SHA256 over the compiler builtins).

## API

- `crypto_hash(s) -> i32` — deterministic checksum (`""` → 0).
  **NON-CRYPTOGRAPHIC**: pedagogical FNV-style checksum with trivial
  collisions — teaching and non-adversarial bucketing only. Never use
  it for integrity, passwords, or tokens; use `crypto_sha256` instead.
- `crypto_sha256(s) -> str` — real SHA-256, lowercase hex.
- `crypto_hmac_sha256(key, msg) -> str` — real HMAC-SHA256, hex.
- `crypto_eq_const(a, b) -> bool` — full-walk string equality
- `crypto_pin4(n) -> str` — zero-padded 4-digit string

## Test

```sh
klang run crypto_test.klang
```

# Wave1 S4 notes (not a shared doc — staging area)

Text that belongs in shared docs (SPEC.md, docs/reference.md,
docs/limitations.md, docs/LANGUAGE_STATUS.md) goes here until owners approve it.
Do NOT copy into those files in this wave.

## D3 migration note (what used to happen vs now)

Integers (decision D3, implemented Wave1 S4):

- Used to be: `Int` was a 32-bit-checked type. Bare literals past
  2147483647 were `E-TYPE`; arithmetic past ±2^31 was loud `E-OVERFLOW`;
  `i64`/`u32`/`u64` annotations were accepted but enforced the `i32`
  range (each use warned `W-TYPE-NARROW`); `u8` (and any other unknown
  spelling) was unchecked dynamic (`Unknown`); returning a large value
  from any function failed the blanket `i32` return check.
- Now: `Int` is 64-bit with checked arithmetic — the same errors fire
  at ±2^63 instead (`E-TYPE` for bare `9223372036854775808`,
  `E-OVERFLOW` for arithmetic past the range;
  `-9223372036854775808` is valid `i64::MIN`). All five annotations
  `i32`, `u32`, `i64`, `u64`, `u8` are honest checked types: values
  outside the annotation's range fail at the boundary — argument
  passing, `return`, rebinding an annotated parameter, struct
  construction, struct field assignment — with `E-RUNTIME` naming the
  type and the value (e.g. `` `id` arg 0: value 256 out of `u8` range
  0..255 ``). `W-TYPE-NARROW` is retired (emits nothing).
- Who must act: programs that depended on `E-OVERFLOW`/`E-TYPE` at 2^31
  as control flow (now run past it); programs returning
  `now_ms()`/`file_size()`-scale values from `-> i32` (now loud
  `E-RUNTIME` — declare `-> i64`); the two moved stdlib-era test
  programs in `tests/time_builtin_gates.rs` were migrated `-> i32` →
  `-> i64` for exactly this reason. Everything else is unaffected:
  every value representable before behaves identically, and the 26
  published packages print byte-identical self-test output.

Truthiness (decision D2, implemented Wave1 S4):

- Used to be / still is by default: `if`/`while` accept `Bool` and
  `Int`; `&&`/`||`/`!` accept `Bool`/`Int` operands. Nothing changed.
- New opt-in: `--strict` (`klang run`/`klang check`) or `strict = true`
  under `[project]` in `klang.toml` rejects statically-known non-`Bool`
  conditions in `if`/`while`/`&&`/`||`/`!` with `E-TYPE` naming the type
  (`` if condition: want bool, got i32 ``). Dynamic (`Unknown`)
  conditions stay allowed in both modes. `assert()`, match guards, and
  `for` bounds are unchanged in both modes.

## Shared-doc text staged for later (verbatim proposals)

SPEC.md §2 table row + paragraph (replace the `i64`/`u32`/`u64` alias
+ `W-TYPE-NARROW` + `u8`-unchecked paragraph):

```text
| `i32`, `int`, `i64`, `u32`, `u64`, `u8` | Int (64-bit, checked) |
```

```text
`int` coerces to `float`. `Int` holds the full `i64` range and
arithmetic past it is loud `E-OVERFLOW` (never wraps). The narrower
spellings enforce their range at value boundaries — argument passing,
`return`, rebinding an annotated parameter, struct construction, and
struct field assignment: `i32` (−2147483648..2147483647), `u32`
(0..4294967295), `u64` (0..2^64−1, non-negative `i64` values pass),
`u8` (0..255). A value outside the range is `E-RUNTIME` naming the
type and the value. `i64` and `int` accept the full range (no
narrowing). Everything else (map lookups, dynamic index, unknown
names) is `unknown`, a wildcard that never emits `E-TYPE` — this keeps
dynamic code working while still catching `"hi" - 1`, `1 + true`,
`add("hi", 1)`, `if "hi"`, `while "x"`, `for i in 0.."x"`, `for x in
42`. (`return "hi"` in `-> i32` is still `E-TYPE` at check time.)
```

docs/reference.md §1 (integer literals) + §2 (types table):

```text
- Integer literals are decimal (`42`), floats need digits on both
  sides of the dot (`4.0`; `0..10` still lexes as a range because the
  dot must be followed by a digit to count as a float). Integer
  literals must fit `i64` (`-9223372036854775808` is allowed via unary
  minus); out-of-range literals are `E-TYPE`, and literals beyond
  `u64` magnitude are `E-PARSE`.
```

```text
| `i32`, `int`, `i64`, `u32`, `u64`, `u8` | 64-bit integer (`Int`, checked) |
```

```text
`int` coerces to `float` wherever a float is wanted. Narrower integer
spellings enforce their range at value boundaries with `E-RUNTIME`
(`i32`: −2147483648..2147483647; `u32`: 0..4294967295; `u64`:
non-negative; `u8`: 0..255); `i64`/`int` accept the full range.
`unknown` is the documented escape hatch: map lookups, dynamic
indexes, unknown names, and generic-variable positions never emit
`E-TYPE` — dynamic code keeps working while concrete mismatches
(`"hi" - 1`, `1 + true`, `return "hi"` from `-> i32`) are rejected.
Generic struct fields and generic match bindings are also `unknown` at
the use site.
```

docs/reference.md §3 (`if`/`while` conditions) — append:

```text
- Strict mode (`klang run/check --strict`, or `strict = true` under
  `[project]` in `klang.toml`) narrows conditions to `Bool` (dynamic
  `unknown` still allowed): a statically-known non-`Bool` condition in
  `if`/`while` or with `&&`/`||`/`!` is `E-TYPE` naming the type.
  Default mode is unchanged (`Bool`/`Int`).
```

docs/reference.md §5 (`parse_int` row) — no change needed (already says
strict decimal `i32`).

docs/limitations.md — append a short item:

```text
## Integer widths (D3, Wave1 S4)

`Int` is 64-bit checked (`E-OVERFLOW` past ±2^63, never wraps);
`i32`/`u32`/`u64`/`u8` enforce their range at argument/return/assign
boundaries (`E-RUNTIME` naming type and value); `W-TYPE-NARROW` is
retired. Still unrepresentable: `u64` magnitudes past `i64::MAX`
(cannot be spelled or produced); enum payloads are statically checked
only (no runtime boundary there); the int-only JIT still wraps (pinned
in `tests/parity/allowlist.toml` at the 64-bit boundary); `parse_int`
and `random_int` bounds stay 32-bit.
```

docs/LANGUAGE_STATUS.md Phase-1-update style addendum (for the next
auditor, not this wave's call):

```text
- Items 3/2 (Wave1 S4, D3): `Int` is now 64-bit checked
  (`tests/int_boundary_gates.rs`); `W-TYPE-NARROW` retired.
- Item 2 (Wave1 S4, D2): default truthiness unchanged; opt-in
  `--strict` / `[project] strict = true` rejects statically-known
  non-`Bool` conditions (`tests/strict_mode_gates.rs`).
```

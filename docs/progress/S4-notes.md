# Wave1 S4 notes (not a shared doc — staging area)

Text that belongs in shared docs (SPEC.md, docs/reference.md,
docs/limitations.md, docs/LANGUAGE_STATUS.md) goes here until owners approve it.
Do NOT copy into those files in this wave.

## D3 migration note (what used to happen vs now)

TBD — will state: Int used to be i32-checked (literals past 2^31 were
E-TYPE, arithmetic past 2^31 was E-OVERFLOW, `i64`/`u32`/`u64` warned
W-TYPE-NARROW and `u8` was unchecked dynamic). Now: Int is 64-bit checked
(same errors at 2^63), the five annotations enforce their range at
assignment/parameter/return boundaries with E-RUNTIME naming type+value,
and W-TYPE-NARROW is retired.

## Shared-doc text staged for later

TBD.

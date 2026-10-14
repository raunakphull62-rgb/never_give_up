# L1 notes — proposed text for shared docs (for the later L1-docs stage; NOT applied now)

The queue forbids editing shared docs in feature stages (rule 9); the only
`docs/ai/` change in L1 is the minimal `stdlib.md` table-row gate fix. The
full verified guide section belongs to the later L1-docs stage. Draft:

## stdlib.md — datetime rows (verified examples to add in L1-docs)

```klang run
fn main() -> i32 {
    let ms = dt_from_parts(2024, 2, 29, 12, 34, 56)
    print(dt_format(ms, "%Y-%m-%d"))
    print(dt_format(ms, "%H:%M:%S"))
    print(dt_diff_ms(dt_add(ms, {"days": 1}), ms))
    return 0
}
```

```output
2024-02-29
12:34:56
86400000
```

## stdlib.md — compression rows (verified examples to add in L1-docs)

```klang run
fn main() -> i32 {
    let g = gzip_compress("hello")
    print(gzip_decompress(g))
    print(zlib_decompress(zlib_compress("hello")))
    return 0
}
```

```output
hello
hello
```

## reference.md / limitations.md — points to cover in L1-docs

- Canonical instant = `Int` epoch milliseconds (UTC); named time zones
  unsupported; `%z` always `+0000`; `%f` = milliseconds 3-digit.
- No bytes type: compressed bytes are lowercase hex (strings) or files;
  use the `hex` package for general hex helpers.
- `tar_extract` caps at 256 MB by default (`tar_extract_limit` option);
  traversal entries (`..`, absolute, links) are `E-COMPRESS-UNSAFE-PATH`.
- Sandbox: datetime + string compress allowed; file forms need temp-dir
  paths.
- `repeat` caps at 1M chars, so huge test strings come from files.

## errors.md — codes to document in L1-docs

`E-TIME-PARSE`, `E-TIME-RANGE`, `E-TIME-PATTERN`, `E-COMPRESS-CORRUPT`,
`E-COMPRESS-TOO-LARGE`, `E-COMPRESS-UNSAFE-PATH` (all with trigger +
minimal example).

# compress

Run-length codec (runs capped at 9) plus file round-trip helpers over
`io`. Real gzip/zip is a documented Phase 2 addition.

## API

- `cmp_rle_encode(s)`, `cmp_rle_decode(s)` — inverse pair
- `cmp_ratio(orig, coded) -> i32` — coded size as % of original
- `cmp_roundtrip_file(path, content) -> bool`

## Test

```sh
klang run compress_test.klang
```

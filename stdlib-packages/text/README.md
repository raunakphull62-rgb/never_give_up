# text

Pure text utilities. No builtins besides string helpers; nothing here
can fail except `text_wrap` (asserts `width > 0`).

## API

- `text_pad_left(s, width, pad)`, `text_pad_right`, `text_center`
  (widths in characters; short widths return the input; `text_center`
  puts the extra pad on the right).
- `text_repeat(s, n)`, `text_reverse(s)`, `text_count(s, sub)`
  (non-overlapping; `""` needle → 0), `text_index_of(s, sub) -> i32`
  (`-1` when absent), `text_skip_chars(s, n)`.
- `text_strip_prefix/suffix` (no-op when absent; empty affix → input).
- Case: `text_snake_case`, `text_kebab_case`, `text_camel_case`,
  `text_title_case` — split on `_ - . space` plus camelCase boundaries
  (`"fooBar"` → `foo/bar/baz`, `fooBar`). All-caps acronyms
  (`"HTMLParser"`) are not split.
- `text_wrap(s, width) -> array<str>` — greedy word wrap.

## Test

```sh
klang run text_test.klang
```

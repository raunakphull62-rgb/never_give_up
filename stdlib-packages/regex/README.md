# regex

Pattern matching over the `regex_is_match` / `regex_find` builtins.
Depends on `string` (resolved transitively — no explicit declaration
needed by consumers).

## API

- `regex_matches(pat, text) -> bool` — true when the pattern matches.
- `regex_first(pat, text) -> str` — first match, or `""`.
- `regex_find_or(pat, text, default) -> str` — first match, or `default`.
- `regex_groups(pat, text) -> array<str>` — capture groups 1..n (`[]` on
  no match).
- `regex_named_map(pat, text) -> map` — named groups (`{}` on no match).
- `rx_match`, `rx_find_or_empty` — legacy aliases, kept for old tests.
- `rx_matches_word(pat, text, word) -> bool`, `rx_count_words(text) -> i32`

## Failure convention

A bad pattern raises `E-REGEX-INVALID-PATTERN` (real parse error as the
cause). No match is a normal empty result, never an error.

## Test

```sh
klang run regex_test.klang
```

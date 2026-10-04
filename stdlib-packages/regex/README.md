# regex

Pattern matching over the `regex_is_match` / `regex_find` builtins.
Depends on `string` (resolved transitively — no explicit declaration
needed by consumers).

## API

- `rx_match(pat, text) -> bool`
- `rx_find_or_empty(pat, text) -> str`
- `rx_matches_word(pat, text, word) -> bool`
- `rx_count_words(text) -> i32`

## Test

```sh
klang run regex_test.klang
```

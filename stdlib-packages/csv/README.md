# csv

Pure CSV parsing and writing. No builtins besides string helpers; no
IO, no failure except programmer-error asserts.

## API

- `csv_parse(text, delim) -> array<array<str>>` — handles quotes,
  escaped quotes (`""`), newlines inside quotes, CRLF. A single
  trailing newline produces no extra row; `""` yields `[]`.
  Unterminated quotes consume to end of input (documented, no abort).
- `csv_parse_comma(text)` — `csv_parse(text, ",")`.
- `csv_write(rows, delim) -> str` — quotes fields containing the
  delimiter, `"`, `\n` or `\r` (doubling inner quotes); rows joined
  with `"\n"`, no trailing newline.
- `csv_write_comma(rows)` — `csv_write(rows, ",")`.
- `csv_header_map(rows) -> array<map>` — first row is headers; short
  rows get `""` for missing cells, extra cells are ignored; `[]` for
  empty input.

The delimiter must be a single-character string (asserts otherwise).

## Test

```sh
klang run csv_test.klang
```

# io

Reader/Writer wrappers over the file builtins (`read_file`,
`write_file`, `append_file`, `exists`, `remove_file`).

## API

- `Reader { path }` / `Writer { path }` — `io_reader`, `io_writer`
- `io_read_all(r) -> str`, `io_write_all(w, content) -> i32` (byte count)
- `io_append_all(w, content) -> i32`, `io_exists_path`, `io_remove_path`
- `io_write_lines(path, lines: array) -> i32` — newline-joined write

## Test

```sh
klang run io_test.klang
```

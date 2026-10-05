# fs

File helpers over the file builtins (`read_file`, `write_file`,
`append_file`, `exists`, `remove_file`).

## API

- `fs_read(path) -> str` — whole file.
- `fs_write(path, content) -> i32` — overwrite, returns byte count.
- `fs_append(path, content) -> i32` — append (creates when absent).
- `fs_exists(path) -> bool` — never fails.
- `fs_remove(path) -> i32` — returns 1.
- `fs_read_lines(path) -> array<str>` — lines without endings; a single
  trailing newline produces no extra element; CRLF is stripped; an empty
  file yields `[]`.
- `fs_write_lines(path, lines) -> i32` — joins with `"\n"` (no trailing
  newline), returns byte count.
- `fs_copy_file(src, dst) -> i32` — read + write, returns byte count.
- `fs_touch(path) -> i32` — 1 when created, 0 when it already existed.
- `fs_read_or(path, default) -> str` — file contents, or `default` on any
  failure (the only function that catches errors).

## Failure convention

All functions except `fs_read_or` let the builtin diagnostic propagate:
missing file is `E-IO-NOT-FOUND`, permission problems are
`E-IO-PERMISSION`. Check `fs_exists` first when a missing file is
expected, or use `fs_read_or`.

## Test

```sh
klang run fs_test.klang
```

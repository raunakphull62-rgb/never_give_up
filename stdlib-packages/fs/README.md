# fs

File and directory helpers over the file builtins (`read_file`,
`write_file`, `append_file`, `exists`, `remove_file`, `list_dir`,
`make_dir`, `make_dirs`, `is_dir`, `is_file`, `rename_file`,
`copy_file`, `file_size`).

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
- `fs_copy_file(src, dst) -> i32` — read + write copy (kept for
  compatibility; `fs_copy` uses the OS copy).
- `fs_list_dir(path) -> array<str>` — sorted entry names.
- `fs_make_dir(path) -> i32` — single level (existing dir is an error);
  `fs_make_dirs(path) -> i32` — idempotent with parents.
- `fs_is_dir(path)`, `fs_is_file(path) -> bool` — never fail.
- `fs_file_size(path) -> i32` — bytes.
- `fs_rename(from, to)`, `fs_copy(from, to) -> i32`.
- `fs_walk(dir) -> array<str>` — depth-first recursive listing
  (`dir/name` paths, dirs included; symlink cycles are cut by
  revisiting check plus a 10000-entry assert).
- `fs_glob(dir, pattern) -> array<str>` — non-recursive; `"*"`,
  `"*.ext"`, `"prefix*"`, exact names. Anything else prints its reason
  then asserts.

## Failure convention

All functions except `fs_read_or` let the builtin diagnostic propagate:
missing file is `E-IO-NOT-FOUND`, permission problems are
`E-IO-PERMISSION`. Check `fs_exists` first when a missing file is
expected, or use `fs_read_or`.

## Test

```sh
klang run fs_test.klang
```

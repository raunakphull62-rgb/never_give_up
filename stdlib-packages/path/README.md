# path

Pure path manipulation with `/` separator. No builtins besides string
helpers, so nothing here touches the filesystem or fails.

## API

- `path_join(a, b) -> str` — absolute `b` wins; else join with one `/`.
- `path_basename(p) -> str` — after the last `/` (`"/"` → `"/"`).
- `path_dirname(p) -> str` — up to the last `/` (`"c.txt"` → `"."`,
  `"/b"` → `"/"`).
- `path_extension(p) -> str` — after the last dot of the basename, no
  dot (`"a.tar.gz"` → `"gz"`; `"Makefile"`, `".gitignore"` → `""`).
- `path_stem(p) -> str` — basename minus extension (`"a.tar.gz"` →
  `"a.tar"`).
- `path_is_absolute(p) -> bool` — starts with `/`.
- `path_normalize(p) -> str` — collapses `.`, `..`, duplicate `/`
  (`"/../c"` → `"/c"`, `"../a"` stays, `""` → `"."`, `"/"` → `"/"`).
- `path_split(p) -> map` — `{"dir": ..., "base": ...}`.

## Test

```sh
klang run path_test.klang
```

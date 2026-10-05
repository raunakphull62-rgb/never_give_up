# Klang Standard Library Packages (Phase 1 + expansion)

22 publishable packages (15 PRD modules + `itertools`, the shared
transitive leaf, + 6 Phase 1 expansion packages). Each package ships
`klang.toml` + root `lib.klang` (import entry) + `src/*.klang` + root
`<name>_test.klang` (runnable via `klang run`) + `README.md`.

Layout note: tests live at the package root (not `tests/`) because
the import resolver rejects `..` paths — a `tests/` file could not
import `../lib.klang`.

Builtins reference: `BUILTINS.md` (exact signatures + tested behavior;
re-probe before wrapping a new builtin).

## Publish order (dependencies first)

1. Leafs (no deps): `itertools`, `string`, `math`, `time`,
   `testing`, `crypto`, `io`, `sql`, `sync`, `json`, `random`, `csv`,
   `text`, `path`, `fs`, `os`
2. Second layer: `regex` → string; `compress` → io; `net` → io
3. Third layer: `http` → io, string; `logging` → io, time;
   `collections` → itertools

```sh
# per package (with KLANG_REGISTRY_TOKEN set):
klang publish --dir stdlib-packages/<name> --registry <url>
# or all in order (skips versions that already exist):
sh scripts/publish-stdlib.sh
# consumer:
klang add collections && klang run app.klang
```

## Conventions

- Every `fn` is prefixed (`list_`, `http_`, …): whole-file imports
  merge, so unprefixed names would collide (`E-DUPLICATE`).
- Value semantics: mutating ops return the updated collection.
- `crypto`/`compress` are pedagogical (RLE, FNV-style checksum);
  archive integrity stays SHA-256 in the registry client.
- `net`/`http` live-exchange notes: `http` wraps the real client;
  offline tests stay in `<pkg>_test.klang`, loopback-server tests in
  `tests/http_pkg_gates.rs`, real-internet checks in
  `scripts/live-tests.sh` (`KLANG_LIVE_TESTS=1`).
- Error handling: wrappers let builtin diagnostics propagate
  (`E-IO-*`, `E-PROCESS-*`, `E-REGEX-*`, `E-NET-*`); pure code returns
  sentinels or result maps (`fs_read_or`, `json_parse`,
  `time_parse_iso8601`); programmer errors assert. Each package README
  documents which convention its functions use.

## Which functions need which builtin

| Package | Builtins used |
|---|---|
| `collections` | none (pure + `itertools`) |
| `compress` | file builtins via `io` |
| `crypto` | `chars`, `len` (pure) |
| `csv` | none (pure) |
| `fs` | `read_file`, `write_file`, `append_file`, `exists`, `remove_file` |
| `http` | `http_get`, `http_post`, `http_get_async`, `http_post_async` (+ `io`, `string` helpers) |
| `io` | `read_file`, `write_file`, `append_file`, `exists`, `remove_file` |
| `itertools` | `len`, `push`, `range` |
| `json` | `parse_int`, `parse_float`, `chars` (+ kept encoders use `str`) |
| `logging` | file builtins via `io`, `time_now` via `time` |
| `math` | none (pure) |
| `net` | file builtins via `io` |
| `os` | `env`, `run_process` |
| `path` | none (pure) |
| `random` | `time_now` (only `rand_seed_from_time`) |
| `regex` | `regex_is_match`, `regex_find` (+ `string` helpers) |
| `sql` | none (pure) |
| `string` | str methods |
| `sync` | none (pure) |
| `text` | none (pure) |
| `testing` | `assert`, `print`, `str` |
| `time` | none (pure; wall-clock reads via `time_now` happen in user code) |

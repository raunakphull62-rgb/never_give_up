# Klang Standard Library Packages (Phase 1)

16 publishable packages (15 PRD modules + `itertools`, the shared
transitive leaf). Each package ships `klang.toml` + root `lib.klang`
(import entry) + `src/*.klang` + root `<name>_test.klang`
(runnable via `klang run`) + `README.md`.

Layout note: tests live at the package root (not `tests/`) because
the import resolver rejects `..` paths — a `tests/` file could not
import `../lib.klang`.

## Publish order (dependencies first)

1. Leafs (no deps): `itertools`, `string`, `math`, `time`,
   `testing`, `crypto`, `io`, `sql`, `sync`, `json`
2. Second layer: `regex` → string; `compress` → io; `net` → io
3. Third layer: `http` → io, string; `logging` → io, time;
   `collections` → itertools

```sh
# per package (with KLANG_REGISTRY_TOKEN set):
klang publish --dir stdlib-packages/<name> --registry <url>
# consumer:
klang add collections && klang run app.klang
```

## Conventions

- Every `fn` is prefixed (`list_`, `http_`, …): whole-file imports
  merge, so unprefixed names would collide (`E-DUPLICATE`).
- Value semantics: mutating ops return the updated collection.
- `crypto`/`compress` are pedagogical (RLE, FNV-style checksum);
  archive integrity stays SHA-256 in the registry client.
- `net`/`http` ship address/request builders only; live sockets are
  Phase 2.

# S1 notes (Wave1 S1) — package imports by name + version string

This file holds S1 design notes. Shared docs (`docs/LANGUAGE_STATUS.md`,
`docs/reference.md`, `docs/limitations.md`, `SPEC.md`, `README`,
`docs/design/README.md`) are untouched per task rules.

## S1a: bare package imports (`import "math"`)

### Entry discovery
- Vendor layout is `.klang_pkgs/<name>/<version>/` (see `registry::pkg_dir`).
- Real stdlib packages ship root `lib.klang` as import entry
  (`stdlib-packages/README.md`: each package ships `klang.toml` + root
  `lib.klang` + `src/`).
- Bare entry candidates in order: `lib.klang`, `<name>.klang`, `main.klang`.
  First existing file wins. No `klang.toml` `entry` lookup (that key names a
  function, not a file).

### Bare detection
- `is_bare_package_name(raw)` is `registry::valid_pkg_name(raw)`:
  `[A-Za-z0-9_-]+`, no `/`, no `.`. Anything with `/` or `.` keeps the
  legacy relative + `pkg/file.klang` vendor-slash path byte-identical.
- Existing tests use `*.klang` (dot) or `pkg/file.klang` (slash), so none
  trigger the bare path; vendored stdlib self-tests are unaffected.

### Local wins
- Local candidates for bare `foo` in importer dir `D`: `D/foo` (file),
  `D/foo.klang`, `D/foo/lib.klang`. First existing wins.
- When a local file wins and a vendor entry also exists, one line goes to
  stderr via `eprintln!`:
  `note: local "<path>" shadows package "<name>@<version>"`.
- Stdout stays the program's own output (notes are stderr-only).

### Version lookup
- Reads `klang.lock` for the exact pinned version (not the manifest).
- When the manifest declares a constraint for the name, picks the max locked
  version satisfying it (`package::version::parse_constraint` + `matches`);
  otherwise picks the max locked version (transitive visibility).
- Transitive case (`collections -> itertools` where the user lists only
  `collections`): the lock pins both; the synthetic-dep construction in
  `load_from` plus `bare_locked_version` finds `itertools` via the lock even
  with no direct manifest line. Package-internal bare imports resolve against
  the user's project root (entry's root), never the vendor dir's own root.

### Alias for packages
- `import "math" as m` resolves the bare name to the vendor entry, then D1
  alias rewriting applies.
- Bare package alias exposes the whole package closure (entry + all files
  under the package dir), not just the entry file, because real package
  libs are re-export shims (`lib.klang` only imports `src/*.klang`).
  All package functions share the entry's internal-name hash
  (`{orig}@{fnv(entry_canon)}`), so two aliases for one package share one
  copy (file identity, not alias identity). Calls to outside deps stay flat.
- Slash aliases (`import "os/lib.klang" as os`) keep the single-file behavior.

### Errors
- All bare failures are `E-IO-NOT-FOUND` (same code as the old
  `cannot read math` path, now with actionable messages):
  - Unknown: `cannot resolve package "foo": searched local "<a>" (and "<b>")
    and vendor ".klang_pkgs/foo/<version>/" (run: klang add foo)`.
  - Listed but not locked: `... listed in klang.toml but missing pinned
    version in klang.lock (...; run klang fetch to pin it)`.
  - Pinned but entry missing: `... pinned in klang.lock but entry file
    missing (... ".klang_pkgs/foo/<version>/lib.klang"; run klang fetch ...)`.
  - No project root: `... with no project klang.toml found (run: klang add foo)`.
- Messages name both places searched and always suggest the fix.

## S1b: real version string

- Code keeps `env!("CARGO_PKG_VERSION")` (`src/main.rs` prints
  `klang {version}` for `--version`/`-V`).
- `.github/workflows/release.yml` `build` job gains a `Stamp version from tag`
  step BEFORE any `cargo build`, gated by
  `if: startsWith(github.ref, 'refs/tags/v')`:
  - `TAG="${GITHUB_REF_NAME#v}"` strips the leading `v`.
  - Portable `bash` + `python3`/`python` fallback edits `Cargo.toml`
    `[package] version` only and the `klang` entry in `Cargo.lock`
    consistently; other versions untouched.
  - Non-tag runs (including `workflow_dispatch`) skip the step, leaving
    `Cargo.toml` untouched. Repo `Cargo.toml` stays `0.7.0`.
- Gate `tests/s1_version_gates.rs` asserts `klang --version` (and `-V`)
  equals `klang {env!("CARGO_PKG_VERSION")}`.

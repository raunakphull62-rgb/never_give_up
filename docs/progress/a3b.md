# A3b — PostgreSQL client

## Ambiguity readings (taken as specified, documented here)

- **`pg_exec` returns `{changes}` as a map.** The queue writes
  `pg_exec(pg, sql, params_array) -> {changes}`; this stage returns a
  `Map` with a single `changes: Int` key (rows affected, `INSERT`/
  `UPDATE`/`DELETE`; `0` for DDL like `CREATE TEMP TABLE`). `pg_query`
  returns `Array<Map>` (one map per row, column name -> value).
- **Null is SQL NULL both ways.** `Value::Null` (`is_null`, from
  `json_decode("null")`) binds as SQL NULL and every SQL NULL reads back
  as `Null` (`is_null(row["col"])`), consistent with A3 JSON
  (`json_decode("null")` is `Null`, `json_stringify(Null)` is `"null"`).
  There is no `null` literal; tests use `json_decode("null")` for NULL
  params and `SELECT null` for NULL reads.
- **Bool erases to Int 0/1.** Like A3 JSON (`true`/`false` decode to
  `1`/`0`), PG `BOOL` reads as `Int` (`1` true, `0` false). For writes,
  `Int` params go as INT4/INT8 (so `BOOL` columns need a literal
  `true`/`false` or an explicit `$N::bool` cast with a `bool` param,
  which Klang cannot produce from `Int`; the gate uses literals and
  documents the cast). `1::bool` is true in PG, so `$1::bool` with an
  `INT` param would need a `bool`-typed param the language has no
  literal for; the example uses literals.
- **Integers choose INT4 vs INT8 by range.** `i32`-fitting `Int`s bind as
  INT4 (accepted by SMALLINT/INT/BIGINT via implicit cast); larger `Int`s
  bind as INT8 (BIGINT; INT columns correctly reject out-of-range).
  `i64`-only binding would fail `INT` columns (`INT4` vs `INT8`
  `ToSql::accepts`), so the range split is required.
- **Float vs NUMERIC.** `Float` binds as FLOAT8 (DOUBLE PRECISION).
  NUMERIC has no driver-side decimal (no BigDecimal dep); reads without a
  cast are `E-PG-TYPE` (loud, never panic). Use `n::text` (exact decimal
  `Str`) or `n::float8` (Float) casts in SQL; the gate pins `n::text`.
  Spec allows “Float or Str (document)”: this stage documents
  FLOAT->Float, NUMERIC->cast-or-error.
- **Timestamps and JSON need no extra Klang types.** With `postgres`
  features `with-chrono-0_4`/`with-serde_json-1`, TIMESTAMP/TIMESTAMPTZ/
  DATE/TIME read as ISO `Str` (`TIMESTAMP` as `%Y-%m-%dT%H:%M:%S%.3f`,
  TIMESTAMPTZ as RFC3339, DATE/TIME as display), JSON/JSONB read as
  canonical JSON `Str`. Writes for these use `Str` params with explicit
  SQL casts (`$N::timestamp`, `$N::jsonb`) or literals.
- **Transactions are `BEGIN`/`COMMIT`/`ROLLBACK` SQL, not objects.**
  The single `Client` connection runs explicit SQL so the handle stays
  shareable (`Arc<Mutex>`). `pg_begin` when already in txn is `E-PG-TX`;
  `pg_commit`/`pg_rollback` when not in txn is `E-PG-TX`. A failed query
  aborts the txn (PG rule); caller must `pg_rollback` before further
  queries.
- **TLS: verify by default, `disable` for local.** `sslmode=disable`
  uses `NoTls` (local tests). `require`/`verify-ca`/`verify-full` use
  verified rustls (bundled WebPki roots, same as `ureq`, no second TLS
  stack). `prefer` (default) tries verified TLS first and falls back to
  plaintext ONLY when the server does not speak SSL at all (not on cert
  or auth failures). Local snakeoil certs fail verification (by design);
  local tests use explicit `?sslmode=disable`. The test helper appends
  `sslmode=disable` for loopback URLs without an explicit mode.
- **Handles close on drop, `pg_close` idempotent.** No `ExecCtx`
  registry (unlike servers/threads which join); `Client` drop closes the
  socket (open txn aborts, like libpq). `pg_close` twice is `1`; use
  after close is `E-PG-CLOSED`.
- **Sandbox denies all seven.** Every `pg_*` (including `pg_close`)
  checks `is_sandbox()` first and returns `E-SANDBOX-NETWORK`, like
  network handles (unlike A3 pure constructors which stay allowed).
- **No `pg` cargo feature.** All new crates (`postgres`,
  `tokio-postgres-rustls`, `rustls`, `webpki-roots`, `serde_json`) are
  pure Rust and build on every release target (Android/musl/Windows/
  macOS); no target needs the feature off, so no `E-PG-UNAVAILABLE`
  path is added. If a future target cannot build, gate then.

## What changed

- New file `src/stdlib/a3b_pg.rs`: URL parsing/redaction, verified TLS
  config (ring + WebPki, no native-tls/openssl), `PgState`/`PgHandle`
  (`Arc<Mutex<Option<Client>>>`, `Debug` redacts URL), `E-PG-*`
  constructors, `pg_connect`/`pg_exec`/`pg_query`/`pg_begin`/
  `pg_commit`/`pg_rollback`/`pg_close`, param binding (`Int`->INT4/INT8
  by range, `Float`->FLOAT8, `Str`->TEXT, `Null`->NULL; others
  `E-PG-TYPE`), result mapping (INT->Int, FLOAT->Float, TEXT->Str,
  BOOL->Int 0/1, TIMESTAMP/DATE/TIME->ISO Str, JSON->Str, NULL->Null,
  NUMERIC->`E-PG-TYPE` unless cast, others->Str-or-`E-PG-TYPE` via
  `try_get`, never panic), txn via SQL, sandbox denials.
- `src/runtime/mod.rs`: new `Value::Pg` (`Arc` identity eq, `0`/`0.0`,
  truthy, `<pg>`, `pg` type name); `is_builtin` + 7 `exec_builtin` arms
  (`pg_exec` returns `{changes}` map, `pg_query` returns array).
- `src/hir.rs`: `is_builtin` + arity (1 for connect/begin/commit/
  rollback/close, 3 for exec/query) + type checks (handles require
  `Unknown`, SQL `Str`, params `Array`; `pg_exec`->`Map`,
  `pg_query`->`Array`, others->`Int`).
- `src/stdlib/mod.rs`: `pub mod a3b_pg;`.
- `src/runtime/v2.rs`: `Pg` rejects in `value_to_data` (loud).
- `Cargo.toml`/`Cargo.lock`: `postgres 0.19`
  (`with-chrono-0_4,with-serde_json-1`), `tokio-postgres-rustls 0.14`
  (`ring,webpki-roots`), `rustls 0.23`, `webpki-roots 1`, `serde_json 1`
  (all pure Rust, no openssl/native-tls/libpq; `ring` already in tree).
- Tests `tests/a3b_gates.rs`: 21 gates (see below).
- Example `examples/apps/notes_api/main.klang`: notes REST API
  (`GET /health`, `POST /notes`, `GET /notes`, `GET /notes/:id`,
  `DELETE /notes/:id`) via `http_serve` + Postgres, `DATABASE_URL`/`PORT`
  (+`PORT_FILE` test hook), table created on startup, per-request
  connections, `TERM`/`INT` poll then `http_stop`, `fmt --check` clean.
- `docs/ai/stdlib.md`: 7 table rows (no fenced examples) + regenerated
  `docs/ai/KLANG_FOR_AI.md` (d1 gate fix, same deviation as A1/A2/A3/L1).
- Script `scripts/a3b_pg_test.sh`: runs `a3b_gates` with and without
  `KLANG_TEST_PG_URL` (live vs skip).

## Builtin signatures (exact)

- `pg_connect(url: str) -> Pg` (`postgres://user:pass@host:port/dbname?sslmode=...`,
  `postgresql://` also accepted; port defaults 5432, dbname defaults to
  user; `sslmode` `disable|prefer|require|verify-ca|verify-full`
  (`allow` aliases `prefer`), default `prefer`; password never in errors).
- `pg_exec(pg: Pg, sql: str, params: array) -> Map` (`{changes: int}`).
- `pg_query(pg: Pg, sql: str, params: array) -> Array<Map>`.
- `pg_begin(pg: Pg) -> Int` (`1`; `E-PG-TX` if already in txn).
- `pg_commit(pg: Pg) -> Int` (`1`; `E-PG-TX` if not in txn).
- `pg_rollback(pg: Pg) -> Int` (`1`; `E-PG-TX` if not in txn).
- `pg_close(pg: Pg) -> Int` (`1`, idempotent; use-after-close `E-PG-CLOSED`).
- Params: `Int`->INT4/INT8 by range, `Float`->FLOAT8, `Str`->TEXT,
  `Null`->NULL; others `E-PG-TYPE`. Always bound (`$1`...), never
  interpolated (injection string lands as data, gate-pinned).

## Error codes

- `E-PG-CONNECT` (bad URL scheme/host/port/sslmode, refused/unreachable,
  TLS failure; message uses redacted URL with `***`, never password).
- `E-PG-AUTH` (SQLSTATE `28P01`/`28P00` or password-auth message; never
  password, redacted marker present).
- `E-PG-SQL` (`[SQLSTATE] message`, e.g. `42P01` undefined table; never
  password; includes NUL-SQL `42602`, serialization as `08000`).
- `E-PG-TYPE` (Map/Array/Struct/Closure/handle as param; unsupported
  column type incl. bare NUMERIC; names type).
- `E-PG-CLOSED` (use after `pg_close`).
- `E-PG-TX` (begin-twice, commit/rollback-without-begin).
- `E-SANDBOX-NETWORK` (all seven in sandbox).
- Reused: `E-TYPE` (wrong Klang kinds, `pg` named), `E-ARITY`.

All messages name the offending value (URLs redacted past 200 chars,
password always `***`); all are `file: "runtime", 0, 0`; none panic.

## Limits

- One `Client` per handle (`Mutex`, one query at a time per handle;
  handlers open per-request connections to avoid txn cross-talk).
- Connect timeout 10s; refused-host fails fast (no hang).
- `Int` params: `i32` range as INT4 (covers SMALLINT/INT/BIGINT via cast),
  larger as INT8; `BOOL` writes via literals/`::bool` with bool params
  (Klang has no bool literal; `Int` 0/1 reads as BOOL, writes need
  literal or cast documented above).
- NUMERIC reads need `::text`/`::float8` casts (else `E-PG-TYPE`);
  exact decimal via `::text`.
- Timestamps via chrono (`TIMESTAMP` `%Y-%m-%dT%H:%M:%S%.3f`, TIMESTAMPTZ
  RFC3339); JSON via serde_json canonical text.
- Passwords: URL-parsed, percent-decoded, never stored in full in
  diagnostics (only redacted URL in `PgState::Debug`).

## Tests

`tests/a3b_gates.rs` (21 tests: 12 always-run + 9 live-DB, unique temp
dirs, TEMP tables, explicit closes):
always: bad scheme/host/port/sslmode (`E-PG-CONNECT`), password redaction
(unit + error JSON has `***`, no secret), refused-host `E-PG-CONNECT`,
static arity/type (`E-ARITY`/`E-TYPE`), dynamic `E-TYPE` via Unknown,
hostile NUL/no-panic, sandbox denies all seven (`E-SANDBOX-NETWORK` via
Unknown handles), JIT loud `unsupported`, v2 `E-UNDEFINED`;
live (skip cleanly without `KLANG_TEST_PG_URL`): CRUD with bound params,
NULL both ways (`json_decode("null")` param + `SELECT null` read via
`is_null`), types (INT/BIGINT/FLOAT/NUMERIC-via-`::text`/TEXT/BOOL->1/
TIMESTAMP/JSONB), injection-as-data, `E-PG-SQL` with `42P01` + no leak,
`E-PG-AUTH` with wrong pw + no leak, txn commit/rollback counts,
`E-PG-TX`/`E-PG-CLOSED`/`E-PG-TYPE`, notes_api subprocess
(`PORT`/`PORT_FILE`, `Connection: close` to avoid keep-alive shutdown
delay, CRUD + 404 + `SIGTERM` exit 0 <5s, `fmt --check`, table dropped).

## Baseline / final

- Baseline (`wave2` at `6cb3de4`, full suite): **1082 passed, 0 failed**.
- With `KLANG_TEST_PG_URL=postgres://klang:klangtest@localhost/klang_test`:
  `a3b_gates` **21/21** (12 always + 9 live ran, 0 skipped).
  Without the variable: **21/21** (12 always ran, 9 live skipped cleanly).
- Final (`cargo test --locked --no-fail-fast`): **1103 passed, 0 failed**
  (1082 + 21). `d1_stdlib_covers_every_builtin` passes after the
  `docs/ai/stdlib.md` 7-row fix.
- Live-DB run: local PostgreSQL 16.15 (`klang/klang_test`, SCRAM-SHA-256,
  snakeoil TLS `on` but tests use `sslmode=disable` for loopback) –
  all 9 live ran, 0 skipped with the variable; 0 ran, 9 skipped without.
- Clean checkout (`git worktree add /tmp/clean-a3b <final-commit>`, build
  + `a3b_gates` with and without PG URL there; worktree removed after).

## Commits (local `wave2`, human pushes)

- `3d0b3c9` A3b: Postgres client core (pg_* builtins, rustls, no libpq).
- (tests + example + docs): A3b gates + notes_api + progress docs.
- (merge? no, single stage): final A3b commit.

## Deviation: `docs/ai/stdlib.md` table rows (d1 gate vs queue rule 9)

- WAVE2_QUEUE rule 9 says `docs/ai/` changes land ONLY in L1-docs. The
  in-tree gate `d1_stdlib_covers_every_builtin` parses
  `src/hir.rs:is_builtin` and fails when any builtin is missing from
  `docs/ai/stdlib.md` (it failed listing all 7 pg names). Rule 1 (never
  let the suite go down) outranks rule 9, so this stage adds the minimal
  gate fix: 7 table rows in `docs/ai/stdlib.md` (no new fenced examples,
  so `d1_all_fenced_blocks_pass` is unaffected) + regenerated
  `docs/ai/KLANG_FOR_AI.md` via `scripts/build_ai_guide.sh` + bump
  header `100`->`107` flat. Shared docs (`reference.md`,
  `limitations.md`, `SPEC.md`, `README.md`) stay untouched per rule 9;
  the full verified A3b section still belongs to L1-docs.

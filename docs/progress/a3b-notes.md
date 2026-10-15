# A3b notes (proposed text for shared docs; NOT applied per queue rule 9)

## reference.md (builtins)

Add after the A3/L1 sections:

- `pg_connect(url: str) -> Pg` — Postgres connection (password redacted).
- `pg_exec(pg, sql, params) -> {changes}` — bound `$1` execution.
- `pg_query(pg, sql, params) -> array of maps` — `Null` for SQL NULL,
  `Int` 0/1 for BOOL, ISO `Str` for timestamps, `Str` for JSON.
- `pg_begin` / `pg_commit` / `pg_rollback` / `pg_close` — explicit
  transactions, idempotent close.

Types: `Int`->INT4/INT8 by range, `Float`->FLOAT8, `Str`->TEXT,
`Null`->NULL; others `E-PG-TYPE`. NUMERIC needs `::text`/`::float8` casts.
`BOOL` writes via literals (Klang has no bool literal).

## limitations.md

- Postgres: pure-Rust `postgres` + rustls (ring + WebPki, no libpq/C).
  SCRAM-SHA-256 on all targets. TLS verified by default
  (`require`/`prefer`); local tests use `sslmode=disable`.
- Transactions are per-connection `BEGIN`/`COMMIT`/`ROLLBACK`; a failed
  query aborts the txn (must `pg_rollback`).
- Handles close on drop; `pg_close` idempotent; use-after-close
  `E-PG-CLOSED`.

## SPEC.md / README.md

- No changes in A3b (example lives in `examples/apps/notes_api/`).

## docs/ai/stdlib.md (already applied as minimal d1 fix)

7 rows for `pg_*` (see `docs/progress/a3b.md` deviation); full verified
examples belong to L1-docs.

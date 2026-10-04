# sql

SQL query builders. Pure string construction — execution against a
real database is out of scope for Phase 1.

## API

- `sql_select(table, cols)`, `sql_where(q, cond)`, `sql_limit(q, n)`
- `sql_insert(table, cols, vals)`, `sql_quote(s)` (escapes `'`)
- `sql_eq_int(col, v)`, `sql_and(a, b)`

## Test

```sh
klang run sql_test.klang
```

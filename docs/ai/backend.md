# Klang backend guide — HTTP, threads, Postgres, gzip

Example-first. This chapter shows how to write a backend: a JSON API
with routing (`route_match` + `http_json`), an HTTP server with graceful
shutdown (`http_serve` + `signal_pending`), worker threads with channels
(`thread_spawn` + `chan`/`send`/`recv`), a Postgres CRUD API (`pg_*`,
based on `examples/apps/notes_api/main.klang`), and a logship-style gzip
job (`dt_*` + `gzip_compress`). Blocks marked `klang run` execute;
blocks marked `klang check` only parse and type-check (they need a
network or database at run time).

## JSON API with routing (runs)

`route_match` returns a params map on match, `null` (`is_null`) on
no-match. `http_json`/`http_text` build response maps.

```klang run
fn handle(path: str) -> str {
    let m = route_match("/notes/:id", path)
    if (is_null(m) == false) {
        return m["id"]
    }
    return "none"
}

fn main() -> i32 {
    print(handle("/notes/42"))
    print(is_null(route_match("/a/:x", "/b/c")))
    let r = http_json(200, {"status": "ok"})
    print(r["status"])
    return 0
}
```

```output
42
1
200
```

## HTTP server with graceful shutdown (check only — needs a port)

Bind `0.0.0.0` with `PORT` from the environment, poll `TERM`/`INT`,
then `http_stop`. `PORT_FILE` is a test hook (Render ignores it).

```klang check
fn handle(req: map) -> map {
    if (req["method"] == "GET") {
        if (req["path"] == "/health") {
            return http_json(200, {"status": "ok"})
        }
    }
    return http_text(404, "not found")
}

fn main() -> i32 {
    signal_watch("TERM")
    signal_watch("INT")
    let port_str = get_env("PORT")
    let port = 0
    if (port_str != "") {
        port = parse_int(port_str)
    }
    let s = http_serve("0.0.0.0", port, "handle", {"workers": 4})
    print(server_port(s))
    let port_file = get_env("PORT_FILE")
    if (port_file != "") {
        write_file(port_file, str(server_port(s)))
    }
    while (signal_pending("TERM") == false) {
        if (signal_pending("INT") == true) {
            break
        }
        sleep_ms(50)
    }
    http_stop(s)
    return 0
}
```

Rules: `http_serve(host, port, handler, opts)` (port `0` = any);
`server_port(s)` is `E-HTTP-CLOSED` after stop; `http_stop(s)` is
idempotent; handler errors become `500` JSON and the server keeps
running; bodies need `Content-Length` (`chunked` is `400`); sandbox
denies serve/port/stop/request with `E-SANDBOX-NETWORK`.

## Worker threads with channels (runs)

`thread_spawn` runs a named top-level function on a new OS thread;
`chan`/`send`/`recv` move values (deep-copied); `thread_join` observes
the result; unjoined threads are reaped at exit (join to see failures).

```klang run
fn double(n: i32) -> i32 {
    return n * 2
}

fn main() -> i32 {
    let ch = chan(4)
    send(ch, 21)
    print(recv(ch))
    close(ch)
    let t = thread_spawn("double", [21])
    print(thread_join(t))
    print(thread_id() == 0)
    return 0
}
```

```output
21
42
1
```

Rules: pass a handle to ONE worker (cloned `Arc`, shared socket);
`chan(0)` is `E-CHAN-CAPACITY` (use `1`); `recv` on closed-empty is
`null`; `send` on closed is `E-CHAN-CLOSED`; forgetting `thread_join`
hides `E-THREAD-FAILED`; sandbox caps spawns at 4 (`E-SANDBOX-THREADS`).

## Postgres CRUD (check only — needs DATABASE_URL)

One connection per request (no pool); params are always bound (`$1`);
`Null` (`json_decode("null")`) binds as SQL NULL and every SQL NULL
reads back as `Null` (`is_null`); `BOOL` reads as `Int` 0/1 (write BOOL
with SQL literals); `NUMERIC` needs `::text`/`::float8` casts.

```klang check
fn get_note(db_url: str, id: i32) -> i32 {
    let db = pg_connect(db_url)
    let rows = pg_query(db, "SELECT id, text FROM notes WHERE id = $1", [id])
    pg_close(db)
    return len(rows)
}

fn main() -> i32 {
    let db_url = get_env("DATABASE_URL")
    let db = pg_connect(db_url)
    pg_exec(db, "CREATE TABLE IF NOT EXISTS notes (id SERIAL PRIMARY KEY, text TEXT NOT NULL)", [])
    let rows = pg_query(db, "INSERT INTO notes (text) VALUES ($1) RETURNING id", ["hi"])
    print(rows[0]["id"])
    pg_exec(db, "DELETE FROM notes WHERE id = $1", [rows[0]["id"]])
    pg_begin(db)
    pg_exec(db, "INSERT INTO notes (text) VALUES ($1)", ["tx"])
    pg_rollback(db)
    pg_close(db)
    pg_close(db)
    return 0
}
```

Rules: `pg_exec` returns `{changes}`; `pg_query` returns array of maps;
`pg_begin` twice / `commit`/`rollback` without `begin` is `E-PG-TX`;
use-after-`pg_close` is `E-PG-CLOSED`; bad SQL is `E-PG-SQL` with
`[SQLSTATE]`; bad password is `E-PG-AUTH` (redacted `***`, never the
secret); sandbox denies all seven with `E-SANDBOX-NETWORK`; each request
opens its own connection (the example handler connects per request).

## Timestamps, JSON null, gzip job (runs)

Datetime is UTC millis (`Int`); `%z` is always `+0000`; years `1..9999`;
`tar_create` archives (does not compress — combine with gzip).

```klang run
fn main() -> i32 {
    let ms = dt_from_parts(2024, 2, 29, 12, 0, 0)
    print(dt_format(ms, "%Y-%m-%d"))
    print(dt_is_leap(2024))
    print(json_decode("null"))
    print(is_null(json_decode("null")))
    print(json_stringify(json_decode("{\"a\": null}"), 0))
    let g = gzip_compress("hello")
    print(gzip_decompress(g))
    return 0
}
```

```output
2024-02-29
1
null
1
{"a":null}
hello
```

## Full notes API (see example file)

`examples/apps/notes_api/main.klang` wires it together: `GET /health`,
`POST /notes` (`json_decode` body, `INSERT ... RETURNING id`),
`GET /notes`, `GET /notes/:id` (`route_match`, 404 when empty),
`DELETE /notes/:id` (`{changes}` decides 404), table created on startup,
`PORT`/`DATABASE_URL` from the environment, `TERM`/`INT` shutdown.

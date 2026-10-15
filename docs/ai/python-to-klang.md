# Python to Klang translation

76 common operations. Python column is for reference; Klang column was
executed (each snippet runs; full programs in `core.md` and `examples/`).
Klang has no `None`, no classes, no `in` operator, no slicing syntax.

| # | Concept | Python | Klang |
|---|---|---|---|
| 1 | print newline | `print(x)` | `print(x)` |
| 2 | print no newline | `print(x, end="")` | `print_raw(x)` |
| 3 | string concat | `"a" + str(1)` | `"a" + 1` |
| 4 | format join | `" ".join(["a", 1])` | `format("a", 1)` |
| 5 | template | `f"{a} + {b}"` | `sprintf("{} + {}", [a, b])` |
| 6 | len string (chars!) | `len("é")` | `len("é")` is bytes (2); chars via `"é".chars()` |
| 7 | len list | `len([1, 2])` | `len([1, 2])` |
| 8 | len dict | `len({"a": 1})` | `len({"a": 1})` |
| 9 | int division | `7 // 2` | `7 / 2` |
| 10 | float division | `7 / 2` | `7.0 / 2.0` |
| 11 | modulo | `7 % 3` | `7 % 3` |
| 12 | power | `2 ** 3` | `pow(2.0, 3.0)` |
| 13 | sqrt | `math.sqrt(9)` | `sqrt(9.0)` |
| 14 | abs | `abs(-5)` | `abs(0 - 5)` |
| 15 | min/max | `min(3, 7)` | `min(3, 7)` |
| 16 | int from string | `int("42")` | `parse_int("42")` |
| 17 | float from string | `float("3.5")` | `parse_float("3.5")` |
| 18 | str from value | `str(42)` | `str(42)` |
| 19 | int from float | `int(4.9)` | `int(4.9)` |
| 20 | upper | `"hi".upper()` | `"hi".upper()` |
| 21 | lower | `"HI".lower()` | `"HI".lower()` |
| 22 | strip | `"  hi  ".strip()` | `"  hi  ".trim()` |
| 23 | split | `"a,b".split(",")` | `"a,b".split(",")` |
| 24 | replace | `"aaa".replace("a", "b")` | `"aaa".replace("a", "b")` |
| 25 | contains str | `"i" in "hi"` | `"hi".contains("i")` |
| 26 | starts/ends | `"hi".startswith("h")` | `"hi".starts_with("h")` |
| 27 | repeat | `"ab" * 3` | `"ab".repeat(3)` |
| 28 | pad left | `"hi".rjust(5, "0")` | `"hi".pad_start(5, "0")` |
| 29 | lines | `"a\nb".splitlines()` | `"a\nb".lines()` |
| 30 | chars | `list("hi")` | `"hi".chars()` |
| 31 | append | `a.append(3)` | `push(a, 3)` |
| 32 | pop | `a.pop()` | `pop(a)` |
| 33 | insert | `a.insert(0, 9)` | `insert(a, 0, 9)` |
| 34 | sort copy | `sorted([3, 1])` | `sort([3, 1, 2])` |
| 35 | sort by key | `sorted(w, key=len)` | `sort_by(w, fn(s: str) -> i32 { return len(s) })` |
| 36 | slice | `a[0:2]` | `slice(a, 0, 2)` |
| 37 | range list | `list(range(0, 3))` | `range(0, 3)` |
| 38 | join | `", ".join(["a", "b"])` | `["a", "b"].join(", ")` |
| 39 | contains list | `1 in [1, 2]` | `[1, 2].contains(1)` |
| 40 | dict get | `m["a"]` | `m["a"]` |
| 41 | dict set | `m["b"] = 2` | `m["b"] = 2` |
| 42 | dict keys | `list(m.keys())` | `keys(m)` |
| 43 | dict contains | `"a" in m` | `m.contains("a")` |
| 44 | for range | `for i in range(3):` | `for i in 0..3 { }` |
| 45 | for list | `for x in [10, 20]:` | `for x in [10, 20] { }` |
| 46 | for dict | `for k in m:` | `for k in m { }` |
| 47 | while | `while i < 3:` | `while i < 3 { }` |
| 48 | if/else | `if x: ... else: ...` | `if x == 1 { } else { }` |
| 49 | and/or/not | `a and b` | `a && b` |
| 50 | function | `def f(a): return a` | `fn f(a: i32) -> i32 { return a }` |
| 51 | closure | `lambda x: x + 1` | `fn(x: i32) -> i32 { return x + 1 }` |
| 52 | try/except | `try: ... except e:` | `try { } catch e { }` |
| 53 | read file | `open(p).read()` | `read_file(p)` |
| 54 | write file | `open(p, "w").write(c)` | `write_file(p, c)` |
| 55 | file exists | `os.path.exists(p)` | `exists(p)` |
| 56 | env var | `os.getenv("X", "")` | `env("X")` |
| 57 | run command | `subprocess.run([...])` | `spawn("cmd", ["arg"])` |
| 58 | argv | `sys.argv[1:]` | `args()` |
| 59 | stdin line | `input()` | `read_line()` |
| 60 | exit | `sys.exit(5)` | `exit(5)` |
| 61 | HTTP GET | `requests.get(url).json()` | `http_request("GET", url, {}, "", 10000)["body"]` + `json_decode` |
| 62 | HTTP server | `http.server` / `flask` | `http_serve("0.0.0.0", port, "handle", {"workers": 4})` + `http_stop` |
| 63 | JSON null | `None` | `json_decode("null")` (`is_null`), no literal |
| 64 | JSON round-trip | `json.loads/dumps` | `json_decode(s)` / `json_stringify(v, 0)` |
| 65 | URL routing | `flask @route("/notes/<id>")` | `route_match("/notes/:id", path)["id"]` |
| 66 | threads | `threading.Thread` | `thread_spawn("fn_name", [args])` + `thread_join` |
| 67 | queue | `queue.Queue(n)` | `chan(n)` + `send`/`recv`/`close` |
| 68 | signals | `signal.signal(SIGTERM, …)` | `signal_watch("TERM")` + `signal_pending("TERM")` |
| 69 | sockets | `socket.create_connection` | `tcp_connect(host, port, ms)` + `conn_read`/`conn_write` |
| 70 | DNS | `socket.gethostbyname` | `dns_lookup(host)` |
| 71 | datetime now | `time.time()*1000` | `dt_now()` (epoch millis, UTC) |
| 72 | parse ISO | `datetime.fromisoformat` | `dt_parse_iso(s)` |
| 73 | format date | `strftime` | `dt_format(ms, "%Y-%m-%d")` |
| 74 | gzip | `gzip.compress(b)` | `gzip_compress(s)` (hex) / `gzip_file(src, dst)` |
| 75 | tar | `tarfile.open` | `tar_create(dir, out)` (plain tar; combine with gzip) |
| 76 | postgres | `psycopg.connect` + `%s` params | `pg_connect(url)` + `pg_query(db, sql, [params])` (`$1` bound, never interpolated) |

Verified mini-programs (each runs):

```klang run
fn main() -> i32 {
    print(7 / 2)
    print(7.0 / 2.0)
    print(pow(2.0, 3.0))
    print(parse_int("42"))
    print("ab".repeat(3))
    print(slice([1, 2, 3], 0, 2))
    return 0
}
```

```output
3
3.5
8.0
42
ababab
[1, 2]
```

```klang run
fn main() -> i32 {
    print("hi".upper())
    print("  hi  ".trim())
    print(["a", "b"].join(", "))
    print(sort([3, 1, 2]))
    print(keys({"a": 1}))
    return 0
}
```

```output
HI
hi
a, b
[1, 2, 3]
[a]
```

```klang run
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn main() -> i32 {
    print(add(1, 2))
    let f = fn(x: i32) -> i32 {
        return x + 1
    }
    print(f(1))
    print(1 && 0)
    print(!1)
    return 0
}
```

```output
3
2
0
0
```

```klang run
fn main() -> i32 {
    let m = {"a": 1}
    print(m["a"])
    m["b"] = 2
    print(len(m))
    print(m.contains("a"))
    print([1, 2].contains(1))
    return 0
}
```

```output
1
2
1
1
```

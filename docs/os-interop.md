# OS interop: files, processes, regex, env, time, HTTP

Small, synchronous building blocks for ordinary scripting work:
read/write files, run external commands with an argument array (never
a shell string), match text with regex, read environment variables,
sleep and measure elapsed time, and make HTTP requests. Every sample
below is a complete program verified against the real
`klang check` / `klang run` binary. (The precise per-builtin rules
live in the [reference](reference.md); known boundaries live in
[limitations](limitations.md).)

The PRD sketches these as `file::read`, `process::run`, `regex::find`,
`env::get`, `time::sleep`, `http::get`. Klang spells them as flat
builtins (`read_file`, `run_process`, `regex_find`, `env`,
`time_sleep`, `http_get`): `::` calls only resolve through
declared `mod` blocks, so a namespaced spelling would not type-check.
The behavior is what the PRD scopes; only the dots differ.

## Files

Write a file, then read it back. `write_file` returns the byte count
written.

```klang
// @run prints: hello klang ; return: 11
fn main() -> i32 {
    let n = write_file("/tmp/klang_docs_file.txt", "hello klang")
    print(read_file("/tmp/klang_docs_file.txt"))
    return n
}
```

Append twice, then confirm both writes landed. `append_file` creates
the file when absent and returns the byte count appended; `exists`
reports whether a path is present.

```klang
// @run prints: ab | 1 ; return: 42
fn main() -> i32 {
    write_file("/tmp/klang_docs_append.txt", "a")
    append_file("/tmp/klang_docs_append.txt", "b")
    print(read_file("/tmp/klang_docs_append.txt"))
    print(exists("/tmp/klang_docs_append.txt"))
    return 42
}
```

Reading a missing file fails loudly with a specific code — never a
silent empty string. Paths are taken literally: no globbing, no `~`
or environment-variable expansion.

```klang
// @run-fail E-IO-NOT-FOUND
fn main() -> i32 {
    print(read_file("/tmp/klang_docs_missing_xyz_12345.txt"))
    return 0
}
```

## Processes

Run a command with an argument array and read back stdout, stderr,
and the exit code as a map. Arguments travel as argv entries — a value
containing spaces or shell metacharacters arrives unmangled, because
no shell is involved. There is no shell-string execution API.

```klang
// @run prints: hi ; return: 0
fn main() -> i32 {
    let r = run_process("echo", ["hi"])
    print(r["stdout"])
    return r["exit_code"]
}
```

A non-zero exit is an ordinary result, not an error: check
`r["exit_code"]` instead of catching a failure.

```klang
// @run prints: 1 ; return: 1
fn main() -> i32 {
    let r = run_process("false", [])
    print(r["exit_code"])
    return r["exit_code"]
}
```

A command that cannot be started at all (missing binary, not
executable) fails with its own code and the real OS cause.

```klang
// @run-fail E-PROCESS-NOT-FOUND
fn main() -> i32 {
    let r = run_process("klang-docs-nonexistent-xyz-12345", [])
    return r["exit_code"]
}
```

## Regex

Test a match, or capture the matched text plus indexed and named
groups. A non-match is an ordinary result (`is_match` is false,
`find` reports `matched == 0`), not an error.

```klang
// @run prints: user@example | user | example ; return: 1
fn main() -> i32 {
    let m = regex_find("(\\w+)@(\\w+)", "user@example")
    print(m["match"])
    print(m["groups"][0])
    print(m["groups"][1])
    return m["matched"]
}
```

Named groups ride alongside the indexed ones:

```klang
// @run prints: user | example ; return: 1
fn main() -> i32 {
    let m = regex_find("(?P<user>\\w+)@(?P<host>\\w+)", "user@example")
    print(m["named"]["user"])
    print(m["named"]["host"])
    return m["matched"]
}
```

A malformed pattern fails with the underlying parse error, not a
generic message:

```klang
// @run-fail E-REGEX-INVALID-PATTERN
fn main() -> i32 {
    let b = regex_is_match("([", "text")
    return 0
}
```

## Environment variables

`env` reads one variable by name. Klang has no `Option` type, so a
missing variable is `""` — never an error.

```klang
// @run prints: default ; return: 42
fn main() -> i32 {
    let v = env("KLANG_DOCS_DEMO_MISSING_XYZ_12345")
    if v == "" {
        print("default")
        return 42
    }
    print(v)
    return 0
}
```

## Time

Block the thread, read the clock, measure durations. `time_sleep`
takes fractional seconds (something shelling out to the OS `sleep`
command cannot express); `time_now` returns seconds since the Unix
epoch; `time_elapsed` measures against a prior `time_now` reading on
the same clock. All synchronous, like `run_process`.

```klang
// @run prints: slept ; return: 42
fn main() -> i32 {
    let t0 = time_now()
    time_sleep(0.5)
    let e = time_elapsed(t0)
    print("slept")
    if e < 0.4 {
        return 1
    }
    return 42
}
```

`time_sleep` accepts an integer too, and returns 1 on success.
`time_now` is a real positive timestamp:

```klang
// @run prints: ok ; return: 42
fn main() -> i32 {
    let a = time_sleep(0)
    let t = time_now()
    if a != 1 {
        return 1
    }
    if t < 0.0 {
        return 2
    }
    print("ok")
    return 42
}
```

A negative duration fails loudly instead of sleeping zero seconds —
or forever:

```klang
// @run-fail E-TIME-INVALID
fn main() -> i32 {
    time_sleep(-1.0)
    return 0
}
```

## HTTP

A real synchronous HTTP client: `http_get(url)` and
`http_post(url, body, headers)` return a map with `status`
(int), `body` (str), and `headers` (map of lowercased name to
value). Request headers are an array of `"Name: Value"` strings —
the same array-of-strings shape as `run_process`'s argv.

```klang
// @run prints: 200 | body ok ; return: 42
fn main() -> i32 {
    let r = http_get("https://httpbin.org/get")
    print(r["status"])
    if r["status"] != 200 {
        return 1
    }
    if r["body"].contains("httpbin.org") {
        print("body ok")
        return 42
    }
    return 2
}
```

POST a body with custom headers; the echo service below sends both
back, and the assertions check the real echoed content:

```klang
// @run prints: 200 | echo ok ; return: 42
fn main() -> i32 {
    let r = http_post("https://httpbin.org/post", "docs-probe-1", ["Content-Type: text/plain", "X-Docs-Probe: yes-1"])
    print(r["status"])
    if r["status"] != 200 {
        return 1
    }
    if r["body"].contains("docs-probe-1") {
        if r["body"].contains("yes-1") {
            print("echo ok")
            return 42
        }
        return 2
    }
    return 3
}
```

An HTTP error status (404, 500, …) is an ordinary result in
`r["status"]`, not an error — parallel to `run_process`'s
non-zero exit and `regex_find`'s non-match. Only transport failures
raise, each with its own code and the real cause. These three need
no internet at all (the URL never parses / the header never
validates / nothing listens on port 9):

```klang
// @run-fail E-NET-INVALID-URL
fn main() -> i32 {
    let r = http_get("not a url at all")
    return r["status"]
}
```

```klang
// @run-fail E-NET-INVALID-HEADER
fn main() -> i32 {
    let r = http_post("http://127.0.0.1:9/echo", "x", ["no-colon-here"])
    return r["status"]
}
```

```klang
// @run-fail E-NET-UNREACHABLE
fn main() -> i32 {
    let r = http_get("http://127.0.0.1:9/")
    return r["status"]
}
```

Timeouts are 10s to connect with a 60s whole-call backstop, so a
stalled server fails loudly instead of hanging the interpreter.
Trust model, stated plainly: these calls fetch and send to whatever
URL the program passes — same unrestricted capability as
`run_process` taking a real argv array, no allowlist. Credentials
are just headers the caller provides; there is no secrets layer.

## Putting it together

File content flows into a child process through argv, and the
child's output flows into a regex — the same chain as
`examples/osio_integration.klang`:

```klang
// @run prints: contact: ada@example.com | ada@example.com | ada ; return: 42
fn main() -> i32 {
    write_file("/tmp/klang_docs_chain.txt", "contact: ada@example.com")
    let content = read_file("/tmp/klang_docs_chain.txt")
    let r = run_process("echo", [content])
    print(r["stdout"])
    let m = regex_find("([\\w.]+)@([\\w.]+)", r["stdout"])
    print(m["match"])
    print(m["groups"][0])
    return 42
}
```

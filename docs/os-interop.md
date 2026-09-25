# OS interop: files, processes, regex, env

Small, synchronous building blocks for ordinary scripting work:
read/write files, run external commands with an argument array (never
a shell string), match text with regex, read environment variables.
Every sample below is a complete program verified against the real
`klang check` / `klang run` binary. (The precise per-builtin rules
live in the [reference](reference.md); known boundaries live in
[limitations](limitations.md).)

The PRD sketches these as `file::read`, `process::run`, `regex::find`,
`env::get`. Klang spells them as flat builtins (`read_file`,
`run_process`, `regex_find`, `env`): `::` calls only resolve through
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

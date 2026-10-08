# D5. FFI / `extern`: calling system functions without a new Rust builtin

Status: design note — no implementation. (Phase 2; see the Phase 2 report
for the recommendation summary and ordering.)

## 1. Problem

Every system call a Klang program can make today is a hardcoded Rust
builtin (`read_file`, `run_process`, `env`, `http_get`, …). Adding one
more — `getpid`, `getuid`, `sysconf`, a crypto primitive, an `ioctl` —
means a compiler change, a release, and a forever-API. There is no
`extern` keyword, no `dylib`/`dlopen` surface, no syscall spelling
(Phase 0, item 9). The nearest thing is process spawning:

```text
let r = run_process("uname", ["-m"])   // argv-array, no shell; E-PROCESS-NOT-FOUND
print(r["stdout"])                     // map: stdout/stderr/exit_code
```

`run_process` is honest but coarse: no shared memory, no callbacks, no
types beyond argv/strings, and a process spawn per call. Package authors
(`os`, `fs`, `time`, `net`) feel this first — each new capability is a
core-team ticket.

## 2. Options

### Option A — declared `extern fn` against a static host table + manifest capabilities (recommended; see §3)

```text
// math/lib.klang (or user code)
extern fn host_getpid() -> i32

fn main() -> i32 {
    print(host_getpid())
    return 0
}
```

```toml
# klang.toml — capabilities are DENY by default; listing one allows it.
[capabilities]
allow_extern = ["host_getpid"]
```

Rules: `extern fn name(params) -> type` declares (never defines) a
function; the *host* provides an implementation from a **static,
in-tree table** (e.g. `host_getpid`, `host_getuid`, `host_env_count` —
pure/system-query functions only, added by maintainers, reviewed like
builtins). No `dylib` path, no `dlopen`, no raw addresses, ever. Types
at the boundary are `i32`/`f64`/`str`/`bool` only (arrays/maps/structs
do not cross — marshal explicitly). Calling an `extern` not listed in
`[capabilities]` is `E-CAPABILITY` at check time (before any run); the
checker reads the manifest, so `klang check` needs no flags. Registry
packages **cannot** silently gain capabilities: a vendored package's
`extern` requires the *root* project's `[capabilities]` entry (deps'
declarations are inert without it), and `klang add` prints the
capabilities a version declares ("adds no caps" vs "needs
`host_getpid` — confirm in klang.toml").

Pros: removes the bottleneck for the read-only/pure 80% with a
sandbox a reviewer can hold in their head; the static table means no
dynamic-loading code path exists to audit. Cons: each table entry
costs a review; past ~50 entries the table itself argues for opening
the Option B investigation.

### Option B — WASM-sandboxed native plugins

```text
[capabilities]
plugin = ["sha256:…/fast_hash.wasm"]
```

Native code ships as WebAssembly modules executed in-process by an
embedded WASM runtime, with imports limited to a capability object
(memory windows, no ambient syscalls, fuel-metered). Expressive (any
compiled language, near-native speed, real sandboxing primitive) but:
a new runtime dependency (against the std-only networking ethos),
a WASM ABI + memory-marshalling spec, determinism/fuel policy, and a
plugin signing story for the registry. All of D5-A's declaration and
manifest machinery is still needed on top.

Pros: expressive (any compiled language, near-native speed) with a
real sandboxing primitive (memory windows, fuel metering). Cons: a new
runtime dependency against the std-only ethos, plus a WASM ABI,
memory-marshalling spec, determinism policy, and plugin signing story
— L+ on its own.

### Option C — no FFI: grow `run_process` + blessed builtins only

```text
let r = run_process_json("sysinfo", ["--json"])  // hypothetical: structured stdio
```

Keep the language FFI-free; answer pressure with better process
interop (JSON stdio conventions, streaming, timeouts) and a faster
builtin-review cadence. Zero new attack surface, zero sandbox design —
and the core team stays the bottleneck for every capability, forever.
Documented here as the honest null option.

Pros: zero new attack surface, zero sandbox to design. Cons: safe and
insufficient — `run_process` cannot express `getpid` without a
fork+exec, and the builtin treadmill (the original complaint) never
ends.

## 3. Recommendation and why

**Option A.**

Why: it removes the bottleneck for the 80% case (read-only system
queries and pure host functions) with a sandbox a reviewer can hold in
their head: static table, four scalar types, deny-by-default manifest,
no dynamic loading *by construction* (there is simply no code path that
opens a library). Option B is the right answer to a different question
(arbitrary native speed/plugins) at 10× the design surface — revisit
when a package genuinely needs it, reusing A's declaration + manifest
syntax so user code does not churn. Option C is safe and insufficient:
`run_process` cannot express `getpid` without a fork+exec, and the
builtin treadmill is the complaint.

Growth path: A's table gains entries by the same review bar as builtins
today; if the table ever exceeds ~50 entries, that is the quantitative
trigger to open the B investigation — with A's call sites unchanged
(they already spell `extern fn`, which B can back with WASM modules).

## 4. Migration impact

- The 26 published packages: **no edits required**. `extern` is a new
  declaration form; existing files parse identically (`extern` becomes
  a contextual keyword in declaration position only — a function
  *named* `extern` keeps parsing, mirroring how 1b treated `as`).
  Packages may adopt `extern` for things they shell out for today
  (cosmetic majors, never forced).
- Existing programs: purely additive. No program that checks today
  changes meaning; the only new failure is calling an unlisted
  `extern` (`E-CAPABILITY`, at check).
- Manifests: `[capabilities]` is optional and absent-by-default-deny;
  old manifests (no section) allow zero externs — exactly today's
  sandbox, spelled out.

## 5. Security and sandboxing

Threat model: Klang runs AI-written, registry-fetched code. The sandbox
must assume a malicious or compromised *package*, not just a buggy one.

- **No dynamic loading, no shell, no raw pointers.** A is enforceable
  *because* the mechanism cannot express more: the host table is a
  `match` in Rust; there is no string-to-symbol resolution, so
  `"libc.so.6"` can never appear. `run_process`'s no-shell argv rule
  stays the template (never regress it for FFI convenience).
- **Deny by default, root-project consent.** Capabilities live only in
  the root `klang.toml`; transitive deps inherit nothing. `check`/`run`
  fail closed (`E-CAPABILITY`) when the manifest lacks an entry —
  including offline runs against a stale lock (the lock pins code, not
  consent).
- **Registry interaction.** Publish records declared `extern`s in the
  index (like deps); `add`/`fetch` surface them before install;
  `publish` of a package declaring `extern` requires no new auth (the
  existing owner-only token stands) but the index *displays* the cap
  list so typosquats can't hide a `host_…` behind a friendly name.
  Vendored code is still hash-pinned (`klang.lock` trust anchor
  unchanged).
- **Determinism and audit.** Table functions must be documented
  pure-or-query with their failure modes (`E-…` codes, never panics);
  time/randomness sources go through the existing seeded/testable paths
  where they exist. A `--dry-run`-style `klang caps` listing (manifest
  caps × used externs × providing table version) gives reviewers the
  one-page audit.
- **What A explicitly refuses:** writing syscalls (`kill`, `ptrace`),
  network dialing (use the audited `http_*` builtins), filesystem
  writes outside the existing file builtins' guards, and any
host-memory access. Those need Option B's stronger sandbox — or
remain builtins.

## 6. JIT / interpreter / v2 impact

- Interpreter: `extern` calls dispatch through the host table like
  builtins (`exec_builtin` shape: arity/type check + `E-…` errors);
  table functions are synchronous and side-effect-declared (pure vs
  query), so tasks/structured-concurrency rules need no change.
- v2 tree-walker: same host-table dispatch as v1 (both runtimes resolve
  declared `extern` names through one table module, so capability
  consent checked once covers `run` and `run-v2` alike).
- JIT: `extern` calls are **rejected loudly** (same arm as builtins:
  `` builtin `…` unsupported ``) until a per-entry JIT intrinsic story
  exists — pure functions first (deterministic, no error paths), queries
  with errors never (error propagation needs D6's fallible-JIT work).
  Int-only pure entries (e.g. `host_getpid`) are the obvious first
  intrinsics; each is an S-sized, separately gated addition.
- No new MIR ops required (lower to `Call` with an `extern::` prefix or
  a flag on the existing op — implementation choice, invisible to
  programs).

## 7. Test plan

- Checker: unlisted `extern` → `E-CAPABILITY` naming the missing
  manifest entry; listed-but-unknown host name → `E-UNDEFINED`
  suggesting the table; dep-declared `extern` without root consent →
  `E-CAPABILITY` (root project fixture with a vendored dep).
- Types: struct/array/map param or return on `extern` is `E-TYPE` at
  declaration.
- `as`-style safety: a user `fn extern()` still parses and runs
  (contextual-keyword gate).
- Sandbox: table review checklist per entry (purity, failure codes,
  determinism note); `klang caps` output golden-tested; registry index
  shows cap lists (ephemeral-registry test in the B5/T10 style).
- Back-compat: all 26 `*_test.klang` unchanged; new
  `tests/extern_gates.rs` pins the above; JIT rejects each table entry
  loudly until intrinsics land (then per-entry differential gates).

## 8. Effort estimate (S / M / L) and risks

**L** overall (table + declaration checking + manifest consent + index
surfacing + `klang caps` + gates), sliceable: a 3-entry query-only
prototype (`getpid`-class) is **M** and proves the whole shape. Option B
would be **L+** on its own (runtime vendoring, ABI, signing).

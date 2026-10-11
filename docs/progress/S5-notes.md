> merged into shared docs in S8.

# Wave1 S5 notes (not a shared doc — staging area)

Text that belongs in shared docs (SPEC.md, docs/reference.md,
docs/limitations.md, docs/LANGUAGE_STATUS.md, docs/design/README.md)
goes here until owners approve it. Do NOT copy into those files in this
wave.

## JIT limit adopted (matches the interpreter)

- Call-depth guard: the JIT fails a call when the current depth is
  already above 1024 (`depth > 1024`), exactly like the v1
  interpreter's `exec_function` guard (`src/runtime/mod.rs`) and the v2
  tree-walker. The entry frame counts as depth 0 on all three backends,
  so `countdown(1023)` runs everywhere and `countdown(1024)` fails
  everywhere with `E-RUNTIME` / `call depth exceeded (possible
  recursion)` (cause `call stack depth limit reached`, rule
  `runtime/execution`), exit 1. Very deep JIT recursion now reports
  instead of aborting natively.

## Shared-doc text staged for later (verbatim proposals)

docs/limitations.md — replace the JIT-vs-interpreter divergence note:

```text
## JIT parity (D6, Wave1 S5)

`--backend-jit` is opt-in and int-only; programs outside its subset
still fail loudly with `JIT-FAIL` (never silently). Inside the subset
it now agrees with the interpreter exactly: division/modulo by zero
(`E-RUNTIME`), 64-bit overflow in `+ - * / %` and unary `-`
(`E-OVERFLOW`, never wraps), the 1024-frame call-depth guard
(`E-RUNTIME`), and the `i32`/`u32`/`u64`/`u8` argument/return/
assignment boundaries (`E-RUNTIME`, rule `types/range`).
```

docs/LANGUAGE_STATUS.md Phase-1-update style addendum (for the next
auditor, not this wave's call):

```text
- Item 6 (Wave1 S5, D6): the int-only JIT is now exact on its subset —
  checked arithmetic, div/mod-by-zero, MIN/-1, the 1024-frame depth
  guard, and D3 narrow boundaries all report the interpreter's
  diagnostics (`tests/jit_parity_diff_gates.rs`, `tests/jit_s5_gates.rs`).
  Rejection (`JIT-FAIL`) remains the contract for out-of-subset code.
```

SPEC.md §8 (backends) paragraph proposal:

```text
The `--backend-jit` Cranelift backend accepts the int-only MIR subset
and reports the same diagnostics as the interpreter on it (overflow,
division/modulo by zero, depth guard, integer boundaries); anything
else is a loud `JIT-FAIL` at compile time, never a silent wrong
answer. The interpreter is the default and the reference.
```

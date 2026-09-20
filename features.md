# Language Features

## v0.1 (Stage 1 - verified complete)
- Built-in effects: throws, async, cancel
- Structured concurrency: task_group, spawn, await, compile-time
  detection of unawaited/leaked tasks (E-TASK-CANCEL,
  E-SPAWN-OUTSIDE-GROUP)
- Structural, per-scope node identity (no global counters; every
  child's path is generated while its parent's scope is still entered)
- Structured JSON-object diagnostics
- Managed memory mode only
- Single blessed CLI, including a working formatter (klang fmt)
- Basic package manifest + lockfile with tamper detection
- Salsa-based incremental compilation, verified via salsa_gates and
  salsa_spike test suites

v0.1 is verified: 79 tests passing across 12 test files, including the
exact parent-prefix containment and sibling-uniqueness assertions that
twice shipped broken in earlier attempts.

## v0.2 - Maturity Phase (current)

Goal: make Klang expressive enough to eventually host its own compiler.
Self-hosting itself is NOT part of this phase - only the language
features a compiler would need to exist first.

### 1. Enums with pattern matching
- Tagged sum-type declarations, variants may carry data
- `match` expressions/statements that destructure variants
- Exhaustiveness checking as a real compile-time diagnostic (not a
  runtime panic) when a match does not cover all variants

### 2. Generics
- Generic type parameters on structs, enums, and functions
- Type inference for generic calls where the type argument is
  unambiguous from the arguments given
- At minimum: a generic list-like container and a generic key-value
  container, since a compiler's own AST/symbol-table representation
  needs both

### 3. Multi-file module system
- Named modules, explicit pub/private visibility on declarations
- Qualified cross-module references (e.g. lexer::Token)
- A private cross-module reference is a compile error, not a silent
  success

### Mandatory test requirements for v0.2 (same discipline as v0.1)
Every test must be a real #[test] with assert!/assert_eq!/assert_ne! -
not a print-based demo. At minimum:
- Enum: correct destructuring of a data-carrying variant; a
  non-exhaustive match is a compile-time diagnostic with a real error
  code
- Generics: two instantiations of the same generic with different
  concrete types do not share or corrupt state; a type mismatch in a
  generic call site is a compile-time diagnostic
- Modules: a private declaration is a compile error when referenced
  from another module; a pub declaration is successfully callable via
  its qualified path; same-named private items in different modules do
  not collide

## Explicitly Out of Scope for v0.2
- Self-hosting the compiler or any self-hosted compiler component
- Anything already present in the codebase beyond v0.1's original scope
  (Salsa internals, JIT, LSP, ownership modes, package manager
  internals) - leave as-is unless it directly blocks one of the three
  features above
- Generic trait bounds / typeclasses (deferred to a later phase)
- Full ownership/borrow modes (still deferred per v0.1's original scope)

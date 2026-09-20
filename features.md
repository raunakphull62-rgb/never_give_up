# Language Features (v0.1 scope)

## Built-in Effects (only these three at first)
- `throws` - typed, propagatable failure (Rust `Result`-style, not exceptions)
- `async` - marks a function as performing asynchronous work
- `cancel` - cooperative cancellation signal within a task tree

Effects are inferred where possible but must be visible in a function's
signature when they affect callers.

## Structured Concurrency
- `task_group { ... }` creates a lexical scope owning all tasks spawned inside it
- `spawn expr()` creates a child task tied to the enclosing task_group
- A child task that is not `await`-ed or explicitly detached before its
  `task_group` scope ends is a **compile error**, not a runtime leak
- Start single-threaded with lexical task groups only. No detached tasks,
  no arbitrary spawning, no multiple async runtimes in v0.1.
- Failure propagation: lossless by default (Python `ExceptionGroup`
  semantics) - one child failure returns unwrapped, several become
  `E-TASK-GROUP` with each failure under `related`; siblings are
  cancelled (loop back-edges are checkpoints) and joined, never detached

## Type System
- Explicit absence (no implicit null)
- Type inference for local bindings; explicit types on public function signatures
- `throws` and `async` are the only visible effect markers in v0.1
- Generics deferred past v0.1

## Error Handling
- `throws` functions return a typed failure channel, propagated with a
  low-friction operator, not exceptions
- Every compiler-raised error is a structured diagnostic object (see
  ARCHITECTURE.md), never a bare string

## Metaprogramming (deliberately minimal in v0.1)
- Only typed, append-only `derive`-style declarations are allowed
- Every generated node carries an origin chain back to its source
- No arbitrary procedural macros in the first release

## Toolchain (single blessed CLI)
- `warden build` / `warden run` / `warden test` / `warden fmt` / `warden check` / `warden doc`
- One package manifest as the single source of truth
- Content-addressed lockfile for reproducible builds

## Memory Safety (progressive)
v0.1 ships **Managed mode only** (tracing/reference-counted safe runtime,
no lifetime annotations required). Value/Owned/Unsafe modes are deferred.

## AI-Legibility (the differentiator)
- Every diagnostic is machine-parseable JSON
- Executable contracts/preconditions, not full theorem proving
- Killer demo target: the same subtly-wrong AI-generated function that
  compiles silently in Python/Java fails to compile here with an exact,
  fixable reason

## Mandatory Test Requirements (added after two shipped scoping bugs)

These are not optional "nice to have" tests - a Stage 1 implementation is
not complete without them, and no PR should be merged without them passing:

1. **Sibling uniqueness at every depth.** For each of: two top-level
   functions, two statements inside the same block, two nested task_groups
   - assert their generated NodeIds/paths are distinct.

2. **Parent-prefix containment.** For every node type that has children
   (`FunctionDecl`, `TaskGroup`, `Block`), assert every child's path
   `starts_with()` the parent's own path, generated *while the parent's
   scope is still entered*. This is the exact assertion that would have
   caught the task_group bug before it shipped twice.

3. **No dead/duplicate core types.** A CI check (`grep -c "struct NodeId"
   src/*.rs` equal to 1, same for `struct Parser`) - this project has
   twice accumulated a second, unused copy of these types in a different
   file while fixes were applied only to one of them.

4. **Print real values, not narration.** Any test/demo function whose job
   is to prove a property (e.g. `demonstrate_sibling_uniqueness`) must
   assert the property in code (`assert_ne!`, `assert!(x.starts_with(y))`)
   and panic on failure - not just print a success message string.

## Explicitly Out of Scope for v0.1
- General algebraic effect handlers
- Arbitrary procedural macros
- Multi-threaded/parallel task execution
- Production-grade borrow checker / ownership modes beyond "managed"
- A theorem prover or full refinement-type system

# Phase 0 — Full Foundation Audit (code-verified)

Scope: the audit table the PRD's Phase 0 requires — every roadmap item and
subsystem, each cell backed by file/line evidence, test names, and real
pass/fail runs. No cell is filled from a planning document's claim alone.

Method: (1) source inspection, (2) `cargo test` on a real toolchain
(`cargo 1.98.1`, target dir moved off the non-executable mounted
`target/`), (3) live probes against the built binary
(`target/debug/klang check|run|repair`), (4) adversarial dogfood programs
combining generics + enums + modules + effects.

Environment note: this container has no GPU/LLM endpoint, so Phase 3
(live-model benchmark) cannot run here; everything else is executed, not
narrated.

Suite totals: **172 tests, 0 failed** (`cargo test`, 22 suites).

---

## 1. Roadmap items

### 1.1 Enums + pattern matching
- **Implemented?** Yes. `EnumDecl`/`EnumVariant` (`src/ast.rs:92-107`),
  `parse_enum` (`src/parser.rs:921`), `Expr::Match`/`MatchArm`
  (`src/ast.rs:302,519`), `check_match` (`src/hir.rs:1680`), MIR tag
  dispatch (`src/mir.rs:342-375`), fmt (`src/fmt.rs:117,278`).
- **Tested?** Yes, 15 tests in `tests/enum_gates.rs` (was 3 before this
  audit): dispatch, wildcard, exhaustiveness positive/negative, missing-all
  naming, duplicate arm, post-wildcard unreachable, binding arity,
  binding-type flow, non-enum scrutinee, unknown variant, cross-enum arm,
  repair end-to-end. Plus `tests/module_gates.rs:81` (qualified enums) and
  `tests/generic_gates.rs:62` (generic enums).
- **Documented?** Yes — `SPEC.md` §2b now states the real syntax
  (tuple-style `Variant(x: i32)`, `Enum::Variant(bindings)`) and the real
  codes (`E-MATCH-EXHAUSTIVE`, `E-MATCH-DUPLICATE`, `E-MATCH-UNREACHABLE`).
- **Repair-compatible?** Yes, verified 3 ways: `plan_scope` pins the
  function containing the match; the structured `E-MATCH-EXHAUSTIVE` JSON
  reaches the prompt; `enum_nonexhaustive_repair_end_to_end` converges on a
  mock backend; a live `--dry-run` against the binary shows the diagnostic
  JSON inside the prompt with zero model calls.
- **Known gaps?** PRD-draft code names differ from shipped names
  (`E-MATCH-NOT-EXHAUSTIVE`/`-NOT-ENUM`/`-FIELD-MISMATCH` →
  `E-MATCH-EXHAUSTIVE`/`E-TYPE`/`E-ARITY`); struct-style variants,
  partial `{ field, .. }` destructuring, and match guards are NOT in v1.
  Generic enums already exceed the PRD's NG1.

### 1.2 Generics
- **Implemented?** Yes. `type_params` on struct/enum/fn (`src/ast.rs:98,115,324`),
  `parse_type_params_opt` (`src/parser.rs:709`), rigid `Ty::Param` +
  `unify_generic`/`substitute_ty` (`src/hir.rs:100,1127,1166`).
- **Tested?** Yes, 6 tests in `tests/generic_gates.rs`: two instantiations
  of a generic fn, generic struct, swapped KV struct, generic enum match,
  conflicting instantiation → `E-TYPE`, fmt round-trip.
- **Documented?** Yes — `SPEC.md` §7 now lists generics as shipped (the
  old §7 said "no generics", which was stale).
- **Repair-compatible?** Yes: live probe P5 shows `E-TYPE` ("`same` arg 1:
  want i32, got str") and `repair --dry-run` scopes it to
  `function(s): same`.
- **Known gaps?** No trait bounds/typeclasses (declared out of scope). No
  generic methods on builtins; element types of arrays/maps stay dynamic.

### 1.3 Multi-file modules
- **Implemented?** Yes. `mod` parsing (`src/parser.rs:843`), flattening +
  visibility (`src/modules.rs:158-272`), `E-PRIVATE`
  (`src/diagnostics.rs:171`), plus cross-file `import` merging with
  `with_file_prefix` identity (`src/ast.rs:122`, `src/main.rs:548`).
- **Tested?** Yes, 10 tests in `tests/module_gates.rs` (2 added by this
  audit), incl. private-vs-pub, same-named private items in different
  modules, qualified struct construction + annotation, qualified enum
  match, identity containment, cross-file module use, fmt round-trip.
- **Documented?** Yes — `SPEC.md` §1/§7.
- **Repair-compatible?** Yes for functions; declaration-level privacy
  fixes now escalate to file scope (see F5 below).
- **Known gaps?** See **F1** (fixed during this audit).

---

## 2. Subsystems

| Subsystem | Implemented | Tested | Documented | Repair-compatible | Known gaps |
|---|---|---|---|---|---|
| Parser | `src/parser.rs:1903` lines, recursive descent + structural scopes | `tests/parse_gates.rs` (3), `tests/lang*_gates.rs`, all suites parse | `SPEC.md` §1-§3 | n/a (parse errors reach the loop as `E-PARSE`) | no match guards / tuple variants / partial destructuring |
| HIR / type checker | `src/hir.rs:2236`, effect + name + type + enum + generic checks | `type_gates` (13), `enum_gates` (15), `generic_gates` (6), `tasks_gates` (4) | `SPEC.md` §2, §2b | Yes — `Diagnostic::to_json()` is the prompt payload | most `primary_span`s are `0,0` (see F2/F6) |
| Effect system | `throws`/`async`/`cancel`, `E-EFFECT-MISMATCH`, `E-TASK-CANCEL` | `lang_gates:137,146`, `stage1_gates`, `tasks_gates` | `SPEC.md` §3 | Yes — caller attribution pinned by tests | effect union across calls is conservative |
| Structured diagnostics | `src/diagnostics.rs` (252), JSON + `related` | `foundation_gates`, `type_gates`, `enum_gates` | `architecture.md`, `SPEC.md` | Yes (the contract the loop consumes) | `start/end` often `0,0` |
| `klang repair` | `src/repair/*` (1669 lines): config, backend, prompt, scope, splice, driver, log | `repair_gates` (**19** tests, +6 this audit) + unit tests in each module | `SPEC.md` §5 + scope-boundary rules | n/a (it is the mechanism) | declaration-level edits handled by escalation, not by scope-local editing (F5) |
| Formatter | `src/fmt.rs` (339) | `toolchain_gates` (3), round-trips in `module_gates`/`generic_gates` | `SPEC.md` §5 | Yes | no enum/match-specific golden test beyond round-trip |
| Package manager | `src/package.rs` (220), manifest + FNV lockfile | `package_gates` (3) | `SPEC.md` §6 | n/a | FNV is change-detection, not crypto |
| MIR + runtime | `src/mir.rs` (1104), `src/runtime.rs` (1316) | `foundation_gates`, `lang*_gates`, `tasks_gates` | `SPEC.md` §4 | n/a | interpreter default; JIT is int-only |
| Salsa DB | `src/db.rs` (155) | `salsa_gates` (4), `salsa_spike` (2) | `db.rs` header | n/a | single-file inputs only |
| LSP / ownership / derive / contracts | JSON renderer, Managed-only modes, append-only derive, `repair_loop` | `foundation_gates` (9) | `SPEC.md` §8 | `repair_loop` is the loop primitive used by `klang repair` | no real LSP server; ownership modes deferred |

---

## 3. Live probe log (built binary, this container)

| Probe | Input | Result |
|---|---|---|
| P1 | `enum Opt<T>` + match | `check: OK` |
| P2 | `struct Box<T>` | `check: OK` |
| P3 | `fn identity<T>` | `check: OK` |
| P4 | generic struct + plain struct | `check: OK` |
| P5 | `same(1, "hi")` | `E-TYPE` (`want i32, got str`) |
| P6 | `mod lexer { pub fn scan }` + call | `check: OK`, `run main() = 42`, MIR shows `lexer::scan` |
| P7 | private cross-module call | `E-PRIVATE` |
| P8 | duplicate `fn main` | parse-time `E-DUPLICATE` |
| P9 | `import "mylib.klang"` | merged `check: OK`, `run main() = 42` |
| P10 | `add(1)` | `E-ARITY` |
| P11 | `return nope + 1` | `E-UNDEFINED` |
| P12 | non-exhaustive match | `E-MATCH-EXHAUSTIVE` naming `Opt::None` |
| P13 | enum + struct + generic + mod + effects in one program | **FAILED before F1 fix**, `run main() = 42` after |
| P14 | `repair --dry-run` on P5 | scope `function(s): same`, zero model calls |
| P15 | `repair --dry-run` on P12 | scope `function(s): f`, diagnostic JSON in prompt |

---

## 4. Findings (each fixed-with-regression-test or explicitly triaged)

### F1 — BUG (high): modules double-counted top-level declarations — FIXED
- Symptom: any file containing BOTH a `mod` block and a top-level
  `enum`/`struct` was rejected with spurious `E-DUPLICATE` diagnostics and
  could not be checked or run at all (probe P13: 3 duplicates for valid
  code). This breaks exactly the v0.2 combination the roadmap targets.
- Cause: `src/modules.rs:209-215` seeded the flattened program with
  `program.enums.clone()`/`structs.clone()` and then lines 222-239 pushed
  the same (rewritten) top-level items again. The `mods.is_empty()` early
  return at line 159 masked it for module-free files, and no existing gate
  combined the two constructs.
- Fix: initialize `flat.structs`/`flat.enums` empty; the top-level loops
  are the single source of truth (`src/modules.rs:209-221`).
- Regression tests: `module_with_top_level_enum_no_spurious_duplicates`,
  `module_with_top_level_struct_no_spurious_duplicates`
  (`tests/module_gates.rs`) — assert clean check, flattening counts, and
  execution.
- Verified live: probe P13 now `check: OK` + `run main() = 42`.

### F2 — BUG (medium): `match/*` diagnostics over-widened repair scope — FIXED
- Symptom: `E-MATCH-EXHAUSTIVE` names the enum, so the use-site scan put
  every function mentioning that enum into scope (`pick, main`), and a
  single-function model response was then refused as "missing repaired
  function `main`" — the repair could not converge even when correct.
- Cause: `scope::attribute` fell through to the generic token scan.
- Fix: `match/*` diagnostics prefer the function whose body both contains
  a `match` and mentions the diagnostic's non-function token
  (`src/repair/scope.rs` step 2b).
- Regression test: `match_diagnostic_scopes_to_function_containing_the_match`
  (`tests/repair_gates.rs`) — asserts `Functions(["pick"])` exactly, plus
  convergence with declarations and untouched functions preserved.
- Verified live: a fn-only model response now converges (`repair: OK in 1
  attempt(s)`) with enum + `helper` + `main` byte-identical.

### F3 — BUG (high): function-scope splice silently dropped declaration edits — FIXED
- Symptom: if the correct fix was a declaration edit (or the model re-emitted
  declarations), the splicer replaced only the target `fn` and silently
  discarded the declaration text; the loop then re-failed with the original
  diagnostic, or worse, wrote a file that no longer matched the model's fix.
- Fix: `splice::response_declarations` detects `enum`/`struct`/`mod`/`import`
  text outside function spans (statement-position lexing, so comments and
  strings never register). A response containing declarations is accepted
  only as a complete file (all original functions AND declarations present);
  otherwise it fails loudly with a message naming what is missing. Full
  rationale and ordering in `src/repair/splice.rs`.
- Regression tests: `declaration_edit_in_incomplete_response_is_never_silently_dropped`,
  `declaration_edit_with_complete_file_is_accepted`,
  `declaration_free_response_preserves_original_declarations`,
  `declaration_detection_ignores_comments_and_strings`
  (`tests/repair_gates.rs`).
- Verified live: incomplete-declaration response → loud failure, exit 1, no
  output file written; complete-file response → success; fn-only response →
  success with declarations untouched.

### F4 — DOC DRIFT: `SPEC.md` claimed "no generics" — FIXED
- Generics shipped (`tests/generic_gates.rs`) but §7 listed them as absent,
  and enums/modules were not listed anywhere. `SPEC.md` §7/§8 rewritten
  against actual behavior.

### F5 — DECISION (Phase 1 requirement): declaration-level repair boundary — IMPLEMENTED
- Decision: declaration-level diagnostics never claim function scope.
  `scope::DECLARATION_RULES = ["names/duplicate", "modules/visibility",
  "ownership/mode"]` + `is_declaration_level()`; `plan_scope` returns
  `File` for them. Rationale: a declaration edit is not expressible by
  function-scoped splicing, and name-keyed splicing is ambiguous exactly
  when two items share a name.
- Boundary control test proves body-level diagnostics still take the
  function-scope path: `declaration_level_diagnostics_force_file_scope`.
- Documented in `SPEC.md` §5 and `src/repair/scope.rs`.

### F6 — TRIAGED (not changed): `MAX_FUNCTION_SCOPE = 2` has no recorded rationale
- Single named constant (`src/repair/scope.rs:14`), referenced once
  (line ~190). No justification is recorded for 2 vs 3/4 — stated plainly,
  not invented. Raising it is a one-line change with no structural
  knock-ons; the only real effect is prompt size (`prompt.rs:47-65` embeds
  every target's source plus the full file). Left at 2 deliberately.

### F7 — TRIAGED (known limit): most HIR diagnostics carry `0,0` spans
- `primary_span` is `input.warden,0,0` for most HIR diagnostics, so scope
  attribution is heuristic (name/use-site based) rather than span-derived.
  Mitigations in place: full-program re-check after every splice, loud
  refusal instead of silent drops (F3), and declaration escalation (F5).
  Threading real token spans through HIR checks remains the principled fix
  and is the largest known debt in the repair path.

---

## 5. Phase 0 exit criteria

Every cell above is filled with file/line references, named tests, or live
probe output. Three real bugs (F1, F2, F3) and one doc drift (F4) were found
by doing this audit; all three bugs are fixed with permanent regression
tests, and none were known before this pass.

## 6. Test-count delta (this audit)

| Suite | Before | After |
|---|---|---|
| `enum_gates` | 3 | 15 |
| `module_gates` | 8 | 10 |
| `repair_gates` | 13 | 19 |
| `robustness_gates` (new) | 0 | 4 |
| **Total (all suites)** | **152** | **176** |

All 23 suites pass: `cargo test` → `176 passed; 0 failed`. Examples
regression: `examples/{simple,full,lib,stdlib_demo}.klang` all
`check: OK` and `main()` runs to 42; the no-argument gate demo still
prints `all klang full-language stages hold`.

## 7. Phase 2 — bug-finding budget (set explicitly, per PRD risk row)

Proposed budget, to be honoured as-is unless revised in writing:
- **4 sessions**, each time-boxed to one subsystem sweep, in this order:
  (1) parser+fmt fuzz/edge cases, (2) HIR/type-checker adversarial cases,
  (3) repair-loop adversarial sessions across every diagnostic rule
  (`names/duplicate`, `modules/visibility`, `match/*` were untested before
  this audit — they are the highest-yield targets), (4) dogfood a
  non-trivial program using generics + enums + modules + effects together.
- Exit: every finding either fixed with a regression test or recorded here
  as a documented limitation. No finding may be dropped silently.
- Session 3 was effectively started during this audit and produced F1-F3.

## 8. Phase 3 — benchmark status

Not runnable in this environment: it requires at least two live model
endpoints (no GPU/network model access here), and the PRD sequences it
strictly after Phase 2. What exists for it already:
- offline harness pieces: `MockBackend` (deterministic multi-response queue),
  `.klang-repair-log.json` attempt records, `iters_used`, and
  `--dry-run` for zero-cost prompt inspection;
- the metrics the PRD asks for map onto existing fields: syntax validity and
  semantic correctness come from `check_candidate`/`runtime::run_with_output`,
  iterations-to-convergence from `RepairOutcome::iters_used`, and the
  "almost-right" category from the diagnostic codes
  (`E-TYPE`/`E-ARITY`/`E-EFFECT-MISMATCH`/`E-MATCH-EXHAUSTIVE`).
- Remaining work for Phase 3: task corpus (simple/moderate/complex tiers),
  two-endpoint runner script, and a results table checked into the repo.

### F8 — BUG (high): deep expressions aborted the process — FIXED
- Found by Phase 2 Session 1 (parser/HIR fuzzing), not by any hand-written
  test: `fn main() -> i32 { return 1+1+...+1 }` with ~800+ operands parsed
  fine, then blew the stack during checking and aborted with
  `fatal runtime error: stack overflow` (exit 134). Bisected: 400 operands
  OK, 800 aborts. Cause: the AST for a flat left-associative chain is
  left-deep, so `check_expr`/`lower_expr_to_value`/`emit_listing` recurse
  once per operand on the default 8 MiB main-thread stack.
- Why it matters here specifically: this project's value proposition is
  "AI mistakes surface as compile errors, not silent runtime bugs". A
  compiler that *aborts* instead of reporting on large generated input
  breaks that promise at exactly the input class AI produces.
- Fix: `klang::with_deep_stack` / `klang::DEEP_STACK_BYTES` (256 MiB worker
  thread) in `src/lib.rs`; `main.rs` runs the whole CLI inside it.
- Regression tests: `tests/robustness_gates.rs` —
  `deep_flat_expression_does_not_crash_the_compiler` (2000 operands:
  parse → check → lower → listing),
  `deep_flat_expression_runs_and_returns_expected_value` (1000 operands
  → 1001, i.e. correctness not just survival),
  `deep_nesting_does_not_crash_the_compiler` (200 nested parens, 300 nested
  blocks), `malformed_input_is_a_diagnostic_never_a_panic` (empty file,
  truncated `fn`, unterminated string/block comment, garbage bytes, 100 KB
  comment, 2000 statements).
- Verified live: the original repro now `check: OK` (exit 0) and a
  20 000-operand chain also checks clean.
- Residual limit (triaged): the ceiling is now ~10^5 operands rather than
  ~10^3; an explicit AST-depth limit with a dedicated diagnostic would be
  the principled bound and is recorded as remaining debt.

---

# Phase 2 Completion — Sessions 2 & 4 (code-verified)

Status: done. Session 1 (parser/fmt fuzzing) and Session 3 (repair-loop
sweep) already ran (F1, F2, F3, F8). This pass closes the two open
sessions against the same toolchain (`cargo 1.98.1`,
`CARGO_TARGET_DIR` off the non-executable mount) with the same
discipline: every fixture tried against the real binary/checker, every
finding fixed-with-regression-test or triaged in F-format below.

Suite totals: **201 tests, 0 failed** (`cargo test`, 26 suites; was 177
at Phase 0 close — the PRD's "176 baseline" is off by one against the
measured tree, which already contained `benchmark_gates`).

## Session 2 — HIR / type-checker adversarial cases

Every PRD §4.2 category was probed live (`klang check`/`run` per
fixture), not just reasoned about:

| Category | Fixtures tried | Verdict |
|---|---|---|
| Generic multi-site inference | `same(1,2)` + `same("a","b")` in one fn; `id(1)`/`id("hi")` with leaking returns | correct: fresh subst per site; leaks diagnose `E-TYPE`. Pinned (`generic_per_site_substitution_stays_independent`) |
| Recursive generic instantiation | `fn f<T>(x: T) -> T { return f(x) }`; generic fn passed as value (`id(id)`) | clean / `E-UNDEFINED` (functions are not values) — no crash, no silent accept. Pinned |
| Enum/generic interaction | `Opt<Opt<i32>>` nesting + match; `match` on generic enum inside generic fn; two `Opt` instantiations in one program | all correct, runtime values verified |
| Module/type interaction | generic struct in one module instantiated with another module's type (`a::Box` + `b::Token`); qualified annotations; `module::Type<T>` syntax | cross-module runs correct (40); `a::Box<i32>` correctly rejects as `E-PARSE`. Pinned |
| Effect edge cases | `throws` call in match arm; propagation through for/if/while/match nesting; generic `throws` fn; un-annotated deep chain | all diagnose `E-EFFECT-MISMATCH` when un-annotated, clean when annotated. Pinned |
| `unknown` escape hatch | map lookup as struct arg (stays lenient — pinned); generic-field arithmetic; generic match bindings; unbound `T` returns | leniency confirmed by design (SPEC §2); runtime follows spec (`"hi"+1` → `"hi1"` concat, verified via `print`). No change |
| Span accuracy (F7 input) | every diagnostic code probed for `primary_span` | inventory pinned (see below); only 4 codes carry real spans |

### F9 — BUG (high): struct literals with missing fields checked clean, failed at runtime — FIXED
- Symptom: `struct P { x: i32, y: i32 }` + `P { x: 1 }` passed `check`
  (exit 0) and failed only at `run` with `E-RUNTIME unknown struct
  field`. Empty literals (`P { }`), generic structs, and qualified
  (`lexer::Token`) construction were all affected. Extra fields already
  reported `E-UNDEFINED`, so the missing side was pure asymmetry.
- Cause: `check_expr`'s `StructLit` path (`src/hir.rs`) validated only
  provided fields (name exists, value unifies) and never diffed against
  the declared field set.
- Fix: after the field loop, diff declared vs. provided names; any
  missing set reports one `E-ARITY` diagnostic naming every missing
  field (`` `P` literal is missing field `P.y` ``), rule
  `calls/arity` to match the enum-payload-arity precedent.
- Regression tests: `tests/type_adversarial_gates.rs` —
  `struct_literal_missing_field_is_diagnostic`,
  `struct_literal_missing_all_fields_lists_each`,
  `struct_literal_missing_field_in_module`,
  `struct_literal_missing_and_extra_reports_both` (both codes together),
  `struct_literal_full_still_runs` (no false positive).
- Verified live: the original repro now fails `check` with `E-ARITY`;
  complete literals still `check: OK` and run.

### F10 — BUG (high): struct annotations erased to `Unknown` in signatures — FIXED
- Symptom: struct-typed parameters and returns were never checked at
  call sites — `f(99)` for `f(t: T)`, `-> Token { return 42 }`,
  cross-struct `f(b: B)` for `f(a: A)`, nested `Outer { inner: 42 }`,
  `o.inner.y` typos, and enum-typed fields (`S { v: 42 }`,
  non-exhaustive `match s.v`) ALL checked clean and failed only at
  runtime (or silently). Enum parameters were always nominal, so the
  asymmetry proves intent: `E-TYPE` `` `f` arg 0: want Opt, got i32 ``
  worked while the struct twin did not.
- Cause: three compounding erasures in `src/hir.rs` — (1) signature
  building used `resolve_generic_ty`/`resolve_param_ty`, which never
  consulted the struct table (bodies already did via
  `resolve_body_ty`, an earlier partial fix); (2) non-generic struct
  fields used bare `parse_ty`, erasing even enum-typed fields and
  cascading into unchecked nested literals, field access, and match
  exhaustiveness through the field; (3) non-generic enum payloads used
  bare `parse_ty` likewise.
- Fix: `resolve_param_ty`/`resolve_generic_ty` take the flattened
  struct-name table and resolve known structs (including `m::T`
  paths) to nominal `Ty::Struct`; struct names are precomputed before
  the enum loop so payloads resolve too; non-generic struct fields and
  enum payloads use the same resolver (primitives/unknown names map
  identically, so only genuinely-known names change behavior).
- Regression tests: `tests/type_adversarial_gates.rs` — nine F10
  tests (wrong-type/qualified/return/cross-struct/nested-lit/
  nested-typo/enum-field/enum-field-exhaustiveness/generic-struct
  params) plus `struct_param_dynamic_arg_stays_lenient` (map-lookup
  args stay `Unknown`-lenient by design) and qualified-good-path
  run-to-42 positives.
- Verified live: every repro above now fails `check` with the code in
  the table (`E-TYPE`, `E-UNDEFINED`, `E-MATCH-EXHAUSTIVE`); all 177
  pre-existing tests still pass unmodified, i.e. no previously-accepted
  valid program regressed.
- Residual (triaged): generic-field access (`.value` on `Box<T>`) and
  generic match bindings still erase to `Unknown` at the use site
  (per-literal instantiations are not tracked) — the documented
  dynamic-typing design, unchanged.

### F11 — TRIAGED (known semantic): `&&` / `||` are eager, not short-circuit
- Symptom (found by Session 4 dogfooding, belongs to the checker
  contract): `i < n && s[i] == "x"` with `i == n` is well-typed
  (`Bool && Bool`) but evaluates `s[n]` anyway → `E-RUNTIME string
  index out of bounds`. MIR lowers `And`/`Or` to eager binops over
  pre-computed temporaries (`src/mir.rs:595`, `src/runtime.rs:591`).
- Decision: NOT changed in this phase. Short-circuit lowering is new
  control-flow semantics (observable: RHS side effects such as `print`
  would be skipped), beyond Session 2's HIR scope and this PRD's NG2.
  The failure mode is loud (`E-RUNTIME`, never a silent wrong answer),
  which preserves the project's core promise.
- Workaround (used by `examples/eval.klang`): bounds guards as nested
  `if`s; `&&` over pure comparisons is unaffected.
- Regression tests: `tests/eval_gates.rs` —
  `eager_and_guard_fails_loudly_never_silently` (pins loud, not
  silent) and `nested_if_guard_runs_correctly`.
- Remaining debt: short-circuit `&&`/`||` lowering, or a compile-time
  lint against bounds-guards in `&&` position.

### Diagnostic span inventory (F7 input, pinned)
Real spans today: `E-PARSE` (parser), `E-TASK-CANCEL` + 
`E-SPAWN-OUTSIDE-GROUP` (binding spans), `E-EFFECT-MISMATCH` (caller's
name span — verified `start: 41, end: 45` on the probe). Everything
else HIR/modules-side is `0,0`: `E-TYPE`, `E-UNDEFINED`, `E-ARITY`,
`E-MATCH-*`, `E-DUPLICATE`, `E-LOOP`, `E-RESERVED-FIELD`,
`E-PRIVATE`, `E-AWAIT-OUTSIDE-GROUP`, `E-SPAWN-POSITION`. Pinned by
`diagnostic_span_inventory` (`tests/type_adversarial_gates.rs`) so any
future span threading shows as a deliberate diff.

## Session 4 — multi-feature dogfood (`examples/eval.klang`)

An expression evaluator (the PRD's recommended candidate: a smaller
version of what Stage 2 self-hosting needs), built incrementally with
a `klang check` at each stage — lexer → parser → eval+driver — so any
stage's failure would attribute cleanly. No stage failed to check;
the one runtime failure during construction was F11 above.

- `mod lexer`: `Tok` struct, `scan(src) -> array` (multi-digit ints,
  lowercase idents, `+-*/()`), private `secret` (E-PRIVATE boundary
  marker), `pub` kind accessors (no const items in the language).
- Top level: `enum Expr` (6 variants, recursive payloads), generic
  `enum Res<T> { Ok(v: T), Err(code: i32) }` instantiated with both
  trees and ints in one program, generic `fn expect<T>`.
- `mod parser`: `POut` struct with an `Expr`-typed field naming the
  top-level enum from inside a module (F1 combination), precedence
  climbing (`expr`/`term`/`factor` + `*_rest`), `Res`-valued errors
  (1 = unexpected token, 2 = missing `)`, 3 = trailing garbage),
  cross-module `lexer::Tok` annotations with statically-checked field
  access (F10 combination).
- `mod eval`: `lookup` over parallel name/value arrays (missing map
  keys are runtime failures, so absence is a scanned `Res::Err(5)`),
  `calc`/`apply`/`run` over `Expr` with an exhaustive 6-arm match
  (no wildcard), div-by-zero as `Res::Err(4)`.
- Effects: `lookup throws` (the effect boundary — Klang has no `throw`
  expression, so the leaf is annotation-only per the language's own
  `fetch_a` idiom) propagates through `apply`/`run` argument
  positions to `main throws`, across two module boundaries. The chain
  is load-bearing, not decorative: stripping `throws` from `main`
  fails with `E-EFFECT-MISMATCH` (pinned).
- Verified live (`klang run examples/eval.klang main`): `2 + 3 * 4`
  → 14, `(2 + 3) * 4` → 20, `x * y + 1` → 31, `10 / 2 - 1` → 4,
  `10 / 0` → 2004, `x + zzz` → 2005, `2 +` → 1001, `(2 + 3` → 1002,
  `2 3` → 1003, generic-`expect` path → 42. `fmt --write` reaches a
  fixpoint; formatted output re-checks clean.

## Test-count delta (this phase)

| Suite | Before | After |
|---|---|---|
| `type_adversarial_gates` (new) | 0 | 18 |
| `eval_gates` (new) | 0 | 6 |
| **Total (all suites)** | **177** | **201** |

All 26 suites pass: `cargo test` → `201 passed; 0 failed`. Examples
regression: `examples/{simple,full,lib,stdlib_demo}.klang` still
`check: OK` and `main()` runs to 42 (unchanged); `examples/eval.klang`
(new) checks clean and runs to the ten pinned outputs above.

## Phase 2 exit criteria

Every finding (F9, F10 fixed with regression tests; F11 triaged with
behavior-pinning tests; Session 1/3's F1–F3, F8 unchanged) is either
fixed-with-regression-test or recorded above as a documented
limitation — nothing dropped silently. Phase 2 is closed, clearing the
sequencing gate into Phase 3, which remains blocked on live model
endpoints per the Phase 0 note (unchanged by this phase).

---

# Security audit — compiler as attack surface (code-verified)

Scope: distinct from Phase 2 correctness work (F1–F11 asked "does the
compiler do the right thing"; this asks "can it be made to do
something dangerous"). Threat model: the compiler will accept
AI-generated and possibly adversarial input — hostile `.klang` files,
hostile model responses, hostile endpoints. Method: (1) source
inspection of every trust boundary, (2) live adversarial probes
against the built binary, (3) OSV/RustSec dependency scan (equivalent
of `cargo audit`; building `cargo-audit` from source exceeded the
session's build budget, so the 85-crate lockfile was checked via the
OSV batch API instead), (4) fixes with regression tests in
`tests/security_gates.rs` (13 tests) or explicit triage below.

Suite totals: **214 tests, 0 failed** (`cargo test`, 27 suites; was
201 — delta is the new `security_gates` suite).

## Dependency scan: clean

All 85 third-party crates in `Cargo.lock` (salsa, cranelift-*,
transitives) queried against OSV: **0 known vulnerabilities**. No
action taken.

## Findings (each fixed-with-regression-test or explicitly triaged)

### F12 — PERF (medium): `match` checking was quadratic in arm count — FIXED
- Symptom: 10k-arm match → 5.5s, 20k arms → 21s of `klang check`
  (4x for 2x input). 100k AI-generated arms would take ~35 minutes.
  Everything else probed (10k-variant enums, 5k-field structs,
  5k functions, 2k generic sites, 1MB strings, 100k-element arrays,
  60-file import chains) is linear and sub-second.
- Cause: `covered: Vec` + `contains` per arm, `seen: Vec` +
  `contains` per arm, and a `covered.contains` filter for the missing
  set in `check_match` (`src/hir.rs`).
- Fix: both become `HashSet`; diagnostics and ordering unchanged
  (missing list still sorted).
- Regression test: `large_match_checks_clean_and_dispatches`
  (`tests/security_gates.rs`) — 3000 arms check clean and dispatch
  correctly at runtime.
- Verified live: 20k-arm match now checks in 1.2s (was 21s).

### F13 — BUG (low): model-output import screen missed separator variants — FIXED
- Symptom: `has_unsafe_import` only matched lines starting with
  `import ` (space), but the parser accepts `import\t"..."` and
  `import"..."` identically. A hostile model response using those
  forms passed the screen (the CLI loader's own check still caught
  them at run time — verified live — so this was defense-in-depth,
  not an open hole).
- Cause: two implementations of one rule (screen in `splice.rs`,
  loader in `main.rs`) drifting apart.
- Fix: single shared predicate `repair::is_unsafe_import_path`
  (absolute / `..` / `~` / NUL) used by both; the screen additionally
  accepts tab/whitespace/quoteless separators. The predicate is
  deliberately substring-based (`a..b.klang` is rejected too) —
  identical to the loader, so the two can never disagree.
- Regression tests: `unsafe_import_screen_covers_all_separator_forms`
  plus driver end-to-end `model_response_unsafe_import_fails_the_
  attempt_loudly` (attempt fails with "rejected unsafe import", never
  a splice).
- Verified live: tab/no-space `import "../../evil.klang"` rejected by
  the CLI loader.

### F14 — BUG (medium): curl argv exposed endpoint + API key; no response cap — FIXED
- Symptom (both confirmed by reading `run_curl`): (1) the endpoint URL
  and `Authorization: Bearer <key>` traveled as curl argv entries,
  visible to all local users via `ps` for the whole (up to 600s)
  request window; (2) no `--max-filesize`, so a compromised endpoint
  (or MITM on a local `http://` URL) could OOM the client by
  streaming an unbounded body. Shell injection was NOT present
  (argv entries, no shell — pinned by test).
- Fix (`src/repair/backend.rs`): URL + auth header move into a
  `0600` `--config` temp file (deleted on every exit path);
  endpoint/key values containing quotes, backslashes, or control
  characters are refused fail-closed (they would break config-line
  syntax); `--max-filesize 10MiB` caps responses (curl exit 63
  surfaces as an `Http` error like any transport failure).
- Regression tests: `curl_argv_carries_no_secrets_and_response_is_
  capped` (PATH-shim `curl` captures argv + config content: secrets
  absent from argv, present in config, cap flag present, no temp file
  left behind), `curl_rejects_config_breakout_values_fail_closed`,
  `curl_missing_binary_is_error_never_panic`.
- Residual (triaged): absolute-path temp-dir carve-out in the runtime
  file builtins trusts `TMPDIR`; a planted symlink under the temp dir
  could redirect reads/writes. No privilege gain (planting the link
  already requires the attacker's fs access), so documented, not
  changed. Same for import-time symlinks (canonicalized; cycle-safe).

### F15 — BUG (medium): response parser panicked on `\` + multibyte — FIXED
- Symptom: `read_json_string`'s unrecognized-escape arm pushed the
  byte after `\` as a Latin-1 char and advanced one byte — a
  multibyte byte there left the index mid-character and the next
  slice panicked, crashing the CLI on a crafted endpoint response.
  Audited the surrounding index arithmetic (`extract_content` search
  cursor): all other advances stay on char boundaries; this was the
  only panic path.
- Fix: the catch-all copies a full UTF-8 char and advances by its
  length; truncated `\u`, lone surrogates, and unterminated strings
  remain fail-closed `Parse` errors.
- Regression test: `response_parser_survives_adversarial_escapes`
  (multibyte round-trip, truncated/surrogate/unterminated/huge/empty
  inputs — verdict is value-or-error, never panic).

### F16 — BUG (low): raw control bytes in file-derived errors reached the terminal — FIXED
- Symptom (verified live): `import "<ESC>[2J"` in a hostile file made
  `klang check` print raw ESC bytes (`cannot read ^[[2Jx`) —
  terminal escape injection on the victim's screen. Diagnostic JSON
  was always properly escaped; only the CLI's plain-text prints of
  file-derived paths were exposed.
- Fix: `diagnostics::sanitize_for_terminal` (control chars → visible
  `\u{...}`, everything else untouched), applied to the import
  loader's error and progress prints in `main.rs`. Runtime file
  builtins need no change (their errors only surface as JSON).
- Regression test: `sanitize_for_terminal_escapes_controls_only`.
- Verified live: the repro now prints
  `cannot read \u{1b}[2Jx`.

### F17 — BUG (low): block-comment braces fooled the repair scope scanner — FIXED
- Symptom: `function_spans` tracked strings and `//` comments but not
  `/* */`, so a `{`/`}` inside a block comment unbalanced brace depth
  and mis-sliced function spans for splicing. Blast radius was
  bounded (every splice is fully re-checked; worst case a loud failed
  attempt), but spans are the planner's core input.
- Fix: non-nesting `/* */` tracking mirroring the lexer, with correct
  precedence (string > block comment > line comment).
- Regression tests: `block_comment_braces_do_not_shift_function_spans`
  plus `splicer_never_panics_on_hostile_unicode` (multibyte
  names/strings/comments, unbalanced braces, empty input through
  every text-level repair entry point).

## Confirmed non-findings (probed, no change)

- **Import traversal**: `../../`, absolute, mid-path `..`, `~`, NUL —
  all rejected loudly at load (`E-...` string errors, exit 1) and at
  run (`E-RUNTIME`); `/tmp` writes allowed by design for tests.
  Import cycles terminate (canonicalized `seen` set). Symlink
  planting gains nothing (see F14 residual).
- **Task-group drain**: a failing group with a spinning sibling
  (empty loop AND print loop) terminates with the real error — no
  hang; cooperative cancellation works as documented.
- **Repair log secrecy**: ran `run_repair` with marker endpoint/key
  through `write_log` — neither appears in `.klang-repair-log.json`
  (prompts/responses/diagnostics do, by design). Pinned by
  `repair_log_contains_no_endpoint_or_key_material`.
- **Package manager / FNV**: `verify_lock` has no production callers
  (tests + demo only); nothing trust-relevant depends on FNV being
  cryptographic. The "change-detection, not crypto" note stands as a
  documented limitation, not a vulnerability.
- **`env()`**: reads process env by design (documented); same as any
  language's env access — not a finding.
- **JIT**: integer-only Cranelift path over already-checked programs;
  codegen memory safety is the (OSV-clean) dependency's
  responsibility. Out of scope, noted.
- **Stack exhaustion**: covered by F8; re-probed shapes (deep
  nesting, 2000-operand chains) remain clean.
- Unknown builtins/unknown values stay diagnostics (`E-UNDEFINED`),
  never panics — pinned.

---

# Phase 4, Part A — remove the AI connector, ship MCP (code-verified)

Correction: Klang was built assuming it might need to call a model
directly. The user already runs a harness owning that connection, so
Klang is now the thing the harness calls. Subtraction + one scoped
addition, no language changes.

Suite totals: **218 tests, 0 failed** (`cargo test`, 28 suites).

## Removed (the connector)

- `OpenAiCurlBackend` (entire curl/HTTP client), `chat_url`,
  `chat_request_body`, response parsing (`extract_content`,
  `strip_fences`), `MAX_RESPONSE_BYTES`, config-file secret plumbing.
- Config surface: `endpoint` / `model` / `api_key` / `timeout_secs`
  gone from `RepairCliOverrides`, `RepairFileConfig`, `RepairConfig`;
  `resolve` is infallible (flags + `klang.toml [repair]`, no env);
  retired `klang.toml` keys parse as ignored, so old manifests keep
  working. `KLANG_MODEL_*` env vars no longer read anywhere.
- CLI: `repair --model/--model-name/--api-key/--timeout/--write/
  --verbose` and the `[entry]` arg gone. `klang repair` without
  `--dry-run` exits 2 pointing at `klang mcp`; `--dry-run` (prompt
  plan) and clean-file no-op are unchanged. `bench/run_live.sh` no
  longer drives live repair: it checks the harness signal
  (diagnostic family + planned scope per corpus task) via `klang mcp`
  with zero model calls — verified live, 8/8 tasks report the
  expected families.

## Kept untouched (the mechanism)

`TypedHIR::check`, all diagnostic codes, `Diagnostic::to_json()`,
`scope.rs`, `splice.rs`, `contracts::repair_loop`, the F5
declaration-vs-function boundary, `MockBackend`. The mechanism tests
(`repair_gates.rs` 19, `benchmark_gates.rs`, `enum_gates.rs` repair
case, `driver.rs` unit tests) needed only mechanical edits — dropped
`endpoint:` lines and `resolve_with` -> `resolve` — with zero logic
changes; all pass unmodified in behavior.

## Added: `klang mcp` (stdio JSON-RPC, protocol 2024-11-05)

Four tools, hand-rolled JSON, no new dependencies:
`klang_check` (diagnostics array, empty = clean),
`klang_run` (checks first; 30s execution bound, timeout is an error
not a hung server),
`klang_fmt` (canonical source or the parse diagnostic),
`klang_scope_plan` (source + verbatim `klang_check` diagnostics in,
function names or file-scope + reason out — the same `plan_scope`
the loop uses). Tool payloads embed `to_json()` objects byte-identical
in shape (asserted by test, not by inspection). Requests are
panic-isolated; notifications get silence; JSON depth is capped.

## Verification

- `tests/mcp_gates.rs` (9 tests): scripted fake harness — handshake,
  tool list, diagnostic-shape identity vs direct `to_json()`, scope
  verdict parity with `repair_gates` cases (`pick`, `main`,
  `modules/visibility` -> file + reason), run/fmt paths, all
  JSON-RPC error shapes, and a full oracle loop
  (broken -> check E-ARITY -> scope function(main) -> fix ->
  check clean -> run 3).
- Live: `klang mcp` handshake + all four tools exercised over stdio;
  `bench/run_live.sh` green; `klang repair --dry-run` output and
  exit codes verified (0 clean/dry-run, 2 live-attempt).
- Pre-existing note: `security_gates.rs::curl_argv...` was flaky under
  multi-threaded runs (two tests mutating process `PATH` raced);
  both tests are deleted with the connector they probed, so the flake
  is gone with it — single-threaded run was 13/13 before removal.

## Docs reframed (no model-calling language left)

`README.md` (was a one-line stub) now describes harness-callable
Klang with an MCP quickstart; `SPEC.md` toolchain + `[repair]`
example + benchmark note rewritten; `docs/repair.md` rewritten as the
harness-loop guide (`klang mcp` tools, `--dry-run` inspector, scope
model, bounds — with the "no endpoints to configure and no keys to
leak" correction); `docs/README.md` index and
`docs/limitations.md` benchmark note updated. `scripts/verify_docs.py`:
38/38 samples verified, 0 unverified. Historical planning docs
(`prd.md`, `roadmap.md`, `integration.md`) intentionally untouched —
they record what was believed when.

---

## Heavy testing battery (18 tasks, pre-launch adversarial pass)

Method: every task run against the real binary (`check`/`run`/`repair
--dry-run`), results recorded per task, findings filed below in
F-format. Baseline 218/218 green before the session.

Category 1 — realistic programs: all 5 passed with correct output on
multiple inputs each. (1) key-value config parser into a map with
missing-key defaulting; (2) bank simulator — overdrafts rejected via
`throws`, 3-way `task_group` balances exact; (3) recursive enum tree
with sum/search (20/1/0/0); (4) generic stack over Int/Str/struct in
one program (42/ab/2); (5) two-module tool with `env()` +
`read_file`/`write_file`, missing-file fallback verified.

Category 2 — interaction stress: all 5 passed. (6) `throws` inside a
generic fn propagates through instantiation both ways (a non-`throws`
caller is correctly rejected); (7) nested `task_group`s matching on
awaited enums (328); (8) value-level nested generics run (42);
`Box<T>`-style parameterized annotations are cleanly E-PARSE, matching
SPEC's inference-only promise; (9) three modules with same-named
private fns resolve correctly incl. cross-module calls (600/900);
(10) generic-enum struct field matched + method-chain calls (202).

Category 3 — scale: all 3 passed. (11) 60-field struct + 5000-element
loop-built array exact and fast; (12) recursion works within the
depth cap (fib(20) = 6765) — see F20; (13) 100 sequential 3-task
groups total exactly (30600) in 80ms.

Category 4 — adversarial: all 3 passed, no crashes. (14) variant
shadowing a struct name runs correctly (namespaces separate);
self-import terminates via the seen-set; unprovable generic self-call
fails loudly at runtime via the depth guard; (15) 600-char identifiers
and 100KB strings exact, no truncation; (16) binary garbage is clean
E-PARSE (valid UTF-8) or clean UTF-8 failure, exit 1, never partial
execution.

Category 5 — repair: both passed. (17) three unrelated diagnostics
across three functions escalates to `whole file` scope per
`MAX_FUNCTION_SCOPE`; (18) fix hints render as advisory "Suggested
fixes:" text and the splice/re-check loop is the backstop against a
hint that would introduce a new error.

### F18 — TRIAGED (documented semantic): function arguments are pass-by-value — PINNED
- Symptom: `deposit(alice, 200)` returns 1200 but `alice.balance`
  stays 1000; same for `push` into an array parameter and writes
  into a map parameter. The natural mutation pattern silently does
  not persist. Found via the Task 2 bank simulator.
- Cause: the interpreter binds arguments by value (consistent across
  struct/array/map — verified all three). Not an implementation
  accident in one path; changing it would be a language-semantics
  redesign, not a bug fix.
- Triage: documented in `SPEC.md` §1 (pass-by-value + return-the-
  update guidance) and pinned by
  `args_pass_by_value_no_caller_mutation` (`tests/lang_gates.rs`),
  which asserts all three types keep caller bindings unchanged. No
  silent behavior change; a move to reference/out-param semantics, if
  ever wanted, is roadmap work with its own PRD.
- Verified live: probe programs print 1200/1000, 3/2, 2/1
  (in-callee vs caller-observed).

### F19 — DOC DRIFT: `SPEC.md` claimed block match arms work — FIXED (docs)
- Symptom: `SPEC.md` §2b said "use a block `{ ... }` for
  multi-statement arms", but `parse_match_arm` takes a single
  `parse_expr()` — a `{` after `=>` lexes as a map literal and dies
  with `E-PARSE` ("expected a `\"key\"` or `key` in map literal").
  Found via the Task 3 tree search, which needed early returns.
- Cause: docs written ahead of the grammar (same class as F4).
  Supporting statement blocks as arm bodies would be new
  parser+HIR+MIR surface, i.e. a feature, not a fix.
- Fix: `SPEC.md` §2b + §8 now state single-expression bodies with the
  helper-function workaround (the Task 3 program itself is the proof
  the workaround is ergonomic enough).
- Regression test: `match_block_arm_is_clean_parse_error`
  (`tests/parse_gates.rs`) — block arms in two positions must be
  exactly `E-PARSE`, never a panic, guarding the documented
  limitation against silent drift in either direction.
- Verified live: Task 3 rewritten with `node_has` helper checks
  clean and runs 20/1/0/0.

### F20 — TRIAGED (documented guard): runtime call-depth cap is 64 frames — PINNED
- Symptom: `countdown(600)` and even `countdown(64)` fail at run time
  with `E-RUNTIME "call depth exceeded (possible recursion)"`;
  boundary probed: 63 runs (returns 63), 64 fails. Legitimate
  recursion past ~63 frames is therefore rejected, and the message
  reads as an accusation for correct code. Found via Task 12.
- Cause: deliberate guard in `exec_function` (`src/runtime.rs:288`,
  `depth > 64`) against native stack overflow from the interpreter's
  own recursion. Raising it blindly risks reintroducing F8-class
  aborts, and the failure is loud, never silent or wrong.
- Triage: documented in `SPEC.md` §7b (cap value, boundary, rationale)
  and pinned by `call_depth_limit_is_loud_never_a_crash`
  (`tests/robustness_gates.rs`), which asserts depth 63 returns 63
  and depth 600 fails with exactly `E-RUNTIME`. Any future change to
  the limit (bigger stack budget, explicit depth diagnostics) must
  update this test deliberately.
- Verified live: `fib(20)` = 6765 within the cap; deep chains in
  `check` remain unbounded (front-end deep-stack fix unaffected).

## Usability signals (no finding number — worked, but awkward)

These are launch-relevant frictions this battery surfaced, recorded
here because the project has had no channel for them:
- `push`/`pop` require a variable receiver, so pushing into a struct
  field needs a `let tmp = s.items; push(tmp, v); s.items = tmp`
  dance (Task 4). Loud and documented, but noisy for the most common
  container pattern.
- Single-expression match arms force helper-function extraction for
  any branching arm logic (Task 3). Workable, visibly un-idiomatic
  next to the `if`/`while` statement bodies elsewhere.
- No `array`/`map` nominal types means container-taking functions
  annotate dynamic and lose all checking at the boundary (Task 2
  probes: passing anything where an array is expected checks clean).
  Consistent with the `unknown` design, but worth knowing before
  launch messaging.

---

# Live heavy-testing follow-up — F12–F15 (second series, code-verified)

Scope note: these F-numbers are the live heavy-testing session's own
labels. They intentionally reuse F12–F15, colliding with the earlier
security-audit F12–F15 above; each entry below is marked "(second
series)" to keep the two unambiguous. Baseline going in: 221/221
green; going out: 227/227 green (one regression test each for F12,
F13, F14, three for F15, plus one pinned-test update where F15
deliberately closed a documented gap).

Order of work: F12 → F14 → F13 → F15 (first three small, mechanical,
independent; F15 a parser-level gap needing a design decision first).
Full `cargo test` run after each fix; no later fix regressed an
earlier one.

### F12 (second series) — BUG (medium): semantic diagnostics hardcoded `file` to `input.warden` — FIXED
- Repro: `/tmp/f12f15/f12_arity.klang` —
  `fn add(a: i32, b: i32) -> i32 { return a + b }` +
  `fn main() -> i32 { return add("hi", 1) }`, via
  `klang check /tmp/f12f15/f12_arity.klang`.
- Observed (buggy): every checker/typechecker diagnostic reported
  `"file": "input.warden"` regardless of input path (confirmed across
  E-ARITY, E-TYPE, E-UNDEFINED, E-EFFECT-MISMATCH, E-MATCH-EXHAUSTIVE,
  E-MATCH-DUPLICATE, E-PRIVATE, E-SPAWN-OUTSIDE-GROUP, E-TASK-CANCEL).
  Parser E-PARSE already carried the real path (control:
  `unclosed_brace.klang` → `"file":
  "/tmp/mistakes2/unclosed_brace.klang"`), proving the bug was isolated
  to the checker stage's constructors.
- Cause: `src/hir.rs:24` `pub const FILE = "input.warden"` (pre-rename
  "Project Warden" leftover) used by every semantic helper
  (`type_mismatch`, `duplicate`, `undefined`, `arity_mismatch`,
  `match_*`, `unify_generic`, inline `Diagnostic::error`s) plus
  `src/modules.rs` (`duplicate`/`undefined`/`not_a_value`/`gate`),
  with `TypedHIR::check(program)` taking no file parameter — unlike
  `Parser::new_with_file`, which already threaded the path.
- Fix: `TypedHIR::check` now delegates to new
  `TypedHIR::check_with_file(program, file)`; every HIR helper and
  `check_function`/`check_block`/`check_expr`/`check_match`/etc. takes
  `file: &str` and passes it to its diagnostics (same for
  `modules::resolve` → `resolve_with_file`, whose `Ctx` now carries
  `file`). Callers pass the real label: CLI `check`/`run`/`build`
  passes the entry path (`src/main.rs`), Salsa `check_file` passes the
  Salsa file name (`src/db.rs`), repair `initial_check` passes its
  `file_label` (`src/repair/driver.rs`). `check`/`resolve` remain as
  `input.warden`-default wrappers so all pre-existing callers/tests
  are unchanged.
- Regression test: `semantic_diagnostic_carries_real_file_path`
  (`tests/type_gates.rs`) — E-TYPE and E-ARITY cases via
  `Parser::new_with_file` + `check_with_file` with
  `/tmp/f12_semantic_file_path.klang` (deliberately not the
  placeholder); asserts every diagnostic's `file` equals the real path
  and never `input.warden`.
- Live-verified: `klang check /tmp/f12f15/f12_arity.klang` now reports
  `"primary_span": {"file": "/tmp/f12f15/f12_arity.klang", "start": 0,
  "end": 0}` with `"message": "`add` arg 0: want i32, got str"`.

### F14 (second series) — BUG (low): `call depth exceeded` cause blamed concurrency — FIXED
- Repro: `/tmp/f12f15/f14_depth.klang` — single-threaded
  `countdown(600)` (no `task_group`/`spawn`/`await`), via
  `klang run /tmp/f12f15/f14_depth.klang main`.
- Observed (buggy):
  `{"code": "E-RUNTIME", "message": "call depth exceeded (possible
  recursion)", "cause": "runtime failure during concurrent execution",
  "primary_span": {"file": "runtime", ...}}` — wrong: no concurrency
  involved.
- Cause: shared fallback `runtime_err()` (`src/runtime.rs:142`) stamped
  one concurrency cause on ~30 unrelated `E-RUNTIME` paths (div-by-zero,
  OOB, unknown field, depth guard, …). Only 4 sites are genuinely
  concurrent (task panics, `too many concurrent tasks`).
- Fix (pattern, not just instance): `runtime_err()` cause is now the
  neutral `"runtime execution failed"`; new `concurrent_err()` keeps
  `"runtime failure during concurrent execution"` for the 4
  concurrency sites (`fail_group` panic, spawn-rebind panic, await
  panic, task-limit); new `depth_limit_err()` gives the guard its own
  `"call stack depth limit reached"`. `"file": "runtime"` left as-is:
  same placeholder *pattern* as F12 but for the interpreter stage
  (MIR/runtime carry no file table); noted here as a related-but-
  distinct instance, not blocking this fix.
- Regression test: `call_depth_exceeded_cause_is_not_concurrency`
  (`tests/robustness_gates.rs`) — depth-600 run must be `E-RUNTIME`
  whose `cause` contains neither "concurrent" nor "concurrency" and
  does name the depth/stack limit.
- Live-verified: same repro now reports
  `{"code": "E-RUNTIME", "message": "call depth exceeded (possible
  recursion)", "cause": "call stack depth limit reached",
  "primary_span": {"file": "runtime", "start": 0, "end": 0}}`.

### F13 (second series) — BUG (medium): generic mismatch hid inference — FIXED
- Repro: `/tmp/f12f15/f13_same.klang` —
  `fn same<T>(a: T, b: T) -> T { return a }` +
  `fn main() -> i32 { return same(1, "two") }`, via `klang check`.
  Same prompt format reaches the model via `repair --dry-run`, so the
  wording directly shapes auto-fixes.
- Observed (buggy): `` "`same` arg 1: want i32, got str" `` — reads as
  if arg 1 must literally always be `i32`, hiding that `T` was inferred
  as `i32` from arg 0; invites hardcoding an annotation instead of
  fixing the inconsistent instantiation.
- Cause: `unify_generic` (`src/hir.rs`) stored only `param -> Ty`,
  dropping where the binding came from, and on conflict re-emitted the
  generic `type_mismatch(want bound, got actual)` with no parameter
  context.
- Fix: substitution is now `param -> (Ty, origin)` where origin is
  `"arg {i}"` (calls), the field path (struct literals), or
  `"payload {idx}"` (enum payloads, whose diagnostic now also carries
  the payload index). New `generic_mismatch()` reports
  `{what}: {param} was inferred as {bound} from {origin}, got
  {actual}` with cause `generic type parameter ... already inferred
  ...` (rule still `types/mismatch`, code still `E-TYPE`). Explicit
  `<...>` seeds (F15) record origin `explicit type argument {i}` so
  conflicts against them read correctly too.
- Regression test: `generic_mismatch_names_param_and_inference_site`
  (`tests/generic_gates.rs`) — inconsistent `same(1, "two")` must be
  `E-TYPE` whose message contains `T`, `infer*`, and `arg 0`.
- Live-verified: same repro now reports `"message": "`same` arg 1: T
  was inferred as i32 from arg 0, got str"` with `"cause": "generic
  type parameter `T` was already inferred as `i32` from arg 0"` and
  `"primary_span": {"file": "/tmp/f12f15/f13_same.klang", ...}` (F12
  threading visible here too).

### F15 (second series) — BUG (high): explicit generic `<...>` misparsed — FIXED (design decision + implementation)
- Precise scope (confirmed, not assumed): BROKEN — (1)
  `fn unwrap_or(o: Opt<i32>, default: i32) -> i32` failed to parse
  (`expected ')'` at `<`); (2) `count<T>(n - 1)` / `count<i32>(5)`
  silently misparsed as comparison (`count < T`, then `> (n-1)` as a
  second comparison), yielding 8 cascading nonsense diagnostics
  (`undefined T`, `comparison operands: want i32/f64/str, got bool`,
  …) with no hint of the real ambiguity — worse than a clean failure.
  WORKING (controls, not regressed) — generic declarations
  `fn same<T>(...)`, implicit-inference calls
  `util::identity(42)`, generic enum declarations `enum Opt<T>`.
  Boundary: the parser could not handle `<...>` after an identifier in
  type-annotation or call-site position when it contains a type — the
  classic `<`-as-generic vs `<`-as-less-than ambiguity, previously
  resolved unconditionally as comparison.
- Characterization: two code points, one shared ambiguity. Type
  position (`parse_ty_name`: `Ident(::Ident)*`, no `<...>` support) is
  unambiguous (no comparison operator exists in types) — greedy
  extension suffices. Expression position (`parse_primary` Ident arm:
  only `::` / `(`/`{`-literal/Var, no `<` handling, falling through to
  `parse_comparison`'s `Lt` loop) is genuinely ambiguous and needs
  disambiguation.
- Design decision (writeup, not silent): (a) lookahead/backtracking
  (Rust/C++ style: speculatively parse `<...>` as generic args, fall
  back to comparison) vs (b) unambiguous marker (Rust turbofish
  `::<T>`). Tradeoffs — (a): ergonomic for users/AI-generated code
  (no syntax change, existing `<T>` code starts working), more parser
  complexity (speculation + restore, must not break `a < b`); (b):
  zero ambiguity at the cost of a syntax change + docs/spec updates,
  and it does NOT fix case 1 (type annotations `Opt<i32>` have no
  turbofish escape — a `::<>` marker makes no sense there). Since case
  1 requires (a) regardless, (a) was chosen for BOTH positions for
  uniformity: greedy `< Type (, Type)* >` (recursive for nesting) in
  types; speculative `< Type (, Type)* > (` with full-shape commit in
  expressions (only `<...>(` commits; anything else — including
  `<`-without-`>` or `>`-without-`(` — restores and stays a
  comparison, so `a < b` and `f < 123` are untouched). No new tokens;
  `NodeId` identity unaffected (type parsing allocates no IDs;
  speculation restores `pos` + scope counters like `try_parse_assign`).
- Implementation: `ast::Expr::Call` gains `type_args: Vec<String>`
  (empty = implicit); `parse_ty_name` parses optional
  `<...>` (recursive, non-empty, `Opt<Opt<i32>>`/`m::Box<T>`
  supported); `parse_primary` tries `try_parse_generic_call_args`
  on `<` after a bare name (backtracking, `(`-gated commit) and builds
  `Call{type_args}`; HIR resolves generic annotations by base name
  (`Opt<i32>` → nominal `Opt`, `a::Box<i32>` → `a::Box`) and seeds
  call checking from explicit args (arity-checked; conflicts report via
  the F13 message with `explicit type argument {i}` origin);
  `modules::rewrite_ty`/`validate_type_ref` key on the base and
  preserve the suffix (including `m::S<i32>` → `m::S<i32>`);
  `fmt` renders `f<T>(args)` round-trip; MIR ignores `type_args`
  (runtime is inference-determined, unchanged). Qualified generic
  calls (`m::f<T>()`) remain future work; bare-name coverage closes
  both confirmed-broken cases.
- What "generics working" now means (flagged, not silent): generic
  *declarations* and *implicit-inference calls* were already fine;
  explicit instantiation — the syntax needed for e.g.
  `let x: Opt<i32> = ...` and `count<i32>(5)` — was not, until this
  fix. Docs updated (`docs/reference.md`, `docs/tour.md`: explicit
  `<...>` in annotations and call sites, backtracking rule); the
  `type_adversarial_gates::qualified_generic_annotation_syntax_is_parse_error`
  pin (which documented `a::Box<i32>` as E-PARSE) now asserts the new
  parse-and-check-clean behavior with an F15 comment.
- Regression tests (`tests/generic_gates.rs`):
  `explicit_generic_type_annotation_parses_and_checks`
  (`Opt<i32>` param, match, runs to 42),
  `explicit_generic_call_site_parses_without_comparison_diagnostics`
  (`count<T>`/`count<i32>`, asserts clean check with no
  `comparison operands`/`undefined T`, runs to 5),
  `plain_comparison_still_parses_as_comparison` (`a < b` and `f < 123`
  stay comparisons). Pre-existing generic/enum/implicit tests all still
  pass.
- Live-verified: `/tmp/f12f15/f15_opt.klang` (`Opt<i32>` annotation)
  → `check: OK`, `run main() = 42`;
  `/tmp/f12f15/f15_count.klang` (`count<T>`/`count<i32>`) → `check:
  OK`, `run main() = 5`. Genuine comparisons unaffected (see
  `plain_comparison_*` test and `verify_docs` 38/38).

---

# F-V2-1 — Klang v2 Has No Execution Path (code-verified)

## Finding 1: Test count claims do not match reality (data-integrity issue)

**Actual counts from clean `cargo test` run:**

```
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
...
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Total: 291 tests** (grep -rc "#\[test\]" src/ tests/ --include="*.rs" | awk -F: '{s+=$2} END {print s}' → 291)

**Previous claim:** 334 total (63 v2 gate tests, 271 v1 regression tests)
**Discrepancy:** 43 fewer tests than claimed. Every test that exists passes — this is not a regression or failure, it is a reporting inaccuracy. The prior completion summary reported numbers from memory/estimation rather than command output.

**Going forward:** Any completion report must be generated from command output pasted into the report, not stated from memory. If a claim can be checked with a command, run the command.

## Finding 2: v2 programs cannot be executed — only statically checked (real gap)

**Repro (before fix):**
```
$ klang run /tmp/v2_smoke.v2
file: /tmp/v2_smoke.v2 (485 bytes)
parse: FAIL
{"code":"E-PARSE","message":"expected `fn`","cause":"input does not match the v0.1 grammar"}
```

**But check-v2 works:**
```
$ klang check-v2 /tmp/v2_smoke.v2
check: OK (0 diagnostics)
```

**Root cause:** `src/main.rs` has `"run"` exactly once in the subcommand match. No `run-v2` or `--lang v2` handling in the run path — `run` unconditionally calls `run_file_mode` which only knows v1 grammar. Compare to `check`, which has `split_check_lang` and `run_v2_check_mode`.

**Dead code confirmed:** `echo_lowering.rs`, `tuner.rs`, `flow_capture.rs`, `echo_lifetime.rs` are reachable only from static-analysis/check path. No interpreter/JIT code path has ever executed a `flow` block, an `echo fn`, or a `tune()` call.

## Fix implemented (Phase 1 of v2 execution)

### Files created/modified:

**New files:**
- `src/ast/v2.rs` — v2 program AST (schemas, echo fns, functions with resonance types)
- `src/parser/v2.rs` — full v2 program parser (schemas, echo fns, flows, functions, tune/verify/listen)
- `src/mir/v2_lowering.rs` — v2 AST → MIR lowering (schemas, echoes, flows, functions)

**Modified files:**
- `src/lexer/resonance.rs` — added v2 keywords (schema, fn, let, if, else, print, etc.) and operators
- `src/parser/mod.rs` — exported `v2` module
- `src/ast/mod.rs` — exported `v2` module
- `src/lib.rs` — exported v2 parser and AST
- `src/ai_safety/diagnostics.rs` — added `parse_v2` diagnostic
- `src/runtime/mod.rs` — added v2 builtins (`__echo_create`, `__echo_start`, `__echo_listen`, `__tune_validate`, `__verify_validate`, etc.)
- `src/main.rs` — added `run-v2` subcommand and `run_v2_mode` function

### What works now (verified live):

```bash
$ klang run-v2 /tmp/v2_smoke.v2
parse: OK (1 schemas, 1 echo fns, 0 flows, 1 functions)
mir: OK (2 functions)
  fn fetch_profile (1 params, 8 instrs)
  fn main (0 params, 9 instrs)
print: hello
run main() = 0
```

The v2 smoke test program:
```klang
schema Profile "1" { name: str }
echo fn fetch_profile(name: str) -> !Profile { tune<Profile>({name: name}) }
fn main() -> str {
    let profile = fetch_profile("ada")
    let h = listen(profile)
    print("hello")
    return "ok"
}
```

### Status of the 5 execution pillars (PRD §12):

| Pillar | Status |
|--------|--------|
| 1. Parse + lower v2 through MIR (echo_lowering invoked) | **DONE** — echo fns lower to MIR with echo state machine nodes |
| 2. Execute `flow(...)` with `dep=` bindings at runtime | **PARTIAL** — flow decls parse and lower to MIR functions; runtime execution of flows with dep resolution not yet implemented |
| 3. Execute `echo fn` as background work; `listen(handle)` blocks and returns | **PARTIAL** — echo fns lower to MIR; `__echo_listen` builtin returns dummy Profile struct; true background execution with completion signaling not yet implemented |
| 4. Execute `tune<T>(value)` at runtime with schema validation | **PARTIAL** — `__tune_validate` builtin stubbed; returns value as-is; schema registry wired but validation not yet enforced |
| 5. `print()` inside v2 produces real stdout | **DONE** — verified: `print: hello` |

### V1 regression: **Unaffected**

```bash
$ klang run /tmp/v1_test.klang
parse: OK (2 functions, 0 structs, 0 enums)
check: OK (0 diagnostics)
print: 42
run main() = 42
```

All 291 tests pass (28 suites, 0 failed).

## Commands run for this entry:

```bash
# Test counts
cargo test 2>&1 | grep "test result:"
grep -rc "#\[test\]" src/ tests/ --include="*.rs" | awk -F: '{s+=$2} END {print s}'

# v2 execution
klang run-v2 /tmp/v2_smoke.v2

# v1 regression
klang run /tmp/v1_test.klang
```

## F-V2-1 follow-up — de-stubbed (code-verified, this round)

Correction to the prior round's language: three of five pillars were
stubs, and the prior summary's "fixed both findings" rounded that up.
This follow-up makes pillars 2–4 real. Status table now (no rounding):

| Pillar | Status (this round) |
|---|---|
| 1. Parse + lower v2 through MIR (echo_lowering invoked) | REAL (unchanged) |
| 2. Execute `flow(...)` with `dep=` resolved at runtime | REAL |
| 3. Execute `echo fn` + `listen()` (background work, per-handle results) | REAL |
| 4. Execute `tune<T>()` with schema validation (`E-SCHEMA-INVALID` on bad data) | REAL |
| 5. `print()` produces stdout | REAL (unchanged) |

PHASE: F-V2-1 follow-up (one bounded phase: parser body capture + real
v2 interpreter + `run` routing).
STATUS: pillars 2–4 now real; F-V2-1 resolved on the execution axis.
Remaining v2 work (static checking of resonance/schema boundaries at
check-time, flow `dep=` call-site syntax `f(x, dep=n=v)`, JIT backend)
is future roadmap, not stubbed behavior.

FILES CREATED:
- `src/runtime/v2.rs` — real v2 interpreter: flow dep lookup from the
  caller at call time (missing dep = loud `E-FLOW-MUTABLE-CAPTURE`,
  never silent 0); echo bodies run on spawned threads, `listen` joins
  (blocks) and returns that handle's result; `tune`/`verify` convert
  the value and validate against the declared schema, failures return
  the real `E-SCHEMA-INVALID` diagnostic.
- `tests/v2_run_gates.rs` — 4 permanent regression tests (flow dynamic,
  two echoes distinct, tune bad = `E-SCHEMA-INVALID`, tune good succeeds).

FILES MODIFIED:
- `src/parser/v2.rs` — `P` carries `src` + `echo_bodies`; echo decls
  parse real `V2Block` bodies (were skipped); flow bodies capture real
  source slices (were `String::new()`); added `parse_v2_expr` for flow
  bodies; `Tune`/`Verify` parse as values (fixes `let x = tune<..>(..)`);
  `expect_kind` compares by equality (fixes missing `Lt`/`Gt` kinds);
  `True`/`False` tokens handled.
- `src/ast/v2.rs` — `V2Program.echo_bodies: HashMap<String, V2Block>`
  (additive; `EchoDecl` itself unchanged for the check path).
- `src/mir/v2_lowering.rs` — signature threads `echo_bodies` through
  (MIR listing still invokes `echo_lowering::lower_success`, pillar 1).
- `src/runtime/mod.rs` — `pub mod v2`.
- `src/main.rs` — `run_v2_mode` lowers (pillar-1 listing) then executes
  via `runtime::v2::run_v2_program` with the real entry param;
  plain `klang run <file.v2>` and `klang run --lang v2 <file.v2>` route
  to v2 mode via `split_run_lang` + `.v2` extension (explicit, never
  silent: `.klang` files without the flag keep the v1 path untouched).
  Design decision: `run-v2` remains as an explicit alias; `run` on
  `.v2`/flag is the unified entry point the original ask required.
- `src/lexer/resonance.rs` — (prior round, unchanged this round).

TEST RESULTS (live binary `/tmp/klang-target/debug/klang`, pasted output):

```
=== klang run-v2 /tmp/v2_flow_dynamic.v2 ===
parse: OK (0 schemas, 0 echo fns, 0 flows, 1 functions)
mir: OK (1 functions)
  fn main (0 params, 14 instrs)
print: 30
run main() = 30
```

`/tmp/v2_flow_dynamic.v2` computes `threshold` as `(10+5)+5 = 20`
(no literal 20/30 anywhere near the flow); `classify(10)` returns
`10 + 20 = 30`. A placeholder could not produce 30.

```
=== klang run-v2 /tmp/v2_echo_two.v2 ===
parse: OK (0 schemas, 2 echo fns, 0 flows, 1 functions)
mir: OK (3 functions)
  fn double (1 params, 8 instrs)
  fn add100 (1 params, 8 instrs)
  fn main (0 params, 14 instrs)
print: 42
print: 105
run main() = 147
```

`double(21) = 42`, `add100(5) = 105` — two handles, two genuinely
different `listen()` results. A shared/hardcoded stub fails this.

```
=== klang run-v2 /tmp/v2_tune_bad_data.v2 ===
parse: OK (1 schemas, 0 echo fns, 0 flows, 1 functions)
mir: OK (1 functions)
  fn main (0 params, 8 instrs)
run: FAIL
{
  "code": "E-SCHEMA-INVALID",
  "severity": "error",
  "message": "schema `Profile@1:22e75951d5dbf9e9` rejected `age`: want int, got str",
  "primary_span": {"file": "input.v2", "start": 0, "end": 0},
  "cause": "field `age` has the wrong type",
  ...
  "rule": "schema/validation",
  ...
}
(exit 1)
```

Wrong field type (`age: "not a number"` vs `i32`) fails with the real
`E-SCHEMA-INVALID` naming `age` (want int, got str) — not a panic, not
a silent accept, not a generic error. Good-data tune succeeds
(`tests/v2_run_gates.rs::v2_run_tune_good_data_succeeds` + smoke below).

```
=== klang run /tmp/v2_smoke.v2 (plain run, no -v2 flag) ===
parse: OK (1 schemas, 1 echo fns, 0 flows, 1 functions)
mir: OK (2 functions)
  fn fetch_age (1 params, 8 instrs)
  fn main (0 params, 23 instrs)
print: Struct { name: ada, age: 36 }
print: 56
run main() = 56
```

Plain `run` on a `.v2` file now works (previously `E-PARSE`
"expected `fn`"). The smoke exercises all three pillars at once:
echo real work (`fetch_age(30) = 36` on a thread), tune good
(`Profile{name ada, age 36}` validates), flow with computed dep
(`threshold = 20`, `classify(36) = 56`), plus prints. `run --lang v2`
behaves identically (verified).

Full regression (same commands as required):

```
cargo test 2>&1 | grep "test result:"   # every line ok, 0 failed
grep -rc "#\[test\]" src/ tests/ --include="*.rs" | awk -F: '{s+=$2} END {print s}'
# => 297
```

`cargo test` sum of passed = 297, grep count = 297, 0 failed.
Breakdown: `tests/v2_*` gate files hold 59 tests (unchanged from the
F-V2-1 baseline); `src/parser/v2.rs` holds 2 unit tests; the new
`tests/v2_run_gates.rs` holds 4 execution tests; total v2-related = 65;
v1/infra = 232. Prior round reported 291 because the 2
`src/parser/v2` unit tests were missed by the file-scoped count and
`v2_run_gates` did not exist yet (291 + 2 + 4 = 297). The original
334/63/271 claim remains retracted.

V1 regression spot check (`/tmp/v1_test.klang`, `add(20,22)`):
`print: 42`, `run main() = 42` — same behavior/output/exit as before.

---

# STDLIB-OSIO-1 — Phase 1: File I/O (code-verified)

PRD: KLANG STDLIB PRD — OS Interop, Phase 1 (`file::read/write/append/exists`).

## Pre-work reads (per ground rule 6)

Read in full before writing code: `src/stdlib/mod.rs` (17 lines:
`verify` + `net` only — no file/process/regex surface existed),
`src/stdlib/net.rs` (`MockTransport`/`when()`/`when_fail()` wired into
the echo system via `PendingEcho::fulfill`), `src/stdlib/verify.rs`
(`tune`/`verify` minting Harmonic proofs). Registration pattern
followed: new surface is `pub mod file;` in `src/stdlib/mod.rs`,
no parallel scheme invented.

## Naming decision (PRD allows implementer choice)

PRD sketches `file::read(...)` etc. Klang's `::` call syntax only
resolves through declared `mod` blocks: `file::read("x")` parses as an
`EnumCtor` (`src/parser/mod.rs:1869-1910`) and `modules::rewrite_ctor`
converts `m::f(args)` into a `Call` only when `m` is a program module
(`src/modules.rs:605-623`). A builtin `file::read` with no `mod file`
in the program would therefore fail type checking. Existing
convention is flat builtins (`read_file`, `write_file`, `exists`,
`env` — `src/hir.rs:is_builtin`, `src/runtime/mod.rs:exec_builtin`).
Phase 1 keeps flat names and adds the missing one: `append_file`.
MIR passes `Call.func` through generically (`src/mir/mod.rs:580-596`)
so no lowering change was needed; JIT rejects all builtins as
int-only-unsupported (`src/jit.rs:469`), unchanged.

## Error-code decision

Previously every file failure was generic `E-RUNTIME`
(`runtime_err("read_file({path}) failed: {e}")`). Phase 1 maps
`std::io::ErrorKind` in `stdlib::file::map_io_error`: `NotFound` →
`E-IO-NOT-FOUND`, `PermissionDenied` → `E-IO-PERMISSION`, anything else
→ `E-IO-FAILED` carrying the real OS string as both message fragment
and cause (never a shared generic string — cf. F14). Each constructor
has its own cause/fix/rule (`io/not-found`, `io/permission`,
`io/failure`). Non-zero process exits are NOT errors (decided in the
PRD for Phase 2, not this phase). The pre-existing capability guard
`reject_unsafe_path` (absolute paths outside temp dir, `..` escapes)
stays `E-RUNTIME` deliberately: it is a policy rejection, not an OS
I/O failure, and narrowing it to `E-IO-*` would blur the audit trail
(F14 residual documents the same guard). Paths are taken literally —
no globbing, no `~`/env expansion (PRD security section).

## Files created/changed

- CREATED `src/stdlib/file.rs` — `read`/`write`/`append`/`exists` +
  `map_io_error` + three `E-IO-*` constructors.
- MODIFIED `src/stdlib/mod.rs` — added `pub mod file;`.
- MODIFIED `src/runtime/mod.rs` — `read_file`/`write_file` delegate to
  `stdlib::file`; new `append_file` builtin (create-when-absent,
  returns bytes appended); `is_builtin` extended.
- MODIFIED `src/hir.rs` — `append_file` arity 2 + per-arg `str`
  checking in `check_builtin_call`; `is_builtin` extended.
- CREATED `tests/osio_file_gates.rs` — 9 tests (see below).
- MODIFIED `tests/stdlib_gates.rs` — `stdlib_missing_file_is_runtime_error`
  now uses a temp-dir missing path and asserts `E-IO-NOT-FOUND`. Rationale:
  the old `/no/such/...` absolute path never reached the filesystem —
  it hit the unsafe-path guard (`E-RUNTIME`) — so the test passed for
  the wrong reason. Guard behavior is now pinned explicitly by
  `osio_file_unsafe_absolute_path_stays_runtime_error`.
- MODIFIED `docs/reference.md` + `SPEC.md` §4 — one stale cell each
  (`E-RUNTIME if missing` → `E-IO-NOT-FOUND`; added `append_file` row).
  Full OS-interop docs with examples remain Phase 5; these two cells
  were fixed now to avoid F4/F19-class doc drift.

## Test results (real output, `CARGO_TARGET_DIR=/tmp/klang-target`)

Baseline before work: `SUM passed=297 failed=0`, `GREP sum=297`
(agrees with the F-V2-1 follow-up count).

Targeted (after work):

```
Running tests/osio_file_gates.rs (.../osio_file_gates-a550b31d1db36da5)
running 9 tests
test osio_file_append_arg_types_checked ... ok
test osio_file_append_creates_when_absent ... ok
test osio_file_error_codes_are_specific ... ok
test osio_file_append_twice_both_land ... ok
test osio_file_exists_true_and_false ... ok
test osio_file_read_missing_is_not_found ... ok
test osio_file_unsafe_absolute_path_stays_runtime_error ... ok
test osio_file_read_present ... ok
test osio_file_write_then_read_roundtrip ... ok
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
Running tests/stdlib_gates.rs (.../stdlib_gates-895798afa115c6dc)
running 4 tests
test stdlib_arg_types_checked ... ok
test stdlib_env_reads_process_env ... ok
test stdlib_file_roundtrip ... ok
test stdlib_missing_file_is_runtime_error ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Full regression (after work): `SUM passed=306 failed=0`
(297 baseline + 9 new), `GREP sum=306` — both numbers from the same
session, matching. `scripts/verify_docs.py`: `verified 38 samples`,
`all docs samples verified, 0 UNVERIFIED markers`.

Live program probes (`/tmp/klang-target/debug/klang`, excerpt — real
`klang run` also prints `file:`/`parse:`/MIR lines; only the
`check:`/`print:`/`run main()`/diagnostic lines are shown here):

```
=== klang run /tmp/klang_osio_p1.klang main ===
check: OK (0 diagnostics)
print: hello world!
run main() = 42
(EXIT=0)
```

(write "hello" + append " world" + append "!" then read back —
a placeholder could not produce `hello world!`.)

```
=== klang run /tmp/klang_osio_p1_missing.klang main ===
run: FAIL
{
  "code": "E-IO-NOT-FOUND",
  "message": "read_file(/tmp/klang_osio_p1_missing_xyz.txt) failed: file does not exist",
  "cause": "No such file or directory (os error 2)",
  "rule": "io/not-found",
  ...
}
(EXIT=1)
```

## Phase 1 acceptance (against PRD §8)

- AC1: met — read-present, read-missing (`E-IO-NOT-FOUND`),
  write-then-read roundtrip (byte count 7 for `"abc 123"`), double
  append (`"a"+"b"+"c" == "abc"`), `exists` true/false; all live-run.
- AC5 (file slice): met — `NotFound`/`PermissionDenied`/`StorageFull`
  mapped to three distinct codes with real cause strings asserted in
  `osio_file_error_codes_are_specific` (cause text `denied xyz` /
  `no space abc123` visible in JSON, not generic).
- AC7 (regression): met — 306 ≥ 297, sums match, docs 38/38.
- AC8: this entry.
- AC2/AC3/AC4/AC6: not this phase — no process/regex/env code was
  added or claimed here. `process::run_shell`-style APIs do not exist
  anywhere in the tree (only `std::process` use is none; verified by
  inspection, no shell-out path added by this phase).

## Re-verification (2026-09-25, independent session)

Environment had no `cargo` on PATH, so `rustup` minimal/stable was
installed fresh (`cargo 1.98.1`, `rustc 1.98.1` — same as the prior
toolchain). Baseline was re-measured by stashing Phase 1 tracked
changes and moving the two untracked files aside, then running the
full suite at HEAD: `SUM passed=297 failed=0`, `GREP sum=297`
(44 `test result:` lines). After restoring the stash + untracked
files: `SUM passed=306 failed=0`, `GREP sum=306` (45 `test result:`
lines; `tests/osio_file_gates.rs:9` is the only delta).
`CARGO_TARGET_DIR=/tmp/klang-target cargo test --test osio_file_gates
--test stdlib_gates` → 9 + 4 pass (test order varies run to run
under parallelism; same 13 names as above).
`scripts/verify_docs.py` → `verified 38 samples`,
`all docs samples verified, 0 UNVERIFIED markers`.
Live re-probes via `/tmp/klang-target/debug/klang run` on temp-dir
programs (write `"hello"` + append `" world"` + append `"!"` then
read back) → `print: hello world!`, `run main() = 42`, `EXIT=0`;
missing-file probe → `E-IO-NOT-FOUND` with
`cause: No such file or directory (os error 2)` and
`rule: io/not-found`, `EXIT=1`. `grep -rn "std::process\|run_shell"
src/stdlib/ src/runtime/mod.rs` → no hits (no shell-out path).

---

# STDLIB-OSIO-2 — Phase 2: Process execution (code-verified)

PRD: KLANG STDLIB PRD — OS Interop, Phase 2 (`process::run`).

## Pre-work reads (per ground rule 6)

Re-read in full before writing code: `src/stdlib/mod.rs`,
`src/stdlib/net.rs` (`MockTransport`/`when()`/`when_fail()` wired via
`PendingEcho::fulfill`), `src/stdlib/verify.rs` (`tune`/`verify`
minting Harmonic proofs), plus the Phase 1 surface
`src/stdlib/file.rs` as the registration precedent. Registration
pattern followed: new surface is `pub mod process;` in
`src/stdlib/mod.rs`, no parallel scheme invented.

## Naming decision (PRD allows implementer choice)

PRD sketches `process::run(cmd, args) -> ProcessResult`. Klang's `::`
call syntax only resolves through declared `mod` blocks
(`modules::rewrite_ctor` converts `m::f(args)` into a `Call` only when
`m` is a program module), so a namespaced `process::run(...)` spelling
would parse as an enum constructor and fail type checking — same reason
Phase 1 kept flat file-builtin names. Existing convention is flat
builtins (`read_file`, `write_file`, `append_file`, `exists`, `env` —
`src/hir.rs:is_builtin`, `src/runtime/mod.rs:exec_builtin`). Phase 2
keeps the flat name `run_process(cmd, args)`, returning a map with
`stdout: str`, `stderr: str`, `exit_code: i32` (map indexing `r["stdout"]`
is already idiomatic: `Ty::Map` → `Unknown` element, equality with
`Unknown` allowed — `tests/type_gates.rs:80`, `tests/lang3_gates.rs:81`).
MIR passes `Call.func` through generically so no lowering change was
needed; JIT rejects all builtins as int-only-unsupported
(`src/jit.rs:469` keys on `hir::is_builtin`), unchanged.

## Error-code / exit-code decision (stated explicitly per PRD §4)

A non-zero exit is a NORMAL, catchable result: it populates
`exit_code` in the returned map and does NOT raise. Only a failure to
spawn at all is a diagnostic, mapped in
`stdlib::process::map_spawn_error`: `NotFound`/`PermissionDenied`
(missing binary / not executable) → `E-PROCESS-NOT-FOUND`;
any other spawn `io::Error` → `E-PROCESS-FAILED` carrying the real OS
string as both message fragment and cause (never a shared generic
string — cf. F14). Each constructor has its own cause/fix/rule
(`process/not-found`, `process/spawn`). There is deliberately no
`run_shell`-style API anywhere in the tree: the unsafe thing is not
the easy/default thing (PRD security section). Execution is
`std::process::Command` with `.args(argv)` — never a shell string.

## Files created/changed

- CREATED `src/stdlib/process.rs` — `ProcessOutput`
  (`stdout`/`stderr`/`exit_code`) + `run(cmd, args)` via
  `Command::new(cmd).args(args).output()` + two `E-PROCESS-*`
  constructors + `map_spawn_error`.
- MODIFIED `src/stdlib/mod.rs` — added `pub mod process;`.
- MODIFIED `src/hir.rs` — `run_process` arity 2, `cmd: str` +
  `args: array` checking in `check_builtin_call` (returns `Ty::Map`);
  `is_builtin` extended.
- MODIFIED `src/runtime/mod.rs` — new `run_process` builtin: renders
  cmd + array items via `Value::render` (no join/shell), delegates to
  `stdlib::process::run`, returns
  `Map([stdout, stderr, exit_code])`; `is_builtin` extended.
- CREATED `tests/osio_process_gates.rs` — 7 tests (see below).
- MODIFIED `SPEC.md` §4 + `docs/reference.md` — one row each for
  `run_process` (argv-array, map shape, non-zero-is-result,
  `E-PROCESS-NOT-FOUND`). Full OS-interop docs with examples remain
  Phase 5; these rows were added now to avoid F4/F19-class doc drift.

## Test results (real output, `CARGO_TARGET_DIR=/tmp/klang-target`)

Baseline before work (Phase 1 close): `SUM passed=306 failed=0`
(45 `test result:` lines), `GREP sum=306`.

Targeted (after work):

```
Running tests/osio_process_gates.rs (.../osio_process_gates-b7cb14aa842af276)
running 7 tests
test osio_process_arg_types_checked ... ok
test osio_process_error_codes_are_specific ... ok
test osio_process_argv_array_not_shell ... ok
test osio_process_missing_command_is_not_found ... ok
test osio_process_run_success_captures_stdout ... ok
test osio_process_nonzero_exit_is_result_not_error ... ok
test osio_process_stderr_captured_with_nonzero_exit ... ok
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```

Full regression (after work): `SUM passed=313 failed=0`
(306 baseline + 7 new, 46 `test result:` lines), `GREP sum=313` —
both numbers from the same session, matching.
`scripts/verify_docs.py`: `verified 38 samples`,
`all docs samples verified, 0 UNVERIFIED markers` (SPEC/docs edits add
no new `@run` samples, count unchanged).

Live program probes (`/tmp/klang-target/debug/klang`, excerpt — real
`klang run` also prints `file:`/`parse:`/MIR lines; only the
`check:`/`print:`/`run main()`/diagnostic lines are shown here):

```
=== klang run /tmp/klang_osio_p2_success.klang main ===
check: OK (0 diagnostics)
print: hello process
run main() = 0
(EXIT=0)
```

(`let r = run_process("echo", ["hello process"])` then print
`r["stdout"]` — a placeholder could not produce `hello process`.)

```
=== klang run /tmp/klang_osio_p2_nonzero.klang main ===
check: OK (0 diagnostics)
print: 1
run main() = 1
(EXIT=0)
```

(`false` exits 1 yet the call is `Ok` and the CLI exits 0 — proves the
non-zero-is-result decision is real, not prose.)

```
=== klang run /tmp/klang_osio_p2_argv.klang main ===
check: OK (0 diagnostics)
print: <hello world><a;b|c$d>
run main() = 0
(EXIT=0)
```

(`printf` with `["<%s>", "hello world", "a;b|c$d"]`: the space stays
inside one argv entry and `;`, `|`, `$` stay literal — a shell
string-join would have split/expanded them.)

```
=== klang run /tmp/klang_osio_p2_missing.klang main ===
run: FAIL
{
  "code": "E-PROCESS-NOT-FOUND",
  "message": "run_process(klang-osio-nonexistent-xyz-12345) failed: command not found or not executable",
  "cause": "No such file or directory (os error 2)",
  "rule": "process/not-found",
  ...
}
(EXIT=1)
```

## Phase 2 acceptance (against PRD §8)

- AC2: met — success stdout asserted (`echo` → `hello\n`, exit 0),
  non-zero exit asserted as `Ok` value (`false` → 1, CLI still 0),
  missing command → `E-PROCESS-NOT-FOUND` with real OS cause;
  argv-array fidelity proven by `printf '<%s>'` with
  `"hello world"` + `"a;b|c$d"` rendering exactly
  `<hello world><a;b|c$d>`; all live-run as programs, not just unit
  tests.
- AC5 (process slice): met — `NotFound`/`PermissionDenied` →
  `E-PROCESS-NOT-FOUND`, other spawn errors → `E-PROCESS-FAILED`,
  each with its real cause string asserted in
  `osio_process_error_codes_are_specific` (cause text
  `denied exec abc` / `no space proc987` visible in JSON, not generic;
  codes and causes pairwise distinct).
- AC6: met — no shell-string execution path exists anywhere:
  `grep -rn "run_shell" src/stdlib/ src/runtime/mod.rs src/hir.rs`
  hits only the doc comment in `src/stdlib/process.rs` that names its
  deliberate absence; the only spawn call in the tree is
  `Command::new(cmd).args(args)` in `src/stdlib/process.rs:92`
  (no `sh -c`, no `/bin/sh`, no string join).

---

# STDLIB-OSIO-3 — Phase 3: Regex (code-verified)

PRD: KLANG STDLIB PRD — OS Interop, Phase 3 (`regex::is_match`/`find`).

## Pre-work reads (per ground rule 6)

Re-read in full before writing code: `src/stdlib/mod.rs`,
`src/stdlib/net.rs` (`MockTransport`/`when()`/`when_fail()` wired via
`PendingEcho::fulfill`), `src/stdlib/verify.rs` (`tune`/`verify`
minting Harmonic proofs), plus the Phase 1–2 surfaces
`src/stdlib/file.rs` / `src/stdlib/process.rs` as the registration
precedent. Registration pattern followed: new surface is
`pub mod regex;` in `src/stdlib/mod.rs`, no parallel scheme invented.

## Naming decision (PRD allows implementer choice)

PRD sketches `regex::is_match(pattern, text)` /
`regex::find(pattern, text) -> Option<Match>`. Klang's `::` call syntax
only resolves through declared `mod` blocks, so a namespaced spelling
would parse as an enum constructor and fail type checking — same reason
Phases 1–2 kept flat names. Klang also has no `Option` type, so `find`
returns a map in both cases: `{"matched": 0/1, "match": full-text,
"groups": [indexed 1..n], "named": {name: captured}}`. Flat names are
`regex_is_match(pattern, text) -> bool` and
`regex_find(pattern, text) -> map` (prefix rather than suffix keeps the
two grouped and avoids a bare `find` colliding with user functions).
MIR passes `Call.func` through generically so no lowering change was
needed; JIT rejects all builtins as int-only-unsupported
(`src/jit.rs:469` keys on `hir::is_builtin`), unchanged.

## Error-code decision (stated explicitly)

A non-match is a NORMAL result (`is_match` → false,
`find` → `matched == 0` map), NOT an error — parallel to Phase 2's
non-zero-exit decision. Only a malformed pattern raises
`E-REGEX-INVALID-PATTERN`, carrying the real `regex`-crate parse error
verbatim as both message fragment and cause (never a generic message —
cf. F14). One constructor with its own cause/fix/rule
(`regex/pattern`).

## Files created/changed

- MODIFIED `Cargo.toml`/`Cargo.lock` — added `regex = "1"` (new
  dependency; user authorized installs).
- CREATED `src/stdlib/regex.rs` — `is_match` / `find` (per-call
  `Regex::new`, no cache) + `FindResult`
  (`matched`/`text`/`groups`/`named`) + `map_regex_error`.
- MODIFIED `src/stdlib/mod.rs` — added `pub mod regex;`.
- MODIFIED `src/hir.rs` — `regex_is_match`/`regex_find` arity 2, both
  args `str` checking (`Bool` / `Map` returns); `is_builtin` extended.
- MODIFIED `src/runtime/mod.rs` — `regex_is_match` returns
  `Int(0/1)`; `regex_find` returns
  `Map([matched, match, groups: Array, named: Map])`; `is_builtin`
  extended.
- CREATED `tests/osio_regex_gates.rs` — 8 tests (see below).
- MODIFIED `SPEC.md` §4 + `docs/reference.md` — one row each for the
  two builtins (map shape, no-match-is-result,
  `E-REGEX-INVALID-PATTERN`). Full OS-interop docs with examples
  remain Phase 5; these rows were added now to avoid F4/F19-class doc
  drift.

## Test results (real output, `CARGO_TARGET_DIR=/tmp/klang-target`)

Baseline before work (Phase 2 close): `SUM passed=313 failed=0`
(46 `test result:` lines), `GREP sum=313`.

Targeted (after work):

```
Running tests/osio_regex_gates.rs (.../osio_regex_gates-124ff369454329f1)
running 8 tests
test osio_regex_arg_types_checked ... ok
test osio_regex_error_carries_real_cause ... ok
test osio_regex_find_indexed_groups ... ok
test osio_regex_invalid_pattern_find_errors ... ok
test osio_regex_invalid_pattern_is_match_errors ... ok
test osio_regex_non_match_is_false_not_error ... ok
test osio_regex_simple_match ... ok
test osio_regex_find_named_groups ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
```

Full regression (after work): `SUM passed=321 failed=0`
(313 baseline + 8 new, 47 `test result:` lines), `GREP sum=321` —
both numbers from the same session, matching.
`scripts/verify_docs.py`: `verified 38 samples`,
`all docs samples verified, 0 UNVERIFIED markers` (SPEC/docs edits add
no new `@run` samples, count unchanged).

Live program probes (`/tmp/klang-target/debug/klang`, excerpt — real
`klang run` also prints `file:`/`parse:`/MIR lines; only the
`check:`/`print:`/`run main()`/diagnostic lines are shown here):

```
=== klang run /tmp/klang_osio_p3_match.klang main ===
check: OK (0 diagnostics)
print: user@example
print: user
print: example
print: 1
run main() = 1
(EXIT=0)
```

(`regex_find("(\\w+)@(\\w+)", "user@example")` — full match plus both
indexed groups asserted as printed substrings, not just match/no-match.)

```
=== klang run /tmp/klang_osio_p3_named.klang main ===
check: OK (0 diagnostics)
print: user
print: example
run main() = 1
(EXIT=0)
```

(`(?P<user>\\w+)@(?P<host>\\w+)` — named groups via `m["named"]["user"]`.)

```
=== klang run /tmp/klang_osio_p3_bad.klang main ===
run: FAIL
{
  "code": "E-REGEX-INVALID-PATTERN",
  "message": "regex_is_match(([) failed: invalid regex pattern: regex parse error:\n    ([\n     ^\nerror: unclosed character class",
  "cause": "regex parse error:\n    ([\n     ^\nerror: unclosed character class",
  "rule": "regex/pattern",
  ...
}
(EXIT=1)
```

## Phase 3 acceptance (against PRD §8)

- AC3: met — simple match (`hello` in `say hello world` → 42),
  non-match (false / `matched == 0`, still `Ok`), indexed groups
  (`user`/`example` substrings printed), named groups
  (`user`/`example` via `named` map); all live-run as programs.
- AC5 (regex slice): met — `([` and `(?P<bad>` map to
  `E-REGEX-INVALID-PATTERN` with distinct real causes asserted in
  `osio_regex_error_carries_real_cause` (underlying
  `unclosed character class` text visible in JSON, not generic).
- AC7 (regression): met — 321 ≥ 313, sums match, docs 38/38.
- AC8: this entry.
- AC1/AC2/AC4: not this phase — no file/process/env code was added or
  claimed here.

---

# STDLIB-OSIO-4 — Phase 4: Env vars + integration test (code-verified)

PRD: KLANG STDLIB PRD — OS Interop, Phase 4 (`env::get` + multi-phase
program via `klang run`).

## Env decision (no new code — stated explicitly)

`env(name)` already existed as a flat builtin before this PRD
(`src/hir.rs:is_builtin`, `src/runtime/mod.rs:exec_builtin` returning
`std::env::var(&name).unwrap_or_default()`). It IS the flat spelling
of PRD `env::get(name)` under the same `::`-vs-flat rationale as
Phases 1–3 (a namespaced `env::get(...)` would parse as an enum
constructor). Klang has no `Option` type, so unset is `""`, never an
error — parallel to Phase 3's no-match decision. No new diagnostic
codes (the PRD defines none for env). Phase 4 therefore pins the
behavior with tests rather than new implementation.

## Files created/changed

- CREATED `tests/osio_env_gates.rs` — 3 tests: set var returns value,
  unset var returns `""` (still `Ok`), arity/type checking
  (`E-TYPE`/`E-ARITY`).
- CREATED `examples/osio_integration.klang` — the PRD's
  multi-capability program: writes `/tmp/klang_osio_integration.txt`,
  reads it back, echoes the content through `run_process` argv (a
  dynamic file-content value, not a literal), regex-extracts the
  contact address with capture groups, returns 42.
- No `src/` changes (env implementation untouched since before Phase 1;
  file/process/regex surfaces untouched since STDLIB-OSIO-1/2/3).

## Test results (real output, `CARGO_TARGET_DIR=/tmp/klang-target`)

Baseline before work (Phase 3 close): `SUM passed=321 failed=0`
(47 `test result:` lines), `GREP sum=321`.

Targeted (after work):

```
Running tests/osio_env_gates.rs (.../osio_env_gates-b4bb021fb1d92a14)
running 3 tests
test osio_env_arg_checked ... ok
test osio_env_set_var_returns_value ... ok
test osio_env_unset_var_returns_empty_not_error ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Full regression (after work): `SUM passed=324 failed=0`
(321 baseline + 3 new, 48 `test result:` lines), `GREP sum=324` —
both numbers from the same session, matching.
`scripts/verify_docs.py`: `verified 38 samples`,
`all docs samples verified, 0 UNVERIFIED markers`.

Integration program — real `klang run`, not a unit test
(`/tmp/klang-target/debug/klang`, excerpt — real output also prints
`file:`/`parse:`/MIR lines):

```
=== klang run examples/osio_integration.klang main ===
check: OK (0 diagnostics)
print: contact: ada@example.com
print: contact: ada@example.com
print: ada@example.com
print: ada
print: example.com
run main() = 42
(EXIT=0)
```

(On-disk `/tmp/klang_osio_integration.txt` confirmed to contain
`contact: ada@example.com` — the first print is the file read, the
second is the process stdout echoed through argv, the last three are
the regex full match + both capture groups. A stubbed phase could not
produce this chain: each stage's output is the next stage's input.)

## Phase 4 acceptance (against PRD §8)

- AC4: met — `env` (flat `env::get`) pinned set/unset; one real
  multi-capability `.klang` program combining file + process + regex
  live-run successfully via `klang run` with pasted output above.
- AC7 (regression): met — 324 ≥ 321, sums match, docs 38/38.
- AC8: this entry.
- AC1/AC2/AC3/AC5/AC6: unchanged since STDLIB-OSIO-1/2/3 — no new
  diagnostics or process surface in this phase.

---

# STDLIB-OSIO-5 — Phase 5: Docs (code-verified)

PRD: KLANG STDLIB PRD — OS Interop, Phase 5 (usage examples for all of
the above, scoped to OS-interop only — no claim about any unrelated
v2 acceptance items).

## Convention (checked before writing)

`docs/README.md` index + `scripts/verify_docs.py`: every ` ```klang `
block in `docs/*.md` carries a machine-checked first-line directive
(`// @run prints: … ; return: N`, `// @run-fail E-CODE`,
`// @check-fail E-CODE`, `// @check-ok`) and the script runs each
block against the real binary (this session: `KLANG_BIN` default
`/tmp/klang-target/debug/klang`). No existing docs samples covered
file/process/regex/env builtins, so this phase is a new page rather
than edits to the tour flow.

## Files created/changed

- CREATED `docs/os-interop.md` — 11 verified samples: files
  (write+read, append+exists, missing-file `E-IO-NOT-FOUND`),
  processes (echo stdout, `false` non-zero-is-result, missing-binary
  `E-PROCESS-NOT-FOUND`), regex (indexed groups, named groups,
  malformed-pattern `E-REGEX-INVALID-PATTERN`), env
  (missing-var default), plus a file→process→regex chain mirroring
  `examples/osio_integration.klang`.
- MODIFIED `docs/README.md` — index bullet for the new page.
- No `src/`/`tests/` changes in this phase.

## Test results (real output)

`scripts/verify_docs.py` (after work):

```
verified 49 samples: {'check-ok': 0, 'run': 25, 'check-fail': 19, 'run-fail': 5}
all docs samples verified, 0 UNVERIFIED markers
```

(38 before + 11 new: 8 `@run`, 3 `@run-fail`; verified first try, no
fix-ups. Each file sample uses its own `/tmp/klang_docs_*.txt` path
and overwrites before appending, so re-runs are idempotent.)

Full regression (after work, unchanged code): `SUM passed=324
failed=0` (48 `test result:` lines), `GREP sum=324` — same as the
Phase 4 close, matching.

## Phase 5 acceptance (against PRD §8)

- Scoped doc addition for OS-interop only: met — one new page plus an
  index link; no edits to unrelated guides; no claim about any v2
  acceptance items.
- AC7 (regression): met — 324 = 324, sums match, docs 49/49
  (up from 38/38, all verified).
- AC8: this entry.
- AC7 (regression): met — 313 ≥ 306, sums match, docs 38/38.
- AC8: this entry.
- AC1/AC3/AC4: not this phase — no file/regex/env code was added or
  claimed here (file surface untouched since STDLIB-OSIO-1).

---

# STDLIB-OSIO-6 — file::remove follow-up: `remove_file` (code-verified)

Note on numbering: the task prompt asked to log this as `STDLIB-OSIO-4`,
but `STDLIB-OSIO-4` (env vars + integration) and `STDLIB-OSIO-5` (docs)
already exist above, so this is logged as the next free number,
`STDLIB-OSIO-6`, to avoid a duplicate heading. No other file operations
(rename, copy, directory listing, etc.) were added — single function only.

## Naming decision (checked before writing)

`src/stdlib/file.rs` internal functions are `read`/`write`/`append`/
`exists`; the Klang-level convention is flat builtins `read_file` /
`write_file` / `append_file` / `exists` (`src/hir.rs:is_builtin`,
`src/runtime/mod.rs:exec_builtin`). The new surface follows both: Rust
`stdlib::file::remove` wired as flat Klang builtin `remove_file(path)
-> i32` (returns `1` on success; `Int` so `return remove_file(p)`
typechecks like `write_file`/`append_file`). MIR passes `Call.func`
through generically so no lowering change was needed; JIT rejects all
builtins via `hir::is_builtin`, unchanged.

## Error / guard decision (stated explicitly)

- Same `reject_unsafe_path` sandbox as reads/writes/appends/exists:
  absolute paths outside `std::env::temp_dir()` and any `..` component
  stay `E-RUNTIME` (policy rejection, not I/O failure — same rationale
  as STDLIB-OSIO-1). The delete path calls the guard before touching
  the filesystem, so it cannot bypass the boundary.
- Same `E-IO-*` convention via the existing `map_io_error` helper (no
  new mapper): `NotFound` → `E-IO-NOT-FOUND`, `PermissionDenied` →
  `E-IO-PERMISSION`, else `E-IO-FAILED` with the real OS string.
  Implementation is `std::fs::remove_file` + `map_io_error
  ("remove_file", ...)` (`src/stdlib/file.rs`).

## Files created/changed

- MODIFIED `src/stdlib/file.rs` — added `pub fn remove(path)`.
- MODIFIED `src/stdlib/mod.rs` — doc line now lists `remove` /
  `remove_file`.
- MODIFIED `src/runtime/mod.rs` — `is_builtin` + new `remove_file`
  arm (arity 1, `reject_unsafe_path`, delegates to
  `stdlib::file::remove`, returns `Int(1)`).
- MODIFIED `src/hir.rs` — `remove_file` arity 1, `str` arg check,
  returns `Ty::Int`; `is_builtin` extended.
- MODIFIED `tests/osio_file_gates.rs` — 3 new tests (see below).
- MODIFIED `SPEC.md` §4 + `docs/reference.md` — one row each for
  `remove_file` (no new `@run` samples, to avoid F4/F19-class drift).

## Test results (real output, `CARGO_TARGET_DIR=/tmp/klang-target`,
`cargo 1.98.1`)

Baseline before work (Phase 5 close): `SUM passed=324 failed=0`,
`GREP sum=324`.

Targeted (after work):

```
running 12 tests
test osio_file_remove_existing_then_exists_false ... ok
test osio_file_remove_missing_is_not_found ... ok
test osio_file_remove_unsafe_absolute_path_stays_runtime_error ... ok
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

(`osio_file_gates` was 9 tests, now 12 — the only delta is the 3 new
tests; full list also includes the 9 pre-existing tests passing.)

Full regression (after work): `SUM passed=327 failed=0`
(324 baseline + 3 new), `GREP sum=327` — both numbers from the same
session, matching, delta exactly +3. `scripts/verify_docs.py`:
`verified 49 samples`, `all docs samples verified, 0 UNVERIFIED markers`
(unchanged — SPEC/reference edits add no new samples).

## Acceptance (against the task)

- Remove-then-`exists`-is-false: met —
  `osio_file_remove_existing_then_exists_false` (Klang-level
  `remove_file` + `exists` check in one program returning 42, plus a
  Rust-side `Path::exists` assert so a no-op success cannot pass).
- Remove-missing-is-`E-IO-NOT-FOUND`: met —
  `osio_file_remove_missing_is_not_found` (asserts the real code, not a
  silent no-op or panic).
- Remove-outside-sandbox-rejected-like-reads/writes: met —
  `osio_file_remove_unsafe_absolute_path_stays_runtime_error` (same
  `/no/such/klang-file-xyz-outside-tmp` shape as
  `osio_file_unsafe_absolute_path_stays_runtime_error`, asserts
  `E-RUNTIME`).
- Scope: met — no rename/copy/listing or other file ops added.

## Suggestion (not implemented, per scope)

If a next file op is wanted, `rename_file(src, dst)` (move) is the
natural follow-up from real-script porting (temp-write + atomic
replace); it would need both-path sandbox checks plus a defined
cross-device/error semantic before implementation.

---

# Triple-bug fix: unicode escape, i32 overflow, f64 display (code-verified)

Logging choice: one combined entry with three sub-entries
(BUGS-3-1/2/3). Fresh `BUGS-3-*` prefix — no collision with F12–F17,
F-V2-1, or STDLIB-OSIO-*.

Baseline (before work, same session, `CARGO_TARGET_DIR=/tmp/klang-target`):
`cargo test` → `passed: 327 failed: 0`,
`grep -rc "#\[test\]" src/ tests/` → `327`.
After work: `cargo test` → `passed: 337 failed: 0`,
`grep -rc` → `337` — both numbers matching, delta exactly +10
(`tests/bugs3_gates.rs`, 10 regression tests). Toolchain
`cargo 1.98.1 / rustc 1.98.1`.

### BUGS-3-1 — BUG (high): `\uXXXX` escape not decoded — FIXED
- Repro (`bug1_unicode_escape.klang`):
  ```
  fn main() -> i32 {
      let s = "A\u0041B"
      print(s)
      print(len(s))
      return 0
  }
  ```
- Observed (buggy, live `klang run` before fix):
  ```
  check: OK (0 diagnostics)
  print: A\u0041B
  print: 8
  run main() = 0
  ```
  (MIR showed `t0 = const_str "A\\u0041B"` — lexer passed it through.)
- Cause: string lexing in `src/parser/mod.rs:427-438` (`tokenize`)
  matched only `n/t/"/\` and fell every other escape (including `u`,
  and also missing `r`) into the literal `push('\\') + push(other)`
  pass-through. Same duplication in `src/lexer/resonance.rs:477-488`
  (v2 lexer).
- Fix: same code path/pattern, extended match arms — added `r → \r`
  plus a `u` arm parsing exactly 4 hex digits (`u32::from_str_radix`
  + `char::from_u32`; surrogates/invalid fall back to literal
  pass-through, never panic; byte-index math stays on ASCII
  boundaries). Both lexers fixed identically.
- After (live `klang run`, same file):
  ```
  check: OK (0 diagnostics)
  print: AAB
  print: 3
  run main() = 0
  ```
  (MIR now `t0 = const_str "AAB"`.)
- Extra positions (live): `"\u0041BC"` → `ABC`, `"A\u0041C"` → `AAC`,
  `"AB\u0041"` → `ABA`, `"\u0041\u0042\u0043"` → `ABC`, `len` → `3`.
  Other escapes preserved: `"a\nb\tc\rd\\e\"f"` renders with real
  newline/tab/CR/backslash/quote (verified via `cat -A`).
- Regression tests: `bugs3_unicode_escape_decoded`,
  `bugs3_unicode_escape_positions`, `bugs3_other_escapes_still_work`
  (`tests/bugs3_gates.rs`).

### BUGS-3-2 — BUG (high): runtime arithmetic had no i32 bounds checking — FIXED
- Repro (`bug2_overflow_runtime.klang`):
  ```
  fn main() -> i32 {
      let a = 2147483647
      let b = a + 1
      print(b)
      return 0
  }
  ```
- Observed (buggy, live before fix): `check: OK (0 diagnostics)` then
  `print: 2147483648` — a value outside `i32` (`max 2147483647`).
  Companion (`bug2_overflow_literal.klang`, `let x = 2147483648`)
  correctly fails check with `E-TYPE "integer literal: want i32
  range, got i32"` — checker enforced literals, interpreter enforced
  nothing (all `wrapping_*` on `i64` in `src/runtime/mod.rs:544-588`
  and `src/runtime/v2.rs:531-558`).
- Semantic decision (stated, not silent): option (b) — overflow is a
  runtime error. Rationale: the codebase already treats out-of-range
  integers as loud errors at the return boundary (`run_with_output`
  `E-RUNTIME "integer overflow..."`), the checker's literal rejection
  shows intent to reject (not wrap), and the project's core promise is
  "AI mistakes surface as errors, not silent wrong answers" — wrapping
  `2147483647 + 1` to `-2147483648` would be the silent-wrong-answer
  option. Wrapping also contradicts the existing boundary check (a
  wrapped value would never trip it).
- Fix: new `E-OVERFLOW` / `arithmetic/overflow`
  (`overflow_err(op)` + `to_i32_checked` + `num2_checked` in
  `src/runtime/mod.rs`; mirrored `overflow_err`/`to_i32_checked`/
  `checked_int_op` in `src/runtime/v2.rs`). `add/sub/mul/div/mod/neg`
  convert operands to `i32` (out-of-range operand is itself overflow)
  then use `checked_add/sub/mul/div/rem/neg`; `i32::MIN / -1` correctly
  errors via `checked_div`. String-concat and float paths untouched.
  Final-return check unified to the same `E-OVERFLOW` (was `E-RUNTIME`).
  JIT left unchecked (documented int-only probe backend, not the
  interpreter path in the repro).
- After (live `klang run`, same file):
  ```
  check: OK (0 diagnostics)
  run: FAIL
  {
    "code": "E-OVERFLOW",
    "severity": "error",
    "message": "integer overflow in `add`: result out of i32 range",
    "primary_span": {"file": "runtime", "start": 0, "end": 0},
    "cause": "i32 arithmetic never wraps: out-of-range results are errors",
    "expected": null,
    "found": null,
    "fixes": [{"label": "use smaller operands"}, {"label": "check bounds before operating"}],
    "rule": "arithmetic/overflow",
    "related": []
  }
  ```
- Boundary siblings (live, all `E-OVERFLOW`): `i32::MIN - 1` (`sub`),
  `i32::MAX * 2` (`mul`), `i32::MIN / -1` (`div`). In-range
  `20 + 22 = 42` still runs.
- Regression tests: `bugs3_add/sub/mul/div_overflow_is_error`,
  `bugs3_in_range_arithmetic_still_runs` (`tests/bugs3_gates.rs`).

### BUGS-3-3 — BUG (medium): final summary dropped f64 fraction — FIXED
- Repro (`bug3_return_type_display.klang`):
  ```
  fn main() -> f64 {
      let x = 42.0
      print(x)
      return x
  }
  ```
- Observed (buggy, live before fix):
  ```
  print: 42.0
  run main() = 42
  ```
- Cause: `run_with_output` returns `(i32, Vec<String>)` via
  `v.as_int()`; the CLI (`src/main.rs:622-627`) printed that `i32`,
  truncating every `f64` return. `Value::render` already knew the
  right answer (`42.0`); the summary channel threw the type tag away.
  Same truncation in MCP `tool_run` (`return_value: Int(v)`).
- Fix: new `run_with_output_value` (value-preserving channel;
  `run_with_output` kept as the `i32` wrapper so all 39 existing
  callers/tests are untouched). CLI now uses the value channel and
  prints `v.render()` (out-of-range `Int` still loud `E-OVERFLOW`);
  MCP `tool_run` returns `Float(f)` for floats (out-of-range `Int`
  → `E-OVERFLOW` error object).
- After (live `klang run`, same file):
  ```
  print: 42.0
  run main() = 42.0
  ```
  Non-whole control (`return 3.14`): `print: 3.14` /
  `run main() = 3.14` (live-verified, not a whole-float coincidence).
- Regression tests: `bugs3_f64_return_renders_fraction`,
  `bugs3_f64_nonwhole_return_renders` (`tests/bugs3_gates.rs`,
  via `run_with_output_value` + `render()`).

Full regression (after work): `SUM passed=337 failed=0`
(327 baseline + 10 new), `GREP sum=337` — both numbers from the same
session, matching, delta exactly +10.

---

# STDLIB-TIME-1 — Phase 1: time builtins (code-verified)

Scope: PRD Phase 1 only (`time_sleep`/`time_now`/`time_elapsed`).
`src/stdlib/net.rs` untouched; no new dependencies (std only).

## Pre-work baseline (this session, live)

- `cargo test` (with `CARGO_TARGET_DIR=/tmp/klang-target`, since the
  repo's `target/` sits on a non-executable mount — same workaround
  as prior audits): `passed sum: 337, failed sum: 0` across 49
  `test result:` lines, all `ok`.
- `grep -rc "#\[test\]" src/ tests/`: `total test attrs: 337`.
- Both match (337 = 337).

## Naming (PRD §3 question, answered by reading code)

Flat `time_sleep`/`time_now`/`time_elapsed`, not `time::sleep` —
confirmed against `hir.rs::is_builtin` (only flat names; `::` calls
resolve solely through declared `mod` blocks per
`modules::rewrite_ctor`). Same reason documented in
`src/stdlib/file.rs`/`process.rs`/`regex.rs` headers.

## Clock choice (PRD §3 question, answered by reading code)

`time_now()` = seconds since Unix epoch via `SystemTime` (real
wall-clock timestamp). No monotonic-clock precedent exists in the
codebase to match — the sole `Duration` use is the MCP run timeout
(`src/mcp.rs:510`). `time_elapsed(since)` reads the same clock, so
the pair is self-consistent. Documented in `src/stdlib/time.rs`.

## What was built

- `src/stdlib/time.rs` (new): `now()` / `sleep(f64)` /
  `elapsed(f64)` + `E-TIME-INVALID` constructors with a distinct
  cause per mode (negative carries the value; NaN/infinite/overflow
  each named; non-numeric dynamic values via `not_a_number`). Huge
  finite values go through `Duration::try_from_secs_f64`, so they
  are a loud diagnostic, never a `from_secs_f64` panic.
- `src/stdlib/mod.rs`: `pub mod time;` (net.rs untouched).
- `src/hir.rs`: arities (`time_sleep` 1, `time_now` 0, `time_elapsed`
  1), numeric arg checks (`Int|Float|Unknown`, else `E-TYPE`;
  `time_now` → `Ty::Float`, `time_elapsed` → `Ty::Float`,
  `time_sleep` → `Ty::Int`), `is_builtin` entries. Zero-arg
  `time_now()` flows through the existing `check_builtin_call`
  path with no parser changes.
- `src/runtime/mod.rs`: `is_builtin` entries + `exec_builtin` arms.
  Runtime values are matched (`Int` widens to `f64`), so
  `time_sleep(0)` works; a dynamically-typed non-number (e.g. a
  string out of a map lookup, statically `Unknown`) is
  `E-TIME-INVALID` via `not_a_number`, never a silent sleep-0.
  `time_sleep` returns `Int(1)` on success (matches `remove_file`'s
  success-marker convention).
- `tests/osio_time_gates.rs` (new, 7 tests): ~1s sleep measured in
  `[0.8, 5.0]` (lower bound proves the sleep happened; upper bound
  deliberately generous against loaded-CI flake), zero sleep in
  float+int form, `now()` positive epoch timestamp asserted
  `> 1_000_000_000.0`, monotonic advance, negative → 
  `E-TIME-INVALID`, dynamic string → `E-TIME-INVALID`, Rust-level
  cause-specificity (neg/NaN/inf/huge), arity/type gates.

## Live verification (this session, real output)

`cargo test --test osio_time_gates`:
`test result: ok. 7 passed; 0 failed` (finished in 1.01s).

Built binary probes (`cargo run -q -- run ...`):
`time_now`/`sleep(1.0)`/`elapsed` program printed a real timestamp
and measured duration, then `run main() = 42`:
```
print: 1790488792.8507535
print: 1.0005896091461182
run main() = 42
```
Negative probe (`time_sleep(-0.5)`) produced the real diagnostic:
```
{"code": "E-TIME-INVALID", "message": "time_sleep failed: sleep
duration -0.5 is negative", "cause": "negative duration -0.5 is not
a valid sleep duration", "rule": "time/invalid", ...}
```

## Regression (after Phase 1)

`SUM passed=344 failed=0`, 0 non-ok suites;
`GREP sum=344` — both from the same session, matching, delta
exactly +7 (337 baseline + 7 new).

---

# STDLIB-NET-1 — Phase 2: HTTP client (code-verified)

Scope: PRD Phase 2 only (`http_get`/`http_post`). `src/stdlib/net.rs`
untouched (verified: `git status` shows no modification to it).
Phase 1's time work committed before this phase started; pre-work
baseline re-measured below.

## Pre-work baseline (this session, live)

- `cargo test` (`CARGO_TARGET_DIR=/tmp/klang-target`): `passed sum:
  344, failed sum: 0`, all suites `ok` (re-measured; matches the
  Phase 1 closing number exactly).
- `grep -rc "#\[test\]" src/ tests/`: `total test attrs: 344`.

## Implementation choice: ureq, not a subprocess or hand-rolled socket

The crate needs real HTTPS (the test endpoint is https). Options
considered: (a) shelling out to `curl` — the same workaround class
the PRD rejects for sleep; (b) hand-rolled `TcpStream` HTTP/1.1 —
no TLS, so https endpoints are impossible without a second TLS
crate; (c) `ureq` 3.4.2 (added to `Cargo.toml`, +239 lockfile
lines): synchronous, no async runtime, rustls TLS + gzip in default
features. Picked (c). `cargo add ureq` + `cargo fetch` confirm
crates.io is reachable from this environment.

## What was built

- `src/stdlib/http.rs` (new): `get(url)` / `post(url, body,
  headers)` over one `ureq::Agent` per call (no shared pool — each
  call independent, correct for v1). `http_status_as_error(false)`:
  4xx/5xx populate `status`, never raise (parallel to run_process
  non-zero-exit). Timeouts 10s connect / 60s whole-call backstop.
  Bodies are lossy-UTF-8 (same precedent as `process::run`'s stdout
  capture). Request headers: array of `"Name: Value"` strings
  (run_process argv-array precedent; split on FIRST colon so values
  may contain colons); validated up front with the `http` crate's
  own parsers. Response headers: lowercased-name map, repeats joined
  `", "`, so `resp["headers"]["content-type"]` works like
  `regex_find`'s `m["named"]["user"]`.
- Codes: `E-NET-UNREACHABLE` (exchange failed — refused, DNS,
  timeout, TLS, bad server bytes; real `ureq`/OS string as cause),
  `E-NET-INVALID-URL` (`BadUri` AND `Http` — see mapping bug below),
  plus deliberate addition `E-NET-INVALID-HEADER` for malformed
  header entries (own code, not folded into URL/unreachable;
  documented in `http.rs` with rationale).
- `src/stdlib/mod.rs`: `pub mod http;`. `src/hir.rs`: arities
  (`http_get` 1, `http_post` 3), Str/Array arg checks, `is_builtin`
  entries. `src/runtime/mod.rs`: `is_builtin` entries,
  `exec_builtin` arms → `resp["status"/"body"/"headers"]` map via
  `http_response_map`. Non-array `headers` arg is `E-RUNTIME`
  (mirrors `run_process`'s non-array message verbatim in style).
- `tests/osio_http_gates.rs` (new, 10 tests).

## Mapping bug found by live verification (not by reasoning)

First run: 8/10 passed; `"not a url at all"` mapped to
`E-NET-INVALID-HEADER` (`ureq::Error::Http("http: invalid uri
character")`), because I had assumed URI problems always surface as
`BadUri`. Wrong — spaces surface as `Http`. Fixed: `Http` maps to
`E-NET-INVALID-URL` (sound: all request headers pass `parse_header`
first and `http_get` sets none, so no header-caused `Http` can reach
`ureq`; documented in `map_error`). Empty-string URL verified as
`Http("http: empty string")` → also `E-NET-INVALID-URL`. The tests
pin these real strings.

## Endpoint choice (why httpbin.org, what was excluded)

- Probed live: `https://httpbin.org/get` → 200 in 0.30s; POST
  `/post` echoes body in `data` + headers back (verified with
  curl); `https://postman-echo.com/get` → 200 in 0.19s;
  `https://httpbingo.com/get` → TIMEOUT after 15s (000) —
  EXCLUDED, would be a flaky CI dependency.
- Picked https://httpbin.org (the PRD's own example class):
  operating for 10+ years, the canonical echo service, and the
  service ureq's own docs use in examples. Assertions are on real
  content: GET asserts 200 + body contains `httpbin.org`; POST
  asserts 200 + body contains the sent string
  (`klang-post-probe-456`) + the echoed custom header value
  (`header-val-789`).
- Hermetic tier needs no internet at all: std-`TcpListener`
  loopback server (ephemeral port, serves exactly N requests, 20s
  deadline so client failures fail instead of hanging) pins GET
  status/body/headers, POST body+header round-trip (incl.
  colon-in-value `a:b:c`), 404-as-result, and refused-connection
  (bind-drop-guaranteed ECONNREFUSED) → `E-NET-UNREACHABLE` with
  real `Connection refused (os error 111)`.

## External-network flag (explicit, per PRD)

The two `osio_http_public_*` tests need outbound internet wherever
`cargo test` runs. GitHub-hosted runners have unrestricted outbound
access by default — no egress allowlist on standard runners (docs
note only crypto-mining/malicious hosts blocked via /etc/hosts and
inbound ICMP blocked); the repo's own `release.yml` already depends
on this (`actions/checkout`, toolchain download, `cargo install
cross --git https://...` on ubuntu-latest). BUT: this repo has no
test-running CI job (`release.yml` builds binaries on tags only),
so Phase 2 changes no existing CI behavior — the new dependency
falls on dev machines and any future test workflow, stated here.

## Live verification (this session, real output)

`cargo test --test osio_http_gates`: `10 passed; 0 failed`.
Public pair serially (`--test-threads=1`): `finished in 0.30s`
(real round trips, not mocks).

Built binary (`klang run`): GET probe printed real values, `run
main() = 42`:
```
print: 200
print: application/json
run main() = 42
```
Refused probe produced the real diagnostic:
```
{"code": "E-NET-UNREACHABLE", "message":
"http_get(http://127.0.0.1:9/) failed: io: Connection refused (os
error 111)", "cause": "io: Connection refused (os error 111)",
"rule": "net/unreachable", ...}
```

## Regression (after Phase 2)

`SUM passed=354 failed=0`, 0 non-ok suites;
`GREP sum=354` — both from the same session, matching, delta
exactly +10 (344 baseline + 10 new).

---

# STDLIB-NET-2 — Phase 3: time+HTTP integration program (code-verified)

Scope: PRD Phase 3 only — one real multi-capability `.klang`
program. No `src/`/`tests/` changes (checked `git status`: only the
new example file plus this entry).

## Files created

- CREATED `examples/time_http_integration.klang` — follows the
  `examples/osio_integration.klang` header convention (phase label,
  run command, expected output, 42-on-success). Captures
  `time_now()`, issues a real `http_get("https://httpbin.org/get")`,
  measures with `time_elapsed()`, prints status + seconds, returns
  42 only when status is 200, the body names the endpoint, and the
  measurement is non-negative. NEEDS OUTBOUND INTERNET (same
  httpbin dependency as the `osio_http_public_*` gates, stated in
  the file header).

## Live verification (this session, real output)

`klang check`: `check: OK (0 diagnostics)`.

`klang run examples/time_http_integration.klang main` (excerpt —
real output also prints MIR lines, same as prior probes):

```
print: 200
print: 0.1332387924194336
print: body ok
run main() = 42
(EXIT=0)
```

The middle print is the measured request duration in seconds —
both capabilities genuinely combined (a stubbed clock or a mocked
client could not produce a real 200 + a real sub-second
measurement + a real body-content match in one run).

## Regression (after Phase 3)

`SUM passed=354 failed=0`, 0 non-ok suites;
`GREP sum=354` — both from the same session, matching, delta +0
(354 baseline, no new tests this phase — the integration program is
live-run only, same as `osio_integration.klang`).

---

# STDLIB-NET-3 — Phase 4: docs (code-verified)

Scope: PRD Phase 4 only — usage examples where the OSIO stdlib is
documented. No `src/`/`tests/`/`examples/` changes (checked `git
status`: only `docs/*` plus this entry).

## Convention (checked before writing)

`docs/README.md` index + `scripts/verify_docs.py`: every
````klang` block carries a machine-checked first-line directive
(`// @run prints: … ; return: N` matches prints exactly and parses
an integer return, `// @run-fail E-CODE`, `// @check-fail E-CODE`,
`// @check-ok`) and the script runs each block against the real
binary (`/tmp/klang-target/debug/klang`). `docs/reference.md` §5
holds the per-builtin arity/rules table (no separate diagnostics
table exists — nothing else to extend).

## Files created/changed

- MODIFIED `docs/os-interop.md` — the OSIO page (retitled to cover
  time + HTTP): new `## Time` (0.5s sleep measured against a 0.4s
  lower bound — proves the sleep with no flaky upper bound; zero
  sleep + positive `now()`; negative → `E-TIME-INVALID`) and new
  `## HTTP` (httpbin GET with real status/body assertions, httpbin
  POST with echoed body/header assertions, `E-NET-INVALID-URL` /
  `E-NET-INVALID-HEADER` / `E-NET-UNREACHABLE` run-fail samples —
  all three hermetic, needing no internet — plus the timeout values
  and the plainly-stated trust model). The existing "Putting it
  together" chain is untouched (it mirrors
  `examples/osio_integration.klang`).
- MODIFIED `docs/reference.md` — two builtin-table rows
  (`time_sleep`/`time_now`/`time_elapsed`, `http_get`/`http_post`
  with arities, return shapes, and all four new codes).
- MODIFIED `docs/README.md` — index bullet now lists time + HTTP.
- `docs/limitations.md` untouched (nothing there is contradicted:
  the package-manager "no network fetching" note is still true —
  the new client is stdlib HTTP, not package fetching).

## Test results (real output)

`scripts/verify_docs.py` (after work, first try, no fix-ups):

```
verified 57 samples: {'check-ok': 0, 'run': 29, 'check-fail': 19, 'run-fail': 9}
all docs samples verified, 0 UNVERIFIED markers
```

(49 before + 8 new: 4 `@run`, 4 `@run-fail`. The two httpbin `@run`
samples need outbound internet at docs-verification time — same
dependency as the `osio_http_public_*` gates, already flagged in
STDLIB-NET-1; every other new sample is hermetic.)

## Regression (after Phase 4)

`SUM passed=354 failed=0`, 0 non-ok suites;
`GREP sum=354` — both from the same session, matching, delta +0
(354 baseline, docs-only phase).

---

# HEAVY-TEST — heavy real-world testing pass (code-verified, no fixes)

Scope: the prompt's five areas plus one multi-file tool, each as real
`.klang`/`.v2` programs run against `/tmp/klang-target/debug/klang`
(`cargo 1.98.1`, `klang 0.1.0`). Baseline `SUM passed=354 failed=0`,
`GREP sum=354` re-measured at session start; no `src/` changes in this
pass (findings documented, not fixed, per the prompt). Full programs
live in `/tmp/heavy/` (names below); the one new bug's minimal repro is
checked in as `heavy_print_loss.klang` (repo root, `bugN`-style).

## 1. Core language stress — all pass

- `core1_recursion_generics_match.klang`: 4-variant enum exhaustive
  match + recursive `sum_shapes` over an enum array + `fib(15)` +
  generic `wrap`/`describe_wrap` incl. nested `wrap(wrap(21))`.
  `check: OK`; run prints `80 / 610 / 42 / -5`, `run main() = 135`
  (= 30+20+0+30 areas; fib(15)=610; 40+2; Err(5)->-5; 80+fib(10)=135).
  All exact.
- `core2_shadow_coerce_returns.klang`: re-`let` shadowing across types
  (i32 -> str -> f64), int/float coercion on all three return paths,
  int+str concat. `check: OK`; prints `5.5 / 1.5 / -5.5` plus every
  shadowing branch; checker and runtime agree everywhere.
- `core3_struct_enum_nested.klang`: `Profile` with `Status` enum field
  + `Team` nesting `Profile`; `t.lead.status` access through
  `birthday`/`promote`/`team_report` call chains. `check: OK`; all
  prints exact (`90 / -4 / -999`, team totals `44` and `45`,
  `run main() = 86`).

## 2. OSIO stdlib in combination — passes, with one structural limit

- `osio_chain.klang`: write file -> read -> `regex_find` capture ->
  `run_process("grep", ...)` on the same path -> second `regex_find`
  on the process stdout -> `write_file` report -> read back. `check:
  OK`, `run main() = 42`; on-disk report confirmed
  (`first=ada@example.com`, `grep_found=ada@example.com`).
- Error paths: `osio_all_errors_one_run.klang` (all four triggers in
  one program) stops at the FIRST failure with `E-IO-NOT-FOUND` —
  Klang has no try/catch, so one run can never exhibit all four
  diagnostics. Each path run separately is specific and correct:
  missing file -> `E-IO-NOT-FOUND` / `io/not-found` (os error 2);
  `regex_is_match("([", ...)` -> `E-REGEX-INVALID-PATTERN` /
  `regex/pattern` (real `unclosed character class` cause);
  nonexistent command -> `E-PROCESS-NOT-FOUND` / `process/not-found`
  (os error 2); absolute path outside temp dir -> `E-RUNTIME` /
  `runtime/execution` (`unsafe absolute path` message). No shared
  generic code. Minor echo of F14 (second series): the sandbox
  rejection's message is specific but its `cause` is still the generic
  `"runtime execution failed"`.

## 3. v2 combined — runtime pillars still real, one static-check gap

- `v2_combo_good.v2` (schema + valid `tune` + computed-`dep` flow +
  two echoes used together): `check-v2: OK`;
  `klang run-v2` AND plain `klang run` both print
  `Struct { name: ada, age: 36 } / 36 / 105 / 56`,
  `run main() = 141` (`fetch_age = 36`, `fetch_bonus(5) = 105`,
  `classify(36) = 36+20 = 56` with `threshold = (10+5)+5` computed,
  never a literal). No stub could produce these.
- `v2_combo_bad.v2` (`age: "not a number"`): `check-v2: OK (0
  diagnostics)` — the static check does NOT validate tune literals —
  but `run-v2` fails loudly with `E-SCHEMA-INVALID` naming `age`
  (`want int, got str`). See HEAVY-TEST-2. Valid and invalid tune
  cannot coexist in one run (first failure aborts, same structural
  limit as §2).

## 4. Time + networking — passes (Phase 2 confirmed landed)

- `time_net_success_only.klang`: timed `http_get` to httpbin.org
  prints `200 / 0.7874...s / body ok`, `run main() = 42`. The measured
  0.79s matches `curl`'s 0.78s for the same endpoint this session —
  a real clock around a real request.
- Invalid URL (`"not a url at all"`) -> `E-NET-INVALID-URL`
  (`http: invalid uri character`, rule `net/invalid-url`);
  `http://127.0.0.1:9/` -> `E-NET-UNREACHABLE`
  (`Connection refused (os error 111)`). Distinct codes/causes/rules.
  Combined success-then-bad program verified the abort limit (§2) and
  additionally exposed HEAVY-TEST-1 (prints lost on failure).

## 5. Edge cases and adversarial input — all hold, two confirmations

- Malformed: unclosed brace -> `E-PARSE "expected \`}\`"`; binary
  garbage -> `E-PARSE "invalid token"`; truncated signature ->
  `E-PARSE "expected parameter name"`. All exit 1, all carry the real
  file path (F12 second-series still holding), no crash.
- `edge_multi_error.klang` (arity + undefined + type errors):
  `check: FAIL (3 diagnostics)` — `E-ARITY`, `E-UNDEFINED`, `E-TYPE`
  all in one pass (multi-error reporting still holding; spans still
  `0,0` per the F7 debt, but `file` is now real per F12).
- Cross-feature (never jointly tested before):
  - `x_sleep_in_spawn.klang` (`time_sleep` inside `spawn`ed tasks):
    works, `print: 42`, `run main() = 42`.
  - `x_runproc_in_flow.v2` / `x_runproc_v2_only.v2`: v2 rejects
    `run_process("echo", ["hi"])` with `E-PARSE-V2 "expected
    expression"` at the `[` (bytes 51-52). See HEAVY-TEST-3.
  - `x_qualified_generic_call.klang` (`m::count<i32>(5)`): still
    misparses as comparisons with cascading `E-ARITY`/`E-UNDEFINED`/
    `E-TYPE` (F15's documented "qualified generic calls remain future
    work" — confirmed unchanged, loud, no crash). See HEAVY-TEST-4.
  - Depth boundary re-probed: `countdown(63)` -> `63` clean;
    `countdown(64)` -> `E-RUNTIME "call depth exceeded"` with the
    fixed cause `"call stack depth limit reached"` (F14 + F20 both
    holding exactly).

## 6. Multi-file tool: `logscan` (`/tmp/heavy/tool/`, 3 files) — works

`logline.klang` (`mod logline`: `Entry` struct, regex `classify`,
`level_name`) + `stats.klang` (`mod stats`: `pct`, `report_line`) +
`logscan.klang` (`import` both; `scan()` reads a log, classifies each
line, writes a report file, cross-checks ERROR count via
`run_process("grep", ["-c", ...])` + `int()` on the stdout).
`check: OK`; on the 5-line sample prints
`total=5 / ERROR: 2 (40.0%) / WARN: 1 (20.0%) / INFO: 2 (40.0%) /
OTHER: 0 (0.0%)`, `grep_errors=2`, `run main() = 42`. Re-run against a
different 4-line log gives different correct counts (`0/2/0/2`,
return 42); empty log gives `total=0`, return 2 by design (and
exercises non-zero-exit-is-result: `grep -c` exits 1 on empty input
yet parses to `0` normally). `klang fmt` round-trips the entry file.
Frictions met while building (no new numbers — all previously
documented or logged below): `regex_find` has no find-all (first
match only — the grep cross-check covers counting instead);
single-expression match arms (used `if`/`else` chains, F19);
`print` of a multi-line string renders embedded newlines.

### HEAVY-TEST-1 — BUG (medium): `print` output before a runtime failure is silently discarded — DOCUMENTED (not fixed)

- Repro (`heavy_print_loss.klang`, repo root — smallest form):
  `print("before failure")` then a failing `read_file`. Live
  `klang run` prints NO `print:` line at all — only `run: FAIL` +
  the `E-IO-NOT-FOUND` JSON (exit 1). A succeeding run of the same
  first statement prints normally, so the output existed and was
  dropped, not never-produced. Also observed with
  `time_net_success_then_badurl.klang` (three success prints +
  `good=42` vanished) and `x_depth_boundary.klang` (`countdown(63)`
  print vanished when `countdown(64)` failed later).
- Cause (read, not fixed): `run_with_output_value`
  (`src/runtime/mod.rs:287`) propagates the first `Err` with `?`,
  discarding `ctx.output`; the CLI `Err` arm (`src/main.rs:650-654`)
  prints only the diagnostic. Same shape in MCP `tool_run`
  (value-or-error channel, not inspected this pass beyond the CLI).
- Why it matters: every debugging workflow prints-then-fails; losing
  the prints hides how far the program got (here: hid a good 0.79s
  HTTP timing and a good depth-63 result). Failure diagnostics stay
  complete — this is output loss, not misdiagnosis.

  **Fixed (this round, code-verified).** Root-cause citation above
  confirmed by reading: the `?` at `src/runtime/mod.rs:287`, the CLI
  `Err` arm, plus two more instances of the same pattern found by
  checking (not assuming): MCP `tool_run`'s run-failure object
  (`src/mcp.rs`, no `stdout` key on failure) and the v2 mirror
  (`exec_block(...)?` in `src/runtime/v2.rs` dropping
  `global.output`, `run_v2_mode`'s `Err` arm printing only the
  diagnostic). The `check` path has no equivalent pattern: check
  failures print every diagnostic (`src/main.rs` check arm), and
  parse failure precedes checking by construction, so there is no
  partial output to lose there. JIT noted as suggestion below, not
  changed.

  - Fix: new `run_with_output_value_partial` /
    `run_v2_program_partial` return `(Result<.., Diagnostic>,
    Vec<String>)` — output snapshot survives failure; the original
    functions delegate with byte-identical success/failure behavior
    for all existing callers. CLI `run` / `run-v2` `Err` arms print
    accumulated `print:` lines before `run: FAIL`; MCP run-failure
    and overflow objects gain a `stdout` array (additive keys — the
    `mcp_gates` shape assertions use `.get()`, unaffected).
  - Before (`heavy_print_loss.klang`, original code via stash +
    rebuild, live `klang run`, exit 1):
    ```
    run: FAIL
      "code": "E-IO-NOT-FOUND",
    ```
    (no `print:` line — the bug).
  - After (same repro, fixed code, live `klang run`, exit 1):
    ```
    print: before failure
    run: FAIL
      "code": "E-IO-NOT-FOUND",
    ```
  - Regression tests (`tests/print_loss_gates.rs`, 8 tests):
    single-print + diagnostic via the partial runner; multi-print
    order (`first/42/third`); success-path delegate unchanged;
    arity-failure carries empty output; CLI-level repro through
    `CARGO_BIN_EXE_klang` asserting `print:` precedes `run: FAIL`
    with exit 1 (both the checked-in repro and a temp multi-print
    file); v2 partial runner (`E-SCHEMA-INVALID` + surviving print);
    v2 CLI-level print-before-FAIL.
  - Full regression: baseline `SUM passed=354 failed=0` / `GREP
    sum=354` (measured before the change); after:
    `SUM passed=362 failed=0` / `GREP sum=362` (delta exactly +8),
    `scripts/verify_docs.py` 57/57 (no doc `run-fail` sample has a
    completed print before its failure, so none changes shape).
  - Suggestion (not fixed, per scope): the JIT `run: JIT-FAIL` arm
    (`src/main.rs`) has the same drop shape over `jit_output()` —
    but JIT is the documented int-only probe backend, so it is
    recorded here, not changed. HEAVY-TEST-2/-3 untouched.

### HEAVY-TEST-2 — TRIAGED (static/runtime split): `check-v2` accepts invalid `tune` literals that `run-v2` rejects — DOCUMENTED

- `v2_combo_bad.v2`: `check-v2` reports `OK (0 diagnostics)` while
  `run-v2` reports `E-SCHEMA-INVALID` naming `age`. The v1 analogy
  would be `P { x: 1 }` with a missing field — which F9 made a
  check-time error. For v2 the boundary check lives at runtime
  (`runtime::v2`, `SchemaRegistry::check_boundary` in the tuner
  gates), and the CLI check path never evaluates the literal.
- Triage: logged as a split, not fixed — deciding whether `check-v2`
  should const-evaluate `tune` literals is a design call (dynamic
  values in that position can never be checked statically; literals
  equally could be). No silent wrong answer either way: `run` is
  loud, `check` is merely lenient. Any future const-evaluation must
  handle non-literal tune arguments explicitly.

### HEAVY-TEST-3 — SCOPING RESULT (useful negative): v2 cannot express OSIO calls — two independent levels — DOCUMENTED

- Parse level: `run_process("echo", ["hi"])` in `.v2` fails
  `E-PARSE-V2` at the array-literal `[` (bytes 51-52); the v2 grammar
  (`src/parser/v2.rs`) has no array-literal syntax, so argv-style
  builtins are inexpressible before builtins even matter.
- Runtime level: `src/runtime/v2.rs` contains zero references to
  `run_process`/`time_sleep`/`http_get`/`read_file`/`regex_*` (grep
  verified) — even a scalar-arg builtin would have no executor.
- Related placeholder (same pattern as F12, distinct instance):
  v2 diagnostics carry `"file": "input.v2"` instead of the real path
  (seen on both `E-PARSE-V2` and `E-SCHEMA-INVALID`). Recorded here,
  not fixed, matching how F14 handled the `"file": "runtime"`
  sibling.
- Verdict for the prompt's cross-feature question: unsupported, with
  exact diagnostics at the first boundary — a fine result, no
  workaround attempted.

### HEAVY-TEST-4 — CONFIRMATION (no change): qualified explicit-generic calls still parse as comparisons — DOCUMENTED

- `m::count<i32>(5)` yields `E-ARITY` (0 args), `E-UNDEFINED`
  (`i32`), `E-TYPE` (comparison on bool) — the F15 backtracking
  covers bare names only, exactly as F15 stated ("Qualified generic
  calls (`m::f<T>()`) remain future work"). Loud, no crash, no
  miscompilation. The cascade gives no hint of the real ambiguity
  (same wart F15 documented for the bare-name case before its fix);
  extending backtracking across `::` is the natural follow-up, left
  for the fixing round.

## Regression (this pass): `SUM passed=354 failed=0`, `GREP sum=354` — unchanged (no `src/` edits, docs-only + 1 repro file).

---

# QUIET-1 — quiet output mode for `klang run` (code-verified)

PRD Phase A. Baseline at session start: `SUM passed=362 failed=0`,
`GREP sum=362` (354 + the 8 `print_loss_gates` tests from the
HEAVY-TEST-1 fix round).

## Decision: option (a) — `klang run --quiet` / `-q` flag

Option (b) (quiet by default) was rejected on evidence, not taste:
`scripts/verify_docs.py` parses the verbose `run` shape (it greps
`check: OK` in stdout and extracts `print: ` lines), so flipping the
default would break the docs harness and every AUDIT'd verbose probe
in this project's history. A repo-wide grep for tests asserting on
CLI stdout found none (all suites drive the library directly except
`print_loss_gates`/`quiet_gates`, which use substring matching) — but
the docs script alone is the "existing consumer relying on the
default" the PRD names. Additive flag, zero default-output change.

## Quiet contract (pinned by tests, stated explicitly)

- Suppressed: `file:`, `parse: OK` + fn list, `check: OK`, the `mir:`
  dump. `check`, `build`, `run-v2`/`check-v2` keep verbose behavior
  (flag is `run`-gated; `check --quiet` errors exactly as
  `check --backend-jit` always has — unknown-flag-as-path, pre-existing
  class, out of scope).
- `print` lines go raw (no `print: ` prefix) — the python-like case.
  The minimal result line is kept: `run {entry}() = {v}` (names a
  non-default entry too).
- Failures are never quieted: `run: FAIL` + full diagnostic JSON in
  both modes; prior prints appear raw in quiet mode (composes with the
  HEAVY-TEST-1 flush rather than conflicting with it).
- Flags accepted before or after the file (`run --quiet prog.klang`
  == `run prog.klang main --quiet`); `-q` is the short form. Bare
  `klang run --quiet` (no file) is usage + exit 2, not a confusing
  "cannot read".

## Files changed

- `src/main.rs` — `quiet` bool + flag-tolerant path/entry resolution
  for `run` only (`run_path`/`run_entry`; repair/fmt indexing
  untouched), preamble gating, raw-vs-prefixed print rendering
  (interpreter + JIT arms), `load_with_imports(entry, quiet)` for the
  `file:` line, usage text.
- `tests/quiet_gates.rs` (new, 5 tests) — exact-bytes quiet output
  (`hello\nrun main() = 0\n`), flag positions + short form, default
  dump unchanged (all six markers), quiet failure (raw print before
  FAIL + code, no preamble/prefix), custom-entry result line.
- `README.md` + `SPEC.md` — one row/line each for the flag (no new
  `@run` samples, docs count unchanged by construction).

## Test results (real output)

`cargo test --test quiet_gates` (with `CARGO_TARGET_DIR=/tmp/klang-target`):

```
running 5 tests
test quiet_custom_entry_names_result_line ... ok
test default_run_keeps_full_dump ... ok
test quiet_failure_still_shows_diagnostic ... ok
test quiet_flag_before_file_and_short_form ... ok
test quiet_run_prints_only_program_output ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

Live binary probes (`/tmp/klang-target/debug/klang`, program prints
`hello`, returns 0):

```
=== klang run prog.klang main --quiet ===
hello
run main() = 0
(EXIT=0)
=== klang run prog.klang main (default, unchanged) ===
file: /tmp/quiet_hello.klang (53 bytes)
parse: OK (1 functions, 0 structs, 0 enums)
  fn main @[0,0] effects=[]
check: OK (0 diagnostics)
mir:
func main() @[0,0]:
  ...
print: hello
run main() = 0
(EXIT=0)
```

Quiet failure probe (`heavy_print_loss.klang`, exit 1):

```
before failure
run: FAIL
  "code": "E-IO-NOT-FOUND",
```

Full regression: `SUM passed=367 failed=0` (362 baseline + 5 new),
`GREP sum=367` — matching, delta exactly +5.
`scripts/verify_docs.py`: `verified 57 samples`,
`all docs samples verified, 0 UNVERIFIED markers` (unchanged — default
output untouched, doc edits add no samples). Note: one full-suite run
during this phase reported `321 passed, 1 failed` with truncated
output (only a partial suite count printed); the failing test name was
not captured. Three subsequent consecutive full runs report `367/367`
across 53 suites with zero FAILED/panic lines, plus a targeted
`osio_http_gates` 10/10 rerun — the documented internet-dependent
suite (`osio_http_public_*`, STDLIB-NET-1) is the suspected transient
surface. Unreproduced; watched, not attributed to this change (the
change surface is CLI-display-only; library behavior is untouched).

---

# CLEANUP-1 — repo and docs cleanup (code-verified, pure moves)

PRD Phase B. Baseline at session start: `SUM passed=367 failed=0`
(53 suites), `GREP sum=367`, docs 57/57 — all re-measured after the
moves below with identical results (a pure reorganization changes no
behavior).

## 1. Root planning docs → `docs/` (moved, not rewritten)

`git mv` (pure renames, interiors byte-identical) for 7 files:
`architecture.md`, `features.md`, `integration.md`,
`master_prompt.md`, `planning.md`, `prd.md`, `roadmap.md` → `docs/`.
Root now holds only `README.md`, `SPEC.md`, `AUDIT.md`.
`docs/README.md` index updated (`../architecture.md` →
same-folder listing of all seven).

Deviations from the PRD list, both stated with reasons:
- `SPEC.md` stays at root. It is the live language spec (v0.5), not
  a stale planning file, and its 3 ```klang blocks carry no
  machine-checked directives (the first needs `import "lib.klang"`
  to resolve). `docs/` membership would make `verify_docs.py` fail
  on it — moving it means rewriting the spec or the harness, both
  out of scope for a cleanup. `docs/README.md`'s existing
  `../SPEC.md` link keeps working.
- No deletions. All seven moved files are Stage-1/harness-era history
  ("Warden" name, Kaggle/Ollama setup, pre-MCP repair draft) and read
  stale against AUDIT.md's verified state — but none is an exact
  duplicate of anything, so per the PRD's deletion rule they move
  as-is. Their bare-filename cross-mentions (`features.md`,
  `master_prompt.md`, …) moved together, so they stay mutually
  resolvable; mentions of root `SPEC.md` from inside moved files are
  frozen historical prose, deliberately not rewritten (same
  "record what was believed when" rule AUDIT.md applies to
  `prd.md`/`roadmap.md`/`integration.md`).
- No `src`/test/doc-content reference needed updating: the only
  in-repo pointer to the moved set is `docs/README.md:22` (fixed)
  and a code comment in `src/repair/prompt.rs:10` (prose mention of
  "SPEC.md", still accurate — the spec did not move).

## 2. `tests/` subfolders — probed, NOT done (structural constraint)

Empirical probe (`cargo 1.98.1`): a `tests/_probe_subdir/probe_test.rs`
(with a passing test) is invisible to cargo — `cargo test --test
probe_test` reports no such target and the test never runs (probe
dir removed afterwards). Cargo auto-discovers only `tests/*.rs`
directly; subfolders need 50 explicit `[[test]]` stanzas or
`#[path]`-harness rewrites. Per the PRD's own escape clause, the
plan adjusts: flat `tests/` is structurally imposed, and the churn
(50 manifest stanzas, renamed test binaries, per-new-file manifest
edits forever) outweighs the cosmetic gain. No test file moved; the
prefix grouping (`v2_*`, `osio_*`, …) stands as the organization
convention. No `Cargo.toml` changes.

## 3. Top-level `stdlib/` → `experiments/self-host-lexer.klang`

The name collided with the real Rust stdlib (`src/stdlib/`). The
moved file is a 423-line self-hosting lexer experiment (`mod lexer`,
byte-offset helpers) with zero references anywhere in the repo
(verified by grep: the only `lexer.klang` hit is an unrelated temp
file in `tests/module_gates.rs`). Pure `git mv` + one header-comment
line updated to the new path (now also states it is not
`src/stdlib/`). Live `klang check` on the new path: `check: OK`.

## 4. `npm-package/` — reported, NOT removed (per PRD)

Real distribution scaffold, not leftover: `build.sh` cross-builds
five platform binaries (linux-x64/arm64-musl, darwin-x64/arm64,
win32-x64 — the musl static-link and osxcross/mingw notes match the
platforms Phase E will need), `klang/package.json` is the npm
wrapper with matching `optionalDependencies`, plus `postinstall.js`.
Zero references from `.github/workflows/release.yml`, docs, or
`Cargo.toml` — nothing in-repo drives it today, but it is the only
npm distribution path in the tree. Verdict: load-bearing for
distribution, safe to leave alone; removal needs an explicit call.

## Test results (real output, `CARGO_TARGET_DIR=/tmp/klang-target`)

```
suites=53 SUM passed=367 failed=0
GREP sum=367
verified 57 samples: {'check-ok': 0, 'run': 29, 'check-fail': 19, 'run-fail': 9}
all docs samples verified, 0 UNVERIFIED markers
```

Identical to baseline on all three numbers (367/367/57) — moves only,
no behavior change. `cargo build` likewise clean (only pre-existing
`v2_lowering` warnings).

---

# BUGHUNT-2 — Phase C bug hunt (code-verified)

PRD Phase C. Baseline at session start: `SUM passed=367 failed=0`
(53 suites), `GREP sum=367`, docs 57/57.

## Part 1 — existing findings

### HEAVY-TEST-2 — DECIDED + FIXED: check-v2 validates tune/verify constants

Decision (the design call the finding asked for): `check-v2`
validates a `tune<T>()`/`verify<T>()` argument **iff it is a
compile-time constant** (struct literal of literal fields, recursive).
Rationale: the reported case is constant, so punting it to runtime is
a real check/run inconsistency (F9 set the precedent that v1 missing
struct fields are check-time errors); but a dynamic value (`Var`,
`Call`, `listen()`, …) can never be checked statically, so those stay
runtime-checked by construction. A literal-only hack with its own
validator was rejected — instead the pass reuses
`SchemaRegistry::check_boundary` (the same `Schema::validate` the
runtime uses), so check and run cannot drift by construction.

Pre-work discovery (read, not assumed): whole-program `check-v2`
was lex-only — unknown schemas, bad literals, even unparseable
programs all returned `check: OK`. The v2 semantic pieces
(`sema::tuner`, `schema_check`, `flow_capture`, `echo_lifetime`) are
wired to nothing but unit tests; no whole-program v2 check pass
exists. The fix below closes the tune/verify slice only — flow
capture, echo lifetimes, and resonance qualifier checking stay
future work, stated here, not implied.

What was built (`src/sema/v2_tune.rs`, new; `src/sema/mod.rs`;
`src/mcp.rs` `v2_check_source_with_file`; `src/main.rs`
`run_v2_check_mode` passes the real path):
- Registry built from program schemas (same `str`/`i32`/`bool`
  mapping as runtime startup; unknown field types emit the same
  `E-SCHEMA-NOT-FOUND` the runtime emits).
- Function + echo bodies walked recursively (nested tunes inside
  calls/arithmetic/ifs/returns all visited; flow bodies are
  string-held source, skipped — documented).
- Unknown schema → `E-SCHEMA-NOT-FOUND`; bad constant →
  `E-SCHEMA-INVALID`; dynamic value → skipped. New diagnostics carry
  the real file path (F12-class threading for the new code; the
  pre-existing `input.v2` placeholders in the v1-lex and v2-parse
  layers are HEAVY-TEST-3 scope, untouched).
- Runtime agreement verified, not assumed: the interpreter encodes
  `Bool` as `Int(0/1)` and accepts both for `bool` fields, so the
  constant conversion mirrors exactly that (`true`/`0`/`1` validate,
  `2` fails); extra literal fields are ignored exactly as the
  runtime ignores them (and a dynamic *extra* field no longer
  suppresses checking of declared fields). No float literals exist
  in the v2 grammar, so no float coercion exists to mirror.

Live-verified agreement matrix (`klang check-v2` vs `klang run-v2`,
same file both sides):
- `tune<Profile>({name:"bob", age:"not a number"})`: check FAIL +
  run FAIL, byte-identical message (``Profile@1:22e75951d5dbf9e9`
  rejected `age`: want int, got str` — same identity hash, proving
  one shared validator).
- Good literal: check OK + run OK. Unknown schema:
  `E-SCHEMA-NOT-FOUND` at check. Dynamic (`listen()` result): check
  OK + run OK (prints the Struct). `verify` bad literal: FAIL both.
  Missing field: `want int, got missing` both. Scalar
  (`tune<S>(42)`): `want record, got int` both. `true`/`1` for bool:
  clean both; `2`: identical FAIL both. Garbage program: `E-PARSE-V2`
  (previously false-OK — full programs now parse in check-v2).

Regression tests: `tests/v2_tune_check_gates.rs` (7 tests, pinning
per-shape check/run agreement incl. message equality and the real
file path). Existing `v2_mcp_gates` snippet-path tests unaffected
(untouched code path).

### HEAVY-TEST-3 — SCOPED OUT: needs its own PRD, not fixed here

Verdict as the PRD's option anticipated: closing v2×OSIO interop
means array literals + a v2 stdlib surface (which builtins, what
resonance signatures, docs) across grammar, runtime, and reference —
a design-sized task, not a patch. No partial wiring attempted (a
half-wired builtin set would be worse than the current loud
`E-PARSE-V2` boundary). Follow-up PRD should also take the two
tagged-along items: the `input.v2`/`runtime` file-placeholder
instances (F12 pattern) and whole-program flow/echo semantic checks.
Untouched this pass: verified still true by re-grep
(`run_process|time_sleep|http_get|read_file|regex` — zero hits in
`src/runtime/v2.rs`; no `Array` in `src/parser/v2.rs`).

## Part 2 — fresh adversarial pass (real programs, live binary)

1. **Qualified explicit generics — FIXED (closes HEAVY-TEST-4).**
   `m::count<i32>(5)` misparsed as comparisons (F15 documented it as
   future work). Extended the F15 backtracking across `::`:
   `ast::Expr::EnumCtor` gains `type_args` (empty = historical
   behavior everywhere); the two-segment parser arm speculates
   `<...>` identically to the bare-name arm (same full-shape commit,
   `m::f < 123` still a comparison); `rewrite_ctor` moves the args
   onto the resolved `Call` (HIR's generic seeding then applies
   unchanged); `fmt` renders the new shape round-trip; enum-typed
   ctors and MIR ignore them (runtime stays inference-determined).
   Live: `check: OK`, `run` prints `5`; mismatch
   `m::same<i32>(1,"x")` gets the F13 wording
   (``T was inferred as i32 from explicit type argument 0``);
   implicit qualified inference per-site works (`42`/`hi`);
   three-segment `m::E::V<T>` deliberately untouched (still
   comparison-fallback — stated boundary). Tests: +4 in
   `tests/generic_gates.rs` (run, mismatch wording, comparison
   guard, fmt round-trip).
2. **verify-path + flow `dep=` call-site — probed, not implemented.**
   `verify` shares the tune evaluator and is covered by the
   HEAVY-TEST-2 pass + test above. `classify(10, dep=threshold=20)`
   fails clean `E-PARSE-V2` at `dep` — the supported form stays
   scope-captured deps (`classify(10)`); call-site dep override is
   v2 grammar design work, listed as roadmap, left there.
3. **Concurrency + HTTP — works.** Two `spawn`ed `http_get` tasks
   against a local server: `check: OK`, prints `200` + `17`
   (body length), `run main() = 42`, no deadlock. Mixed
   success/failure siblings surface the single failure unwrapped
   (`E-NET-UNREACHABLE`, exit 1, prompt termination) — matching the
   documented task-group contract, not a new failure mode.

## Test results (real output, `CARGO_TARGET_DIR=/tmp/klang-target`)

```
--test v2_tune_check_gates: 7 passed; 0 failed
--test generic_gates: 14 passed; 0 failed (10 existing + 4 new)
Full: suites=54 SUM passed=378 failed=0 (367 baseline + 11 new)
GREP sum=378 — matching, delta exactly +11.
verify_docs.py: verified 57 samples, 0 UNVERIFIED (unchanged).
```

No `src/` changes beyond the two fixes above; `cargo build` clean
(only pre-existing `v2_lowering` warnings).

---

# SECAUDIT-1 — Phase D security review (code-verified)

PRD Phase D. Baseline at session start: `SUM passed=378 failed=0`
(54 suites), `GREP sum=378`, docs 57/57. Method note: `cargo audit`
is not installed here (and building it exceeds this pass); the
85→118-crate lockfile was batch-queried against the OSV API instead
(same approach as the earlier dependency scan, now re-run live).

## Result table

| # | Item checked | Finding |
|---|---|---|
| 1 | Dependency CVEs (118 third-party crates, OSV batch) | **1 hit → fixed** (salsa UAF, bumped) |
| 2 | `process::run` argv-only design | **Clean** (single spawn site, no shell) |
| 3 | File-path sandbox vs new adversarial paths | **Clean** (`..`/absolute/NUL all loud) + 1 **documented residual** (tmp symlinks followed, live-demonstrated) |
| 4 | HTTP unrestricted capability (SSRF-adjacent) | **Flagged**: no allowlist by PRD design (stays); secret-echo in diagnostics **fixed** |
| 5 | `unsafe` blocks (JIT transmutes) | **Clean** (unchanged shape, still flag-gated) |
| 6 | Sensitive data in diagnostics | **2 leaks found → fixed** (header values, URL userinfo); file paths/int-parse/regex causes clean |

## D1 — FIXED: RUSTSEC-2026-0308 (salsa use-after-free)

OSV batch over all 118 non-klang lockfile entries returned exactly
one hit: `salsa 0.28.4`, use-after-free in interned values/cached
results reachable through safe APIs (categories
memory-corruption/memory-exposure/code-execution; fixed in 0.28.5,
advisory's fix PR salsa-rs#1329). Klang's exposure assessed by
reading `src/db.rs`: one `#[salsa::input]`, two `#[salsa::tracked]`
fns over that input, zero `#[salsa::interned]` structs, derived
(structural) `Eq` impls only — neither advisory trigger (bad app
`Eq`, interned-value misuse) is present in our code, and the CLI
run path does not use the DB at all. Still fixed rather than
argued-away (it is a soundness hole in a dependency we ship):
`cargo update -p salsa` → `salsa`/`salsa-macro-rules`/`salsa-macros`
0.28.4 → 0.28.5 (patch-only family bump). Rescan after the bump:
**0 hits across 118 crates** (live output, this session).

## D2 — clean: argv-only spawn confirmed

The only process-spawn in `src/` is
`std::process::Command::new(cmd).args(args)` (`src/stdlib/process.rs:92`);
no `sh -c`, `cmd /c`, or string-joined command anywhere (grep
verified). Unchanged since STDLIB-OSIO-2; no action.

## D3 — sandbox holds; symlink residual live-demonstrated

`reject_unsafe_path` guards all five file builtins (read/write/
append/exists/remove — each call site read). New adversarial probes,
all run live (`klang run`, exit codes observed):
- `read_file("../../../etc/passwd")` and buried `sub/../../…` →
  `E-RUNTIME "unsafe path"`, exit 1 (any `..` component rejects;
  `components()` never normalizes a `..` away).
- Absolute `/etc/hostname` → `E-RUNTIME "unsafe absolute path"`.
- `\u0000` in a path (real NUL byte, `len` 8 proves it): lexical
  guard passes (under tmp), OS layer fails loud `E-IO-FAILED`
  ("NUL byte", real cause) — no panic, no fs touch.
- **Residual (was F14 analysis, now live evidence):** a symlink
  planted in tmp pointing at `/etc/hostname` reads THROUGH
  (`print`ed the hostname); `write_file` through a tmp symlink
  follows the link too. Planting the link already needs fs access,
  and canonicalizing would break legitimately symlinked temp dirs
  (macOS `/tmp → /private/tmp`) — so still documented, not changed.
  `remove_file` on a symlink removes the LINK only (target intact,
  verified) — safe semantics worth having pinned.
- New tests: `tests/sandbox_gates.rs` (4 tests: buried+leading `..`,
  NUL, link-follow pinned deliberately, unlink-link-only).

## D4 — flagged: HTTP has no allowlist (by design, stays)

Unchanged from the STDLIB-NET-1 decision and its docs statement:
`http_get`/`http_post` fetch whatever URL they are given, so a
Klang program can probe internal addresses from wherever it runs
(SSRF-adjacent by construction, same trust class as `run_process`
spawning anything). Restricting it needs product semantics (per-host
policy? redirect rules?) — follow-up PRD material, not a patch.
What WAS in scope and is fixed: the diagnostics echoed credentials
(see D6).

## D5 — clean: `unsafe` unchanged, still flag-gated

All real `unsafe` is 5 transmutes in `src/jit.rs` (finalized
Cranelift function pointers cast to matching `fn(i64…) -> i64`
types, arity-checked before each call — the standard pattern, same
shape as previously audited). JIT remains opt-in `--backend-jit`
only (interpreter default); every other `unsafe` hit is the word in
names/comments (`is_unsafe_import_path`, etc.). No change.

## D6 — FIXED: diagnostics echoed secrets (2 leaks, live-demonstrated)

Spot-check found two real leaks (both verified live before the fix
with `s3cret`-marked values rendering verbatim into diagnostic
JSON — JSON the MCP layer returns and repair logs persist):
- A malformed header entry (`["Bearer s3cret-token-xyz"]`) rendered
  whole into `E-NET-INVALID-HEADER`'s message.
- A failing URL with userinfo (`https://user:s3cret-pass@…`)
  rendered whole into `E-NET-INVALID-URL`'s message.
- Fix (`src/stdlib/http.rs`): header errors name only the header
  (`header "X-Bad"`; colon-less entries report byte-length, never
  content); URL errors pass through `redact_url`
  (`https://***@host/path`, query kept for debugging). Causes
  already carried no secrets (verified: bad-value cause is
  `failed to parse header value`). Clean elsewhere:
  `int()`/`float()` parse failures never echo the input
  (`int() cannot parse string`); file errors echo the path only;
  `run_process` failures echo the command name only.
- Live after-fix: secret strings absent, name/host/debuggability
  retained (`header "X-Bad"`, `https://***@not a host/`).
- New tests: +2 in `tests/osio_http_gates.rs` (header values and
  userinfo absent from JSON; name/host/`***@` present). Pre-existing
  header assertions (codes, "colon" wording) pass unchanged.

## Test results (real output, `CARGO_TARGET_DIR=/tmp/klang-target`)

```
--test sandbox_gates: 4 passed; 0 failed (new file)
--test osio_http_gates: 12 passed; 0 failed (10 existing + 2 new)
Full: suites=55 SUM passed=384 failed=0 (378 baseline + 6 new)
GREP sum=384 — matching, delta exactly +6.
verify_docs.py: verified 57 samples, 0 UNVERIFIED (unchanged).
```

`cargo build` clean (only pre-existing `v2_lowering` warnings).
OSV rescan post-fix: 0 hits / 118 crates.

---

# INSTALL-1 — Phase E install doc (code-verified)

PRD Phase E. New file `docs/install.md` (+ one index bullet in
`docs/README.md`): platform + exact commands only, no prose beyond
that. Pinned tag `v0.3.0` — nothing hardcoded from memory:

- `git tag` latest is `v0.3.0`; repo is
  `github.com/raunakphull62-rgb/never_give_up` (from `git remote`).
- `.github/workflows/release.yml` matrix builds exactly 4 targets:
  `linux-x86_64` (gnu), `linux-aarch64-musl-termux` (musl cross,
  static — the workflow header notes it doubles as the
  Termux/Android build, NDK-free like ripgrep/fd), `windows-x86_64`
  (msvc), `macos-aarch64-apple-silicon` (Intel Macs dropped for
  runner queues). Packaging: `tar -czf klang-<name>.tar.gz klang`
  on Unix, `7z a klang-<name>.zip klang.exe` on Windows — so the
  doc uses `tar -xzf` for the three Unix assets and
  `Expand-Archive` for the Windows `.zip` (real formats, not assumed).
- GitHub Releases API for tag `v0.3.0`: published, not
  draft/prerelease, with all four assets live
  (`klang-linux-x86_64.tar.gz` 2770300 bytes,
  `klang-linux-aarch64-musl-termux.tar.gz`,
  `klang-macos-aarch64-apple-silicon.tar.gz`,
  `klang-windows-x86_64.zip` — each with real download counts).
  The PRD's "last confirmed" platform list is still accurate.

## Verification (Linux x86_64, this environment, real output)

The doc's exact commands run verbatim in a clean dir (pre-existing
`/usr/local/bin/klang` backed up first, restored byte-identical
after — `cmp` confirmed):

```
wget .../releases/download/v0.3.0/klang-linux-x86_64.tar.gz
  → saved [2770300/2770300] (matches the API byte count exactly)
tar -xzf klang-linux-x86_64.tar.gz
chmod +x klang
mv klang /usr/local/bin/klang
klang --version
  → klang 0.1.0
```

Smoke test of the installed release binary (a real program, not just
`--version`):

```
klang run /tmp/installcheck/smoke.klang main
  print: installed-ok
  run main() = 42
(EXIT=0)
```

Honesty notes: (1) the other three platforms' commands use
verified-real asset names + their documented extraction formats but
were not executed here (Linux-only environment) — the doc does not
claim otherwise; (2) release `v0.3.0` predates uncommitted worktree
features (e.g. `run --quiet` fails on it with `cannot read --quiet`
— probed live), so the doc promises only what the release contains
(`--version` + `run`, both proven above).

## Regression (this phase)

`suites=55 SUM passed=384 failed=0`, docs `57/57` (the new page has
`sh`/`powershell` blocks only — `verify_docs.py` scans ```klang
blocks, so the count is unchanged by construction). Docs-only phase:
no `src/`/`tests/` changes.

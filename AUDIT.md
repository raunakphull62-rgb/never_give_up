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

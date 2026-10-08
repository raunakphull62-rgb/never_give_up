# D4. Return-type ergonomics: optional `->` for unit functions and `main`

Status: design note — no implementation. (Phase 2; see the Phase 2 report
for the recommendation summary and ordering.)

## 1. Problem

Every `fn` — named, `main`, or closure literal — must spell `-> type`
(`src/parser/mod.rs:1193,2338`, `src/parser/v2.rs:521`; a missing arrow
is `E-PARSE`, now with a helpful message since Phase 1c). For value
functions that ceremony is load-bearing documentation. For *unit*
functions (`print` wrappers, setup/teardown, `main`s that just run) it
is pure boilerplate: `fn main() -> void { … }` and `fn log(m: str) ->
void { … }` say nothing the body doesn't. AI-written code — Klang's
primary author — gets this wrong in exactly one direction (dropping the
`->`), and today that direction is always an error, even when the
intent (no value returned) is unambiguous.

Constraints from the audit: `main` may already return *any* type
(`Int` → exit code, else 0; `src/main.rs:1677-1682`), `-> void` works,
`-> ()` is rejected by the parser, and closures need their literal type
for the lifted-signature table (`src/hir.rs:358-383`).

## 2. Options

### Option A — omitting `->` means `-> void` (recommended; see §3)

```text
fn main() {
    print("hi")
}

fn log(m: str) {
    print(m)
}

fn add(a: i32, b: i32) -> i32 {   // value functions: unchanged, still explicit
    return a + b
}

let f = fn(x: i32) { print(x) }   // closures too: omitted -> void
```

Grammar: the `->` clause becomes optional; absence desugars to
`return_ty = "void"` *at parse time*, so every downstream stage (HIR
sigs, closure lifting, MIR, fmt) sees exactly today's `-> void`
programs. A `return <expr>` inside a void-defaulted body is `E-TYPE`
against `void` (today's rule, no new code); bare `return`… does not
exist as a statement form (only `return expr`), so nothing else
changes. Explicit `-> void` keeps working forever.

Pros: removes the boilerplate direction with pure parse-time sugar;
the dropped-`-> i32` failure becomes a local `E-TYPE` with a better
span. Cons: two spellings for unit (omitted vs explicit `void`) —
`fmt` canonicalizes to the explicit form, so the codebase converges.

### Option B — inference where unambiguous

```text
fn main() {                        // body has no `return v`: void (same as A)
    print("hi")
}

fn double(n: i32) {                // body ends `return n * 2`: inferred i32??
    return n * 2
}
```

Two sub-cases: (B1) void-default *only* when the body contains no
`return <expr>` (otherwise `E-PARSE` still demands `->`); (B2) full
return-type inference from `return` expressions. B1 is A plus a
body scan that buys nothing (the failure it adds — "you returned a
value but declared nothing" — is precisely what A's `E-TYPE` already
says, one stage later, with better spans). B2 (Hindley-Milner-lite over
`return`s, `if`-branches, match arms, generic instantiations) is a
research project wearing an ergonomics hat: error messages degrade
("cannot infer" instead of "want i32, got str"), AI checkability
degrades (no annotation to verify against), and the checker gains a
fixpoint.

Pros: B1 catches value-returns-without-a-type at parse; B2 is
maximally terse. Cons: B1's body scan re-derives what Option A states
plainly for no new expressiveness; B2 degrades errors and AI
checkability and grows a checker fixpoint — the highest cost of the
three options.

### Option C — keep `->` required (status quo + 1c message)

```text
fn main() -> void {                   // unit spelled out, as today
    print("hi")
}

fn log(m: str) -> void {              // same for helpers
    print(m)
}
```

Change nothing further. The Phase 1c error already turns the ceremony
into a one-line fix, and required annotations are machine-checkable
documentation — arguably the *right* default for AI-written code. Cost:
zero. Benefit: zero beyond today.

Pros: zero cost, zero risk; keeps annotations as checkable
documentation. Cons: the single most-hit ceremony stays in place.

## 3. Recommendation and why

**Option A.**

Why: it removes the boilerplate direction while keeping the guarantee
direction. Every program that compiles today compiles identically (pure
sugar at parse time — `-> void` desugared before HIR). The failure mode
for a dropped `-> i32` is a loud `E-TYPE` on the `return` line, which
is *more* local than today's `E-PARSE` at the `{`. Option B1 adds a
body scan to re-derive what A states plainly; B2 trades checkable
annotations for inference mysteries — wrong for a language whose
primary authors are models and whose repair loop leans on stated types.
Option C is defensible but leaves the single most-hit ceremony in place
after 1c already admitted it is a ceremony.

Explicitly out: `-> ()` acceptance. Either accept it as a `void`
spelling (one parser arm, but two spellings forever) or keep rejecting
it (one spelling, SPEC already documents `void`-only). Recommend **keep
rejecting** — unifying on `void` avoids `()`: "unit or parens?" questions
in every future generic/type-position discussion.

## 4. Migration impact

- The 26 published packages: **no edits required, none affected**.
  Additive sugar: every spelled `->` keeps its meaning; packages may
  drop `-> void` at their leisure (a cosmetic major, never forced).
- Existing programs: everything that parses today parses identically
  (the `->` form is untouched). Newly accepted: only programs that
  fail `E-PARSE` today (`fn f() { … }`). One sharp edge to document:
  `fn f() { return 42 }` flips from `E-PARSE` (missing type) to
  `E-TYPE` (`return` vs `void`) — different code, same loudness, better
  span. No silent behavior change exists by construction (desugar =
  today's `void`).
- `main` specifically: `fn main() { … }` becomes the documented
  hello-world shape (examples and `getting-started` should show it);
  `fn main() -> i32` keeps working with exit-code semantics.

## 5. JIT / interpreter / v2 impact

None beyond parsing. Desugared `void` flows through the exact paths
`-> void` uses today (HIR `Ty::Void`, MIR fall-off-the-end `0`, JIT
`last_var`-seeded `0`, exit-code mapping). No new MIR op, no ABI
change, no backend work. Fmt prints the desugared form (`fn main() ->
void` — canonical, idempotent; the omitted form is writer's sugar, like
today's optional semicolons).

## 6. Test plan

- Parse: `fn f() {}`, `fn main() {}`, `fn(x: i32) {}` all parse with
  `return_ty == "void"` (assert on the AST, not just acceptance).
- Check: void-defaulted body with `return 42` is `E-TYPE` naming `void`;
  without returns is clean; explicit `-> void` and `-> i32` unchanged.
- Error-message regression: the 1c gates still pass (the `->`-present
  path is untouched; add a gate that the *new* `E-TYPE`-for-value case
  suggests adding `-> i32`).
- Round-trip: `fmt` of an omitted-`->` program equals `fmt` of its
  explicit-`-> void` twin; both re-parse identically.
- Back-compat: all 26 `*_test.klang` unchanged; v2 `fn` (which also
  requires `->`, `src/parser/v2.rs:521`) gets the same desugar + gates,
  or an explicit decision not to (v2's `flow`/`echo` already vary —
  document whichever is chosen).
- Docs: `getting-started`/SPEC examples updated to the short `main`
  shape (each a `verify_docs.py` sample, so green-ness is mechanical).

## 7. Effort estimate (S / M / L) and risks

**S** (parser desugar × 3 sites — v1 fn, v1 closure, v2 fn — plus gates
and example updates). No checker, runtime, or backend work.

# D1. Namespaces / qualified imports

Status: design note — no implementation. (Phase 2; see the Phase 2 report
for the recommendation summary and ordering.)

## 1. Problem

A Klang program is a flat merged namespace: `import "x.klang"` merges the
whole file, and two imported files defining the same top-level name are a
hard `E-DUPLICATE` (`src/imports.rs:436-456`). There is no way to use two
same-named functions side by side, and no way to say *which file* a call
comes from. The 26 published packages cope by prefixing every public name
(`json_parse`, `base64_decode`, `cli_get_flag`, …), which works but bakes
the package name into every identifier and makes collisions a
package-rename event instead of an import-site decision.

What exists today (all must keep working):

- Whole-file merge: `import "lib.klang"` (`src/parser/mod.rs:919-941`).
- Selective merge: `import { name } from "lib.klang"`
  (`src/parser/mod.rs:962-1015`); re-merging the same `(file, name)` is
  idempotent, a different file's same name is `E-DUPLICATE`.
- `mod name { … }` blocks with `pub`/private visibility and qualified
  `m::item` paths, flattened before checking (`src/modules.rs:153-272`).

## 2. Options

### Option A — file aliases (`import "…" as m`, `m.f()` calls)

```text
import "math/lib.klang" as m
import "other/lib.klang" as o

fn main() -> i32 {
    print(m.max(1, 2))
    print(o.max(1, 2))
    return 0
}
```

The loader resolves `"math/lib.klang"` (relative + existing vendor
fallback) but merges **nothing** into the flat namespace; instead it
registers the file's top-level items under the alias `m`. Calls spell
`m.name(...)`, struct literals `m.Point { … }`, enum construction
`m.Shape::Circle(…)` (mirroring how `m::E::V` already parses for
modules). Selectivity composes: `import { max } from "p" as m` is legal
but pointless; the plain form imports the whole file's API under `m`.
Two aliases for the same file (`as m`, `as m2`) are allowed (same
canonical path, two views); re-aliasing one alias name to two different
files is `E-DUPLICATE`.

Name-resolution rule: `m.f` resolves only inside the importing file's
scope; the callee body still resolves its *own* unqualified names in its
home file (no dynamic scoping — a call through an alias executes exactly
the same function value as an unqualified call would).

Pros: value-path syntax (composes with first-class functions);
selectivity composes; plain files gain no module-visibility side
effects. Cons: a new `m.name` expression form to parse and check; needs
the alias-vs-`let` shadowing rule (§4).

### Option B — files are implicit modules (no new call syntax)

```text
import "math/lib.klang" as m

fn main() -> i32 {
    print(m::max(1, 2))
    return 0
}
```

`import "p" as m` desugars to a synthetic `mod m { <contents of p> }`
with everything `pub`, reusing the *existing* module machinery
(flattening, `pub` checking, `m::item` paths in `src/modules.rs`) almost
unchanged. No new expression form: `m::max(...)` already parses and
checks today. Transitive imports of `p` behave exactly like today (they
merge into `p`'s scope, i.e. inside the synthetic module).

Pros: cheapest build (reuses `mod` flattening/`pub` checking); no new
syntax. Cons: overloads `::` (declaration vs file membership); `pub`
inside plain files suddenly matters; changes what transitive imports
mean.

### Option C — selective import with per-name aliases (no file aliases)

```text
import { max as imax, min as imin } from "math/lib.klang"
import { max as omax } from "other/lib.klang"

fn main() -> i32 {
    print(imax(1, 2))
    print(omax(1, 2))
    return 0
}
```

Extends the existing selective form with `name as alias` (comma
separated, mirroring ES modules). No qualified paths, no file-level
identity: each imported name is still merged flat, just under a
renamed binding. Whole-file `import "p"` keeps its exact current
meaning.

Pros: smallest change (extends the existing selective form); no
qualified paths to specify. Cons: solves collisions only — call sites
still cannot name a file, and whole-file imports stay
collision-prone.

## 3. Recommendation and why

**Option A**, with the call-site spelling `m.f()` (not `m::f()`).

Why: it reads as a value path (functions *are* first-class values in
Klang, including closures), so `m.f` can later pass the function itself
(`apply(m.f, x)`) without a second spelling; `m::f` would collide
mentally with the *declaration* namespace (`mod` blocks, enum variants
`Shape::Circle`), where `::` means "a member declared inside". Option B
is cheaper to build but overloads `::` and inherits module visibility
semantics (`pub` inside plain files would suddenly matter) that plain
`.klang` files never had. Option C solves only the collision half —
call sites still cannot say which file an unaliased helper came from,
and whole-file imports stay collision-prone.

Concretely: extend the loader (`src/imports.rs`) with an alias table
`alias -> canonical file`, keep the merge path byte-identical when no
`as` is present, resolve `m.name` at check time against the aliased
file's item table (arity/types flow exactly as for flat calls), and
lower `m.f(args)` to the existing `Call` op with the resolved (possibly
file-prefixed internal) name.

## 4. Migration impact

- The 26 published packages: **zero impact required**. Unqualified
  imports keep merging exactly as today; every prefixed stdlib name
  (`json_parse`, …) keeps compiling unchanged, and packages need not
  adopt aliases. A package *may* later re-export a shorter alias-based
  API, but that is a new major version by choice, not a forced edit.
- Existing programs: purely additive. Old code parses and checks
  identically (no new keywords: `as` is contextual after an import
  string; a function literally named `as` still parses — the parser
  only treats `as` specially in `import "…" as ident` position).
  Programs that today fail with `E-DUPLICATE` gain a fix: alias one side
  instead of renaming a file.
- Risk: alias shadowing (`let m = …` vs `import … as m`) must be a loud
  `E-DUPLICATE`/`E-UNDEFINED`, never a silent pick-one. Define: alias
  names live in the same binding scope as `let` names.
- Re-exports: step 1 needs **no new re-export syntax**. A package that
  wants a short public name today already defines a wrapper (`fn max`
  calling the vendored helper); with aliases, consumers just alias the
  defining file instead. If a `pub use m.f`-style form is ever added, it
  is a separate proposal — aliases compose with it (re-export an
  alias, or alias a re-export) without changing either rule.
- Nested imports inside vendored packages: an aliased file's *own*
  relative imports resolve against its own directory exactly as today,
  including the vendor fallback (`.klang_pkgs/<name>/<version>/`).
  `import "os/lib.klang" as os` from user code falls back to the pinned
  vendor dir when no local file matches (SPEC.md §6); the vendor copy's
  internal `import "helper.klang"` resolves inside
  `.klang_pkgs/os/<version>/`, never against the user's directory.
  Aliases are file-local: they do not leak into the imported file, and
  the imported file's aliases do not leak back out (no transitive
  alias visibility — each file's alias table is its own).
- Package-manager interaction: aliasing changes no resolution input —
  `klang add`/`fetch`/`publish`, `klang.lock` pins, and the SHA-256
  trust anchor all key on canonical file identity (path or
  `registry:name@version`), not on the alias spelling. Two aliases for
  the same vendored file share one lock entry and one download. `klang
  publish` needs no manifest change (self-contained-packages rule
  unchanged: an aliased import of a vendored dep is still a dep, an
  aliased import of a sibling file is still local code).

## 5. JIT / interpreter / v2 impact

None at runtime. Aliases resolve at load/check time to plain function
names; MIR sees only the existing `Call`/`StructNew`/enum-tag ops, so
the interpreter runs unchanged and the int-only JIT accepts exactly the
same subset as before (aliased int calls JIT-compile; aliased string
calls are rejected exactly like flat ones). No ABI change, no new MIR
op. The v2 tree-walker needs the same load-time alias table as the v1
checker (it shares the import loader, so this falls out of resolving
aliases before either checker runs); v2's missing features (no
closures, no `try`) are orthogonal — an aliased call to a closure
value is still dynamic dispatch, rejected or run exactly as the flat
call would be.

## 6. Test plan

- Loader: alias two files defining `fn f` → both callable (`m.f`,
  `o.f`); same alias for two files → `E-DUPLICATE`; alias for a missing
  file → `E-IO-NOT-FOUND`; `m.unknown` → `E-UNDEFINED` naming the
  alias's available items.
- Diamond: `a` aliases `c` as `m`, `b` aliases `c` as `n`, entry imports
  `a, b` → one copy of `c` runs (file identity, not alias identity).
- Transitivity: aliased file's own relative imports resolve against its
  own directory (including vendor fallback for registry deps).
- Checker: arity/type mismatch through an alias reports the same
  `E-ARITY`/`E-TYPE` as the flat call; `let m = 1` + `import … as m` is
  loud.
- Back-compat: every `stdlib-packages/*/*_test.klang` passes unchanged;
  a new `tests/import_alias_gates.rs` pins the above.
- Fmt round-trip: `import "p" as m` formats idempotently.

## 7. Effort estimate (S / M / L) and risks

**M** (loader alias table + `m.name` resolution in HIR/closures/modules
visitors + MIR call lowering reuse + gates). No runtime work.

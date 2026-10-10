//! Typed HIR: effect + name + type checking over the parsed program.
//!
//! Checks in v0.5 (superset of v0.3):
//! - `spawn` must appear inside a `task_group`
//! - every handle bound by `let x = spawn ...` must be `await`-ed before its
//!   group ends, else E-TASK-CANCEL
//! - a function calling a `throws` callee must itself declare `throws`
//! - call arity must match the callee's params, else E-ARITY
//! - every variable must be a param or a prior `let`, else E-UNDEFINED
//! - `break`/`continue` only inside loops, else E-LOOP
//! - assignment targets must be already-bound names, else E-UNDEFINED
//! - struct literals must name a declared `struct`, else E-UNDEFINED
//! - binary/unary ops, calls, returns and conditions are type-checked,
//!   else E-TYPE (`Unknown` from maps/indexes/method generics is a wildcard)
//! - generic functions/structs/enums (`fn f<T>`, `struct B<T>`, `enum O<T>`)
//!   infer `T` per call/construction site from the argument types; a
//!   conflicting instantiation is E-TYPE

use std::collections::HashMap;

use crate::ast::{AssignTarget, Block, Effect, Expr, FunctionDecl, MatchArm, MatchBinding, Program, Stmt};
use crate::diagnostics::Diagnostic;

pub const FILE: &str = "input.warden";

/// Static type of an expression value.
///
/// `Unknown` is the wildcard: map lookups, dynamic indexes and generic
/// method results coerce to anything without emitting E-TYPE.
/// `Param` is a generic type variable (e.g. `T` in `fn f<T>(x: T)`); it is
/// also a wildcard wherever a concrete type is required, because its
/// instantiation is only known per call site, never in a generic body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Int,
    Float,
    Str,
    Bool,
    Array,
    Map,
    Struct(String),
    Enum(String),
    Param(String),
    /// `fn(T, ...) -> R`: a closure value's signature (see
    /// `crate::closures`). Structural: same parameter/return shape means
    /// the same type.
    Closure { params: Vec<Ty>, ret: Box<Ty> },
    Void,
    Unknown,
}

impl Ty {
    pub fn name(&self) -> String {
        match self {
            Ty::Int => "i32".to_string(),
            Ty::Float => "f64".to_string(),
            Ty::Str => "str".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Array => "array".to_string(),
            Ty::Map => "map".to_string(),
            Ty::Struct(n) => n.clone(),
            Ty::Enum(n) => n.clone(),
            Ty::Param(n) => n.clone(),
            Ty::Closure { params, ret } => {
                let ps: Vec<String> = params.iter().map(|t| t.name()).collect();
                format!("fn({})->{}", ps.join(", "), ret.name())
            }
            Ty::Void => "void".to_string(),
            Ty::Unknown => "unknown".to_string(),
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::Int | Ty::Float | Ty::Unknown)
    }
}

/// Parse a declared type annotation (`i32`, `f64`, `str`, `bool`, ...).
/// Unknown names become `Unknown` (user struct names resolve via structs map).
pub fn parse_ty(s: &str) -> Ty {    match s {
        "i32" | "i64" | "u32" | "u64" | "int" => Ty::Int,
        "f32" | "f64" | "float" => Ty::Float,
        "str" | "string" => Ty::Str,
        "bool" => Ty::Bool,
        "void" | "()" => Ty::Void,
        _ => Ty::Unknown,
    }
}

/// Enum registry: enum name -> variant name -> payload field types.
pub type EnumTable = HashMap<String, HashMap<String, Vec<Ty>>>;

/// Phase 1d: annotations that spell a wider type than Klang enforces.
///
/// `i64`/`u32`/`u64` parse to `Ty::Int` (checked `i32` range); `u8` — like
/// any other unknown name — stays `Unknown` (unchecked, dynamic). Both
/// keep compiling EXACTLY as before: this pass only surfaces the fact as
/// `W-TYPE-NARROW` (severity `warning`, never an error), so no existing
/// program changes meaning and `check` stays green. Real enforcement is
/// design item D3.
///
/// Mirrors resolution (`resolve_param_ty`/`resolve_generic_ty`): a name
/// that is a type parameter or a known struct/enum (nominal) never warns,
/// and qualified `m::T` paths are module lookups, not builtins.
pub fn narrow_type_warnings(program: &Program, file: &str) -> Vec<Diagnostic> {
    let structs: Vec<String> = program
        .structs
        .iter()
        .map(|s| s.name.clone())
        .chain(program.mods.iter().flat_map(|m| m.structs.iter().map(|s| s.name.clone())))
        .collect();
    let enums: Vec<String> = program
        .enums
        .iter()
        .map(|e| e.name.clone())
        .chain(program.mods.iter().flat_map(|m| m.enums.iter().map(|e| e.name.clone())))
        .collect();
    let mut out = Vec::new();
    let mut fns: Vec<(&FunctionDecl, String)> = program
        .functions
        .iter()
        .map(|f| (f, String::new()))
        .collect();
    for m in &program.mods {
        for f in &m.functions {
            fns.push((f, format!("{}::", m.name)));
        }
    }
    for (f, prefix) in &fns {
        let owner = format!("{prefix}{}", f.name);
        for p in &f.params {
            lint_annotation(
                &p.ty,
                &format!("parameter `{}` of function `{owner}`", p.name),
                &f.type_params,
                &enums,
                &structs,
                file,
                f.name_span,
                &mut out,
            );
        }
        lint_annotation(
            &f.return_ty,
            &format!("return type of function `{owner}`"),
            &f.type_params,
            &enums,
            &structs,
            file,
            f.name_span,
            &mut out,
        );
        let mut lits = Vec::new();
        crate::closures::collect_closures(&f.body, &mut lits);
        for lit in &lits {
            for p in &lit.params {
                lint_annotation(
                    &p.ty,
                    &format!("parameter `{}` of a closure in function `{owner}`", p.name),
                    &f.type_params,
                    &enums,
                    &structs,
                    file,
                    f.name_span,
                    &mut out,
                );
            }
            lint_annotation(
                &lit.return_ty,
                &format!("return type of a closure in function `{owner}`"),
                &f.type_params,
                &enums,
                &structs,
                file,
                f.name_span,
                &mut out,
            );
        }
    }
    for s in program.structs.iter().chain(
        program
            .mods
            .iter()
            .flat_map(|m| m.structs.iter()),
    ) {
        for fld in &s.fields {
            lint_annotation(
                &fld.ty,
                &format!("field `{}` of struct `{}`", fld.name, s.name),
                &s.type_params,
                &enums,
                &structs,
                file,
                (0, 0),
                &mut out,
            );
        }
    }
    for e in program.enums.iter().chain(
        program.mods.iter().flat_map(|m| m.enums.iter()),
    ) {
        for v in &e.variants {
            for fld in &v.fields {
                lint_annotation(
                    &fld.ty,
                    &format!(
                        "field `{}` of variant `{}` in enum `{}`",
                        fld.name, v.name, e.name
                    ),
                    &e.type_params,
                    &enums,
                    &structs,
                    file,
                    (0, 0),
                    &mut out,
                );
            }
        }
    }
    out
}

/// Lint one raw annotation string: closure-type spellings split first,
/// then the base name plus every generic argument is checked.
fn lint_annotation(
    raw: &str,
    where_: &str,
    type_params: &[String],
    enums: &[String],
    structs: &[String],
    file: &str,
    span: (usize, usize),
    out: &mut Vec<Diagnostic>,
) {
    // `fn(A, B) -> R` spellings name real parameter/return positions.
    if let Some((ps, r)) = crate::closures::split_closure_ty(raw) {
        for p in &ps {
            lint_annotation(p, where_, type_params, enums, structs, file, span, out);
        }
        lint_annotation(&r, where_, type_params, enums, structs, file, span, out);
        return;
    }
    let base = base_ty_name(raw);
    // Nominal bindings never narrow: type parameters, known structs/enums,
    // and qualified module paths (a different namespace entirely).
    let nominal = type_params.iter().any(|t| t == base)
        || enums.iter().any(|n| n == base)
        || structs.iter().any(|n| n == base)
        || base.contains("::");
    if !nominal {
        if matches!(base, "i64" | "u32" | "u64") {
            out.push(Diagnostic::warning(
                "W-TYPE-NARROW",
                &format!(
                    "{where_} is annotated `{base}`, but Klang enforces the `i32` range (-2147483648..2147483647)"
                ),
                file,
                span.0,
                span.1,
                "`i64`/`u32`/`u64` are aliases for the `i32`-checked int; real widths are a future design item",
                &["use `i32` to spell what is enforced"],
                "types/narrow",
            ));
        } else if base == "u8" {
            out.push(Diagnostic::warning(
                "W-TYPE-NARROW",
                &format!(
                    "{where_} is annotated `u8`, which is not a checked type (it stays dynamic)"
                ),
                file,
                span.0,
                span.1,
                "unknown type names stay `Unknown` (dynamic); only known annotations are checked",
                &["use `i32` for a checked integer"],
                "types/narrow",
            ));
        }
    }
    for arg in split_generic_args(raw) {
        lint_annotation(&arg, where_, type_params, enums, structs, file, span, out);
    }
}

/// Top-level `<...>` arguments of one annotation (`Opt<i64>` -> [`i64`]);
/// empty unless the annotation carries explicit generic arguments.
fn split_generic_args(raw: &str) -> Vec<String> {
    let bytes = raw.as_bytes();
    let Some(start) = raw.find('<') else {
        return Vec::new();
    };
    // Find the `<` matching `start` (annotations are parser-built, so a
    // match exists; bail out silently otherwise — no warning is safer
    // than a wrong one).
    let mut depth = 0;
    let mut end = None;
    for (i, b) in bytes.iter().enumerate().skip(start) {
        if *b == b'<' {
            depth += 1;
        } else if *b == b'>' {
            depth -= 1;
            if depth == 0 {
                end = Some(i);
                break;
            }
        }
    }
    let Some(end) = end else {
        return Vec::new();
    };
    let inner = &raw[start + 1..end];
    let mut args = Vec::new();
    let mut cur_depth = 0;
    let mut cur = String::new();
    for c in inner.chars() {
        match c {
            '<' => {
                cur_depth += 1;
                cur.push(c);
            }
            '>' => {
                cur_depth -= 1;
                cur.push(c);
            }
            ',' if cur_depth == 0 => {
                args.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        args.push(cur.trim().to_string());
    }
    args
}

/// Base nominal name of a possibly-generic annotation (`Opt<i32>` -> `Opt`,
/// `m::Box<T>` -> `m::Box`). Explicit arguments are validated at parse
/// time; nominal resolution keys on the base so `Opt` and `Opt<i32>` name
/// the same enum for exhaustiveness and call checking.
fn base_ty_name(s: &str) -> &str {
    match s.find('<') {
        Some(i) => &s[..i],
        None => s,
    }
}

/// Resolve a `fn(T, ...) -> R` closure annotation (`None` when `s` is
/// not one). Inner annotations resolve recursively, so nesting and
/// generic parameters work uniformly; anything malformed stays
/// `Unknown`, like every other unknown annotation.
fn resolve_closure_ty(
    s: &str,
    type_params: &[String],
    enums: &EnumTable,
    structs: &[String],
) -> Option<Ty> {
    let (ps, r) = crate::closures::split_closure_ty(s)?;
    Some(Ty::Closure {
        params: ps
            .iter()
            .map(|p| resolve_body_ty(p, type_params, enums, structs))
            .collect(),
        ret: Box::new(resolve_body_ty(&r, type_params, enums, structs)),
    })
}

/// Resolve a parameter annotation: known enum names become nominal
/// `Ty::Enum`, known struct names (including flattened `m::T` paths) become
/// nominal `Ty::Struct`, everything else keeps `parse_ty` behavior.
/// Unknown names stay `Unknown` (lenient, as before): only names the
/// checker actually knows become nominal, so a typo'd annotation never
/// produces a false `E-TYPE` — it simply stays dynamic.
fn resolve_param_ty(s: &str, enums: &EnumTable, structs: &[String]) -> Ty {
    if let Some(clos) = resolve_closure_ty(s, &[], enums, structs) {
        return clos;
    }
    let base = base_ty_name(s);
    if enums.contains_key(base) {
        Ty::Enum(base.to_string())
    } else if structs.iter().any(|n| n == base) {
        Ty::Struct(base.to_string())
    } else {
        parse_ty(base)
    }
}

/// Resolve a declared annotation inside a generic declaration: a name that
/// matches one of the declaration's own type parameters becomes a rigid
/// `Ty::Param`, everything else keeps the existing behavior.
fn resolve_generic_ty(
    s: &str,
    type_params: &[String],
    enums: &EnumTable,
    structs: &[String],
) -> Ty {
    if type_params.iter().any(|t| t == s) {
        Ty::Param(s.to_string())
    } else {
        resolve_param_ty(s, enums, structs)
    }
}

fn type_mismatch(file: &str, what: &str, want: &str, got: &Ty) -> Diagnostic {
    Diagnostic::error(
        "E-TYPE",
        &format!("{what}: want {want}, got {}", got.name()),
        file,
        0,
        0,
        "operand types do not match the operator",
        &["convert the value first", "check the operand types"],
        "types/mismatch",
    )
}

fn method_arity(file: &str, recv: &str, method: &str, want: usize, got: usize) -> Diagnostic {
    Diagnostic::error(
        "E-ARITY",
        &format!("{recv}.{method}() takes {want} arguments, got {got}"),
        file,
        0,
        0,
        "method arity must match",
        &["pass the right number of arguments"],
        "calls/arity",
    )
}

/// Generic-instantiation conflict: a type parameter was already fixed by a
/// prior argument/field and a later one disagrees. Names the parameter and
/// where it was bound so `klang repair` (and users) see the inference
/// story instead of a misleading hardcoded "want X".
fn generic_mismatch(
    file: &str,
    what: &str,
    param: &str,
    bound: &Ty,
    origin: &str,
    got: &Ty,
) -> Diagnostic {
    Diagnostic::error(
        "E-TYPE",
        &format!(
            "{what}: {param} was inferred as {} from {origin}, got {}",
            bound.name(),
            got.name()
        ),
        file,
        0,
        0,
        &format!(
            "generic type parameter `{param}` was already inferred as `{}` from {origin}",
            bound.name()
        ),
        &[
            "make the argument types agree",
            "check the generic instantiation",
        ],
        "types/mismatch",
    )
}

/// A fully effect-checked program.
#[derive(Debug, Clone)]
pub struct TypedHIR {
    pub program: Program,
}

#[derive(Debug, Clone)]
struct Sig {
    effects: Vec<Effect>,
    arity: usize,
    type_params: Vec<String>,
    param_tys: Vec<Ty>,
    return_ty: Ty,
}

impl TypedHIR {
    pub fn check(program: Program) -> Result<Self, Vec<Diagnostic>> {
        Self::check_with_file(program, FILE)
    }

    pub fn check_with_file(program: Program, file: &str) -> Result<Self, Vec<Diagnostic>> {
        // Modules resolve first: `mod` blocks flatten into qualified
        // top-level items (`m::f`) with references rewritten and visibility
        // enforced. Module-free programs come back identical.
        let (flat, mut diags) = crate::modules::resolve_with_file(&program, file);
        let program = flat;
        // D1 alias scopes for the unknown-alias hint: loader-filled when
        // imports were loaded, else derived from this file's own
        // `import ... as ...` lines (unresolved targets, raw paths).
        // Empty in alias-free programs (the common case, unchanged).
        let alias_scopes: Vec<crate::ast::AliasScope> = if program.alias_scopes.is_empty() {
            if program.aliased_imports.is_empty() {
                Vec::new()
            } else {
                vec![crate::ast::AliasScope {
                    file_idx: u32::MAX,
                    aliases: program
                        .aliased_imports
                        .iter()
                        .map(|a| crate::ast::AliasDecl {
                            alias: a.alias.clone(),
                            target_raw: a.path.clone(),
                            target_canon: String::new(),
                        })
                        .collect(),
                }]
            }
        } else {
            program.alias_scopes.clone()
        };
        // Flattened struct names up front (F10): enum payloads, call
        // signatures, and field types all resolve known structs nominally
        // instead of erasing them to `Unknown`. Names only — the field-type
        // table itself is still built in declaration order below.
        let struct_names_early: Vec<String> =
            program.structs.iter().map(|s| s.name.clone()).collect();
        // enum name -> variant name -> payload field types (built first so
        // parameter annotations can resolve to nominal enum types).
        let mut enums: EnumTable = HashMap::new();
        let mut enum_names: Vec<String> = Vec::new();
        // Pass 1 (FOUNDATION-3: recursive payloads): seed every enum name
        // with an empty variant map so self- and mutually-recursive
        // payloads (`Cons(x: i32, xs: List)`) resolve nominally instead of
        // silently erasing to `Unknown`. Names only — payloads still
        // resolve in declaration order below, so all non-recursive
        // behavior is unchanged.
        for e in &program.enums {
            enum_names.push(e.name.clone());
            check_type_params_dup(file, &e.name, &e.type_params, &mut diags);
            enums.entry(e.name.clone()).or_insert_with(HashMap::new);
        }
        // Pass 2: resolve variant payloads (now seeing all enum names,
        // including the enum being built and later-declared ones).
        let mut populated: std::collections::HashSet<String> = std::collections::HashSet::new();
        for e in &program.enums {
            let mut variants: HashMap<String, Vec<Ty>> = HashMap::new();
            for v in &e.variants {
                if variants.contains_key(&v.name) {
                    diags.push(duplicate(
                        file,
                        &format!("duplicate variant `{}` in enum `{}`", v.name, e.name),
                    ));
                }
                variants.insert(
                    v.name.clone(),
                    v.fields
                        .iter()
                        .map(|f| {
                            // Non-generic enums keep the exact historical resolution.
                            if e.type_params.is_empty() {
                                resolve_param_ty(&f.ty, &enums, &struct_names_early)
                            } else {
                                resolve_generic_ty(
                                    &f.ty,
                                    &e.type_params,
                                    &enums,
                                    &struct_names_early,
                                )
                            }
                        })
                        .collect(),
                );
            }
            if !populated.insert(e.name.clone()) {
                diags.push(duplicate(file, &format!("duplicate enum `{}`", e.name)));
            }
            enums.insert(e.name.clone(), variants);
        }
        enum_names.sort();
        enum_names.dedup();
        if enum_names.len() != program.enums.len() {
            diags.push(Diagnostic::error(
                "E-DUPLICATE",
                "duplicate enum name",
                file,
                0,
                0,
                "two enums share one name",
                &["rename one of them"],
                "names/duplicate",
            ));
        }
        let mut sigs: HashMap<String, Sig> = HashMap::new();
        for f in &program.functions {
            if sigs.contains_key(&f.name) {
                diags.push(duplicate(file, &format!("duplicate function `{}`", f.name)));
            }
            // A user definition sharing a builtin's name could never
            // run (builtins win every call dispatch): fail loudly with
            // W-SHADOW instead of letting it silently shadow.
            if is_builtin(&f.name) {
                diags.push(shadowed_builtin(file, &f.name));
            }
            check_type_params_dup(file, &f.name, &f.type_params, &mut diags);
            // Non-generic functions keep the exact historical resolution
            // (`parse_ty` on the return type); generic ones preserve `Param`.
            // Both resolve known structs/enums nominally (F10) so call-site
            // argument and return checking sees struct types instead of
            // `Unknown`.
            let return_ty = if f.type_params.is_empty() {
                resolve_param_ty(&f.return_ty, &enums, &struct_names_early)
            } else {
                resolve_generic_ty(&f.return_ty, &f.type_params, &enums, &struct_names_early)
            };
            sigs.insert(
                f.name.clone(),
                Sig {
                    effects: f.effects.clone(),
                    arity: f.params.len(),
                    type_params: f.type_params.clone(),
                    param_tys: f
                        .params
                        .iter()
                        .map(|p| {
                            resolve_generic_ty(&p.ty, &f.type_params, &enums, &struct_names_early)
                        })
                        .collect(),
                    return_ty,
                },
            );
        }
        // Closure pre-pass: every `fn(...) -> ...` literal gets a synthetic
        // signature (keyed by its NUL-containing lifted name, which source
        // can never spell) so bodies check their `return`s against the
        // literal's own declared type instead of the enclosing function's.
        // Collection recurses into nested closures, so deeply nested
        // literals are all registered before any body is checked.
        for f in &program.functions {
            let mut lits = Vec::new();
            crate::closures::collect_closures(&f.body, &mut lits);
            for lit in &lits {
                let cparams: Vec<Ty> = lit
                    .params
                    .iter()
                    .map(|p| {
                        resolve_body_ty(&p.ty, &f.type_params, &enums, &struct_names_early)
                    })
                    .collect();
                sigs.insert(
                    crate::closures::closure_fn_name(&lit.id),
                    Sig {
                        effects: Vec::new(),
                        arity: lit.params.len(),
                        type_params: f.type_params.clone(),
                        param_tys: cparams,
                        return_ty: resolve_body_ty(
                            &lit.return_ty,
                            &f.type_params,
                            &enums,
                            &struct_names_early,
                        ),
                    },
                );
            }
        }
        // struct name -> (field name -> field type)
        let mut struct_fields: HashMap<String, HashMap<String, Ty>> = HashMap::new();
        let mut struct_names: Vec<String> = Vec::new();
        for s in &program.structs {
            struct_names.push(s.name.clone());
            check_type_params_dup(file, &s.name, &s.type_params, &mut diags);
            let mut fields = HashMap::new();
            for fld in &s.fields {
                if fields.contains_key(&fld.name) {
                    diags.push(duplicate(
                        file,
                        &format!("duplicate field `{}` in struct `{}`", fld.name, s.name),
                    ));
                }
                // Reserve the enum runtime representation (`__variant`,
                // `f0`, `f1`, ...): user structs declaring these fields
                // could spoof or corrupt `match` dispatch.
                if fld.name == "__variant"
                    || fld.name.starts_with("__")
                    || (fld.name.len() >= 2
                        && fld.name.starts_with('f')
                        && fld.name[1..].chars().all(|c| c.is_ascii_digit()))
                {
                    diags.push(Diagnostic::error(
                        "E-RESERVED-FIELD",
                        &format!(
                            "field `{}` in struct `{}` uses reserved enum tag name",
                            fld.name, s.name
                        ),
                        file,
                        0,
                        0,
                        "enum tags live in `__variant` / `f0...` fields",
                        &["rename the field"],
                        "types/reserved",
                    ));
                }
                // Field types resolve exactly like any other annotation
                // (F10): type parameters stay rigid, known enums/structs go
                // nominal, primitives keep `parse_ty` behavior, unknown
                // names stay `Unknown`. Previously non-generic structs used
                // bare `parse_ty`, erasing even enum-typed fields and
                // disabling nested-literal, field-access, and
                // match-exhaustiveness checking through the field.
                let fty = resolve_generic_ty(&fld.ty, &s.type_params, &enums, &struct_names_early);
                fields.insert(fld.name.clone(), fty);
            }
            struct_fields.insert(s.name.clone(), fields);
        }
        struct_names.sort();
        struct_names.dedup();
        if struct_names.len() != program.structs.len() {
            diags.push(Diagnostic::error(
                "E-DUPLICATE",
                "duplicate struct name",
                file,
                0,
                0,
                "two structs share one name",
                &["rename one of them"],
                "names/duplicate",
            ));
        }
        let structs: Vec<String> = struct_names;
        for e in &program.enums {
            if struct_fields.contains_key(&e.name) {
                diags.push(duplicate(
                    file,
                    &format!("type `{}` declared as both enum and struct", e.name),
                ));
            }
        }
        for f in &program.functions {
            check_function(
                file,
                f,
                &sigs,
                &structs,
                &struct_fields,
                &enums,
                &mut diags,
                &alias_scopes,
            );
        }
        if diags.is_empty() {
            Ok(Self { program })
        } else {
            Err(diags)
        }
    }

    pub fn program(&self) -> &Program {
        &self.program
    }
}

fn has_effect(effects: &[Effect], want: Effect) -> bool {
    effects.contains(&want)
}

/// Resolve a parameter annotation inside a function body: type parameters
/// become rigid `Ty::Param`, known enums become nominal `Ty::Enum`, known
/// structs become nominal `Ty::Struct`, everything else keeps `parse_ty`
/// behavior. Previously struct names and type params erased to `Unknown`,
/// letting `x - 1` pass for generic `T` and `p.nonexistent` pass for
/// struct params.
fn resolve_body_ty(s: &str, type_params: &[String], enums: &EnumTable, structs: &[String]) -> Ty {
    if let Some(clos) = resolve_closure_ty(s, type_params, enums, structs) {
        return clos;
    }
    if type_params.iter().any(|t| t == s) {
        return Ty::Param(s.to_string());
    }
    let base = base_ty_name(s);
    if enums.contains_key(base) {
        return Ty::Enum(base.to_string());
    }
    if structs.iter().any(|n| n == base) {
        return Ty::Struct(base.to_string());
    }
    // Qualified `m::T` paths: resolve against known enums/structs (flattened
    // names like `m::Token`). Unknown qualified paths stay `Unknown` (lenient,
    // as before) to avoid false positives on function paths.
    if s.contains("::") {
        return Ty::Unknown;
    }
    parse_ty(base)
}

fn check_function(
    file: &str,
    f: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    enums: &EnumTable,
    diags: &mut Vec<Diagnostic>,
    alias_scopes: &[crate::ast::AliasScope],
) {
    let mut defined: HashMap<String, Ty> = HashMap::new();
    for p in &f.params {
        if defined.contains_key(&p.name) {
            diags.push(duplicate(
                file,
                &format!("duplicate parameter `{}` in `{}`", p.name, f.name),
            ));
        }
        defined.insert(
            p.name.clone(),
            resolve_body_ty(&p.ty, &f.type_params, enums, structs),
        );
    }
    let mut cx = Ctx::default();
    cx.alias_scopes = alias_scopes.to_vec();
    check_block(
        file,
        &f.body,
        f,
        sigs,
        structs,
        struct_fields,
        enums,
        diags,
        0,
        &mut Vec::new(),
        &mut defined,
        &mut cx,
    );
    // Effect gate: task-group/spawn/await require `async`. Only `throws`
    // was previously enforced; `async`/`cancel` were parsed but ignored.
    if fn_uses_tasks(f) && !has_effect(&f.effects, Effect::Async) {
        diags.push(Diagnostic::error(
            "E-EFFECT-MISMATCH",
            &format!("`{}` uses task_group/spawn/await without `async`", f.name),
            file,
            0,
            0,
            "task concurrency requires visible `async` effect in caller signature",
            &["declare `async` in the function signature"],
            "effects/visibility",
        ));
    }
}

#[derive(Debug, Clone)]
struct PendingSpawn {
    handle: String,
    span: (usize, usize),
}

#[derive(Debug, Clone, Default)]
struct Ctx {
    /// Per-file import aliases for the unknown-alias hint (D1). Empty in
    /// alias-free programs, so method-call checking is byte-identical
    /// unless the file actually uses `import ... as ...`.
    alias_scopes: Vec<crate::ast::AliasScope>,
    loop_depth: usize,
    /// `loop_depth` at each enclosing `try` entry, innermost last.
    /// `break`/`continue` inside a `try` must target a loop that also
    /// started inside it: slice execution cannot jump across slices.
    try_loop_depths: Vec<usize>,
    /// `loop_depth` at each enclosing multi-statement `match` arm entry,
    /// innermost last. Arm setup statements lower into the taken branch's
    /// own instruction run (fresh loop stack), so `break`/`continue`
    /// cannot cross an arm boundary — same discipline as `try` slices.
    match_arm_depths: Vec<usize>,
}

/// Walk a block. `group_stack` holds the pending-spawn list of each enclosing
/// group, innermost last. Returns the set of handles awaited in this block.
#[allow(clippy::too_many_arguments)]
fn check_block(
    file: &str,
    block: &Block,
    caller: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    enums: &EnumTable,
    diags: &mut Vec<Diagnostic>,
    depth: usize,
    group_stack: &mut Vec<Vec<PendingSpawn>>,
    defined: &mut HashMap<String, Ty>,
    cx: &mut Ctx,
) -> Vec<String> {
    let mut awaited_here: Vec<String> = Vec::new();
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let(l) => {
                // Only a DIRECT `let h = spawn f()` binds a trackable
                // handle. A `spawn` nested anywhere else (or a bare
                // `spawn f();` statement) leaves an unawaitable thread:
                // reject it here instead of leaking at runtime.
                let ty = if let Expr::Spawn { call, .. } = &l.value {
                    if group_stack.is_empty() {
                        diags.push(Diagnostic::spawn_outside_group(file, l.span.0, l.span.1));
                    } else {
                        // `spawn` runs a named function on its thread: a
                        // closure value carries snapshots the child frame
                        // cannot see, so spawning one is rejected here
                        // instead of failing inside the runtime.
                        if let Expr::Call { func, .. } = call.as_ref() {
                            if matches!(defined.get(func), Some(Ty::Closure { .. })) {
                                diags.push(Diagnostic::error(
                                    "E-TYPE",
                                    &format!("cannot `spawn` closure `{func}` (spawn takes a named function)"),
                                    file,
                                    0,
                                    0,
                                    "spawned tasks run without the defining scope",
                                    &["call the closure directly instead"],
                                    "types/mismatch",
                                ));
                            }
                        }
                        // Re-binding a live handle without awaiting it leaks
                        // the earlier thread at runtime (`pending.insert`
                        // overwrites). Reject unless the previous spawn was
                        // already awaited in this block.
                        let pending = group_stack.last().expect("group present");
                        if pending.iter().any(|p| p.handle == l.name)
                            && !awaited_here.contains(&l.name)
                        {
                            diags.push(Diagnostic::task_leak(file, l.span.0, l.span.1, &l.name));
                        } else if pending.iter().any(|p| p.handle == l.name) {
                            // Previous spawn was awaited: drop its entry so
                            // the group-exit check does not double-count.
                            let p = group_stack.last_mut().expect("group present");
                            p.retain(|x| x.handle != l.name);
                            awaited_here.retain(|x| x != &l.name);
                        }
                        group_stack
                            .last_mut()
                            .expect("group present")
                            .push(PendingSpawn {
                                handle: l.name.clone(),
                                span: l.span,
                            });
                    }
                    // `await` inside the spawned call itself may not use
                    // not-yet-bound handles; check outside-group as well.
                    if expr_contains_await(call) && group_stack.is_empty() {
                        diags.push(await_outside_group(file));
                    }
                    check_expr(
                        file,
                        call,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        enums,
                        diags,
                        depth,
                        &mut awaited_here,
                        defined,
                        group_stack,
                        cx,
                    )
                } else {
                    if expr_contains_await(&l.value) && group_stack.is_empty() {
                        diags.push(await_outside_group(file));
                    }
                    check_expr(
                        file,
                        &l.value,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        enums,
                        diags,
                        depth,
                        &mut awaited_here,
                        defined,
                        group_stack,
                        cx,
                    )
                };
                // `let x = spawn f()` binds the call's return type; `await x`
                // later yields the same type. Re-`let` shadows (insert, not
                // or_insert): the runtime rebinds, so the new type wins.
                defined.insert(l.name.clone(), ty);
            }
            Stmt::Assign(a) => {
                if expr_contains_await(&a.value) && group_stack.is_empty() {
                    diags.push(await_outside_group(file));
                }
                let target_ty = check_assign_target(
                    file,
                    &a.target,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
                let value_ty = check_expr(
                    file,
                    &a.value,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
                if let Some(want) = target_ty {
                    if !assignable(&value_ty, &want) {
                        diags.push(type_mismatch(file, "assignment", &want.name(), &value_ty));
                    }
                }
            }
            Stmt::Expr(e) => {
                if expr_has_spawn(e) && group_stack.is_empty() {
                    let (s, en) = expr_span_hint(e);
                    diags.push(Diagnostic::spawn_outside_group(file, s, en));
                }
                if expr_contains_await(e) && group_stack.is_empty() {
                    diags.push(await_outside_group(file));
                }
                check_expr(
                    file,
                    e,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
            }
            Stmt::Return(r) => {
                if expr_contains_await(&r.value) && group_stack.is_empty() {
                    diags.push(await_outside_group(file));
                }
                let got = check_expr(
                    file,
                    &r.value,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
                let want = sigs
                    .get(&caller.name)
                    .map(|s| s.return_ty.clone())
                    .unwrap_or(Ty::Unknown);
                if !assignable(&got, &want) {
                    // FOUNDATION-3 Part 2C enabler: the return statement
                    // carries a real byte span (`r.span`), so the
                    // mismatch points at the failing `return` instead of
                    // 0,0. Same code/message/cause/fixes/rule as
                    // `type_mismatch` — only the span is real. Other
                    // expression-level type errors remain 0,0 (stated
                    // limitation: expressions carry `NodeId`, not spans).
                    //
                    // D4: a value returned from a `void` function (declared
                    // `-> void` or with the arrow omitted) names the actual
                    // fix — declare the returned type — instead of the
                    // generic conversion hints, which make no sense when
                    // the callee promises no value at all.
                    let fixes: Vec<String> = match (&want, &got) {
                        (Ty::Void, Ty::Int | Ty::Float | Ty::Str | Ty::Bool) => vec![
                            format!(
                                "this function returns a value; add `-> {}`",
                                got.name()
                            ),
                            "or drop the `return` value and fall off the end".to_string(),
                        ],
                        (Ty::Void, Ty::Struct(_) | Ty::Enum(_) | Ty::Closure { .. }) => vec![
                            format!(
                                "this function returns a value; add `-> {}`",
                                got.name()
                            ),
                            "or drop the `return` value and fall off the end".to_string(),
                        ],
                        _ => vec![
                            "convert the value first".to_string(),
                            "check the operand types".to_string(),
                        ],
                    };
                    let fix_refs: Vec<&str> = fixes.iter().map(String::as_str).collect();
                    diags.push(Diagnostic::error(
                        "E-TYPE",
                        &format!("`{}` return: want {}, got {}", caller.name, want.name(), got.name()),
                        file,
                        r.span.0,
                        r.span.1,
                        "operand types do not match the operator",
                        &fix_refs,
                        "types/mismatch",
                    ));
                }
            }
            Stmt::Print(p) => {
                if expr_contains_await(&p.value) && group_stack.is_empty() {
                    diags.push(await_outside_group(file));
                }
                check_expr(
                    file,
                    &p.value,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
            }
            Stmt::Break(_) | Stmt::Continue(_) => {
                if cx.loop_depth == 0 {
                    diags.push(Diagnostic::error(
                        "E-LOOP",
                        "break/continue outside of any loop",
                        file,
                        0,
                        0,
                        "break and continue only make sense inside while/for",
                        &["move it inside a loop", "remove it"],
                        "control/loop",
                    ));
                } else if cx
                    .try_loop_depths
                    .last()
                    .is_some_and(|entry| cx.loop_depth <= *entry)
                {
                    // The target loop encloses the `try`: the jump would
                    // have to cross a slice boundary at runtime, which
                    // cannot work — restructure so the loop is inside.
                    diags.push(Diagnostic::error(
                        "E-LOOP",
                        "break/continue cannot cross a `try` boundary",
                        file,
                        0,
                        0,
                        "break and continue only jump within their own `try` region",
                        &["move the loop inside the `try`", "remove it"],
                        "control/loop",
                    ));
                } else if cx
                    .match_arm_depths
                    .last()
                    .is_some_and(|entry| cx.loop_depth <= *entry)
                {
                    // The target loop encloses the `match` arm: arm setup
                    // statements lower into the taken branch's own run, so
                    // the jump has no lowering to land on — restructure so
                    // the loop is inside the arm.
                    diags.push(Diagnostic::error(
                        "E-LOOP",
                        "break/continue cannot cross a match arm boundary",
                        file,
                        0,
                        0,
                        "break and continue only jump within their own match arm",
                        &["move the loop inside the arm", "remove it"],
                        "control/loop",
                    ));
                }
            }
            Stmt::While(w) => {
                if expr_contains_await(&w.cond) && group_stack.is_empty() {
                    diags.push(await_outside_group(file));
                }
                let cond_ty = check_expr(
                    file,
                    &w.cond,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
                require_condition(file, &cond_ty, "while", diags);
                cx.loop_depth += 1;
                let mut body_defined = defined.clone();
                let pending_before = group_stack.last().map(|v| v.len()).unwrap_or(0);
                let inner = check_block(
                    file,
                    &w.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut body_defined,
                    cx,
                );
                cx.loop_depth -= 1;
                // Soundness: a loop may execute zero times, and a handle
                // spawned in one iteration is a different thread each trip.
                // Spawns inside must be awaited inside the same body; awaits
                // inside never satisfy an outer group (no propagation).
                if let Some(pending) = group_stack.last() {
                    for p in pending.iter().skip(pending_before) {
                        if !inner.contains(&p.handle) {
                            diags.push(Diagnostic::task_leak(file, p.span.0, p.span.1, &p.handle));
                        }
                    }
                }
            }
            Stmt::ForRange(fr) => {
                let s_ty = check_expr(
                    file,
                    &fr.start,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
                let e_ty = check_expr(
                    file,
                    &fr.end,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
                for (ty, what) in [(&s_ty, "for range start"), (&e_ty, "for range end")] {
                    if !matches!(ty, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch(file, what, "i32", ty));
                    }
                }
                cx.loop_depth += 1;
                let mut body_defined = defined.clone();
                // Loop var rebinds at runtime (MIR `Copy`), shadowing any
                // outer binding of the same name.
                body_defined.insert(fr.var.clone(), Ty::Int);
                let pending_before = group_stack.last().map(|v| v.len()).unwrap_or(0);
                let inner = check_block(
                    file,
                    &fr.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut body_defined,
                    cx,
                );
                cx.loop_depth -= 1;
                if let Some(pending) = group_stack.last() {
                    for p in pending.iter().skip(pending_before) {
                        if !inner.contains(&p.handle) {
                            diags.push(Diagnostic::task_leak(file, p.span.0, p.span.1, &p.handle));
                        }
                    }
                }
            }
            Stmt::ForIn(fi) => {
                let iter_ty = check_expr(
                    file,
                    &fi.iter,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
                // Maps iterate over keys in insertion order (same order as
                // `keys()`): MIR lowers `for-in` to integer-indexed
                // iteration and an integer index on a map yields its
                // i-th key, so `for k in m` just works. This matches
                // SPEC §2 (`it` must be array/map/str); the `in` element
                // stays dynamic (`Ty::Unknown`, below) either way.
                match &iter_ty {
                    Ty::Array | Ty::Str | Ty::Map | Ty::Unknown => {}
                    other => diags.push(type_mismatch(
                        file,
                        "for-in iterable",
                        "array/map/str",
                        other,
                    )),
                }
                cx.loop_depth += 1;
                let mut body_defined = defined.clone();
                // Element type is dynamic without generics.
                body_defined.insert(fi.var.clone(), Ty::Unknown);
                let pending_before = group_stack.last().map(|v| v.len()).unwrap_or(0);
                let inner = check_block(
                    file,
                    &fi.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut body_defined,
                    cx,
                );
                cx.loop_depth -= 1;
                if let Some(pending) = group_stack.last() {
                    for p in pending.iter().skip(pending_before) {
                        if !inner.contains(&p.handle) {
                            diags.push(Diagnostic::task_leak(file, p.span.0, p.span.1, &p.handle));
                        }
                    }
                }
            }
            Stmt::TryCatch(t) => {
                // Task discipline mirrors loop bodies: spawns inside a
                // region must be awaited inside the same region; awaits
                // inside never satisfy an outer group (the handler may
                // not run, and the body may fail partway).
                cx.try_loop_depths.push(cx.loop_depth);
                let pending_before = group_stack.last().map(|v| v.len()).unwrap_or(0);
                let mut body_defined = defined.clone();
                let body_awaited = check_block(
                    file,
                    &t.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut body_defined,
                    cx,
                );
                if let Some(pending) = group_stack.last() {
                    for p in pending.iter().skip(pending_before) {
                        if !body_awaited.contains(&p.handle) {
                            diags.push(Diagnostic::task_leak(file, p.span.0, p.span.1, &p.handle));
                        }
                    }
                }
                let handler_before = group_stack.last().map(|v| v.len()).unwrap_or(0);
                let mut handler_defined = defined.clone();
                // The catch binding is a `{code, message}` map.
                handler_defined.insert(t.var.clone(), Ty::Map);
                let handler_awaited = check_block(
                    file,
                    &t.handler,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut handler_defined,
                    cx,
                );
                if let Some(pending) = group_stack.last() {
                    for p in pending.iter().skip(handler_before) {
                        if !handler_awaited.contains(&p.handle) {
                            diags.push(Diagnostic::task_leak(file, p.span.0, p.span.1, &p.handle));
                        }
                    }
                }
                cx.try_loop_depths.pop();
            }
            Stmt::If(s) => {
                if expr_contains_await(&s.cond) && group_stack.is_empty() {
                    diags.push(await_outside_group(file));
                }
                let cond_ty = check_expr(
                    file,
                    &s.cond,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    &mut awaited_here,
                    defined,
                    group_stack,
                    cx,
                );
                require_condition(file, &cond_ty, "if", diags);
                // Branches are block-scoped: new lets inside do not leak out.
                // Must-analysis: only awaits on every path satisfy an outer
                // spawn. Awaits in one branch alone never propagate.
                let pending_before = group_stack.last().map(|v| v.len()).unwrap_or(0);
                let mut then_defined = defined.clone();
                let then_awaited = check_block(
                    file,
                    &s.then_block,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth + 1,
                    group_stack,
                    &mut then_defined,
                    cx,
                );
                // Spawns inside the then-branch must be awaited in the same
                // branch; awaiting outside is unsound (branch may not run).
                if let Some(pending) = group_stack.last() {
                    // Only the spawns added by the then-branch.
                    let new_then: Vec<PendingSpawn> =
                        pending.iter().skip(pending_before).cloned().collect();
                    for p in &new_then {
                        if !then_awaited.contains(&p.handle) {
                            diags.push(Diagnostic::task_leak(file, p.span.0, p.span.1, &p.handle));
                        }
                    }
                    // Remove then-branch spawns from pending so the else
                    // branch check starts clean; re-add them after.
                    let mut saved = new_then;
                    let plen = group_stack.last().map(|v| v.len()).unwrap_or(0);
                    if plen >= saved.len() {
                        group_stack
                            .last_mut()
                            .expect("group")
                            .truncate(plen - saved.len());
                    }
                    if let Some(else_b) = &s.else_block {
                        let mut else_defined = defined.clone();
                        let else_before = group_stack.last().map(|v| v.len()).unwrap_or(0);
                        let else_awaited = check_block(
                            file,
                            else_b,
                            caller,
                            sigs,
                            structs,
                            struct_fields,
                            enums,
                            diags,
                            depth + 1,
                            group_stack,
                            &mut else_defined,
                            cx,
                        );
                        if let Some(pending2) = group_stack.last() {
                            for p in pending2.iter().skip(else_before) {
                                if !else_awaited.contains(&p.handle) {
                                    diags.push(Diagnostic::task_leak(
                                        file, p.span.0, p.span.1, &p.handle,
                                    ));
                                }
                            }
                            // Drop else-branch spawns (already diagnosed if
                            // unawaited) and restore then-branch spawns for
                            // outer must-analysis.
                            let elen = group_stack.last().map(|v| v.len()).unwrap_or(0);
                            if elen >= else_before {
                                group_stack.last_mut().expect("group").truncate(else_before);
                            }
                        }
                        for p in saved.drain(..) {
                            group_stack.last_mut().expect("group").push(p);
                        }
                        // Propagate only the intersection (must-await).
                        for h in then_awaited {
                            if else_awaited.contains(&h) && !awaited_here.contains(&h) {
                                awaited_here.push(h);
                            }
                        }
                    } else {
                        for p in saved.drain(..) {
                            group_stack.last_mut().expect("group").push(p);
                        }
                        // No else: nothing propagates (then may not run).
                    }
                } else if let Some(else_b) = &s.else_block {
                    let mut else_defined = defined.clone();
                    let else_awaited = check_block(
                        file,
                        else_b,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        enums,
                        diags,
                        depth + 1,
                        group_stack,
                        &mut else_defined,
                        cx,
                    );
                    for h in then_awaited {
                        if else_awaited.contains(&h) && !awaited_here.contains(&h) {
                            awaited_here.push(h);
                        }
                    }
                }
            }
            Stmt::TaskGroup(g) => {
                group_stack.push(Vec::new());
                let inner_awaited = check_block(
                    file,
                    &g.body,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth + 1,
                    group_stack,
                    defined,
                    cx,
                );
                let pending = group_stack.pop().expect("group just pushed");
                for p in &pending {
                    if !inner_awaited.contains(&p.handle) {
                        diags.push(Diagnostic::task_leak(file, p.span.0, p.span.1, &p.handle));
                    }
                }
                awaited_here.extend(inner_awaited);
            }
        }
    }
    awaited_here
}

/// `value` of type `got` flows into a slot of type `want`.
/// `Unknown` on either side is a wildcard (dynamic maps/indexes).
/// `Param` on either side is also a wildcard: a type variable's
/// instantiation is only known per call site, never in a generic body.
fn assignable(got: &Ty, want: &Ty) -> bool {
    if matches!(got, Ty::Unknown | Ty::Param(_)) || matches!(want, Ty::Unknown | Ty::Param(_)) {
        return true;
    }
    match (got, want) {
        (a, b) if a == b => true,
        // int coerces to float
        (Ty::Int, Ty::Float) => true,
        // Closures are structural: same arity with compatible
        // parameters and return type (component `Param`s hit the
        // wildcard above, staying dynamic).
        (
            Ty::Closure {
                params: gp,
                ret: gr,
            },
            Ty::Closure {
                params: wp,
                ret: wr,
            },
        ) => {
            gp.len() == wp.len()
                && gp.iter().zip(wp.iter()).all(|(g, w)| assignable(g, w))
                && assignable(gr, wr)
        }
        // struct names must match exactly (handled above)
        _ => false,
    }
}

fn require_condition(file: &str, ty: &Ty, what: &str, diags: &mut Vec<Diagnostic>) {
    match ty {
        Ty::Bool | Ty::Int | Ty::Unknown => {}
        other => diags.push(type_mismatch(
            file,
            &format!("{what} condition"),
            "bool",
            other,
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn check_assign_target(
    file: &str,
    t: &AssignTarget,
    caller: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    enums: &EnumTable,
    diags: &mut Vec<Diagnostic>,
    depth: usize,
    awaited: &mut Vec<String>,
    defined: &HashMap<String, Ty>,
    group_stack: &mut Vec<Vec<PendingSpawn>>,
    cx: &mut Ctx,
) -> Option<Ty> {
    match t {
        AssignTarget::Var { name } => {
            if let Some(ty) = defined.get(name) {
                Some(ty.clone())
            } else {
                diags.push(undefined(file, name));
                None
            }
        }
        AssignTarget::Index { base, index } => {
            let base_ty = check_expr(
                file,
                base,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let idx_ty = check_expr(
                file,
                index,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            match &base_ty {
                Ty::Array => {
                    if !matches!(idx_ty, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch(file, "array index", "i32", &idx_ty));
                    }
                    // Element type is dynamic without generics.
                    Some(Ty::Unknown)
                }
                Ty::Map => Some(Ty::Unknown),
                Ty::Str => {
                    if !matches!(idx_ty, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch(file, "string index", "i32", &idx_ty));
                    }
                    Some(Ty::Str)
                }
                Ty::Unknown => Some(Ty::Unknown),
                other => {
                    diags.push(type_mismatch(file, "index base", "array/map/str", other));
                    Some(Ty::Unknown)
                }
            }
        }
        AssignTarget::Field { base, field } => {
            let base_ty = check_expr(
                file,
                base,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            match &base_ty {
                Ty::Struct(name) => {
                    if let Some(fields) = struct_fields.get(name) {
                        if let Some(ty) = fields.get(field) {
                            Some(ty.clone())
                        } else {
                            diags.push(undefined(file, field));
                            None
                        }
                    } else {
                        Some(Ty::Unknown)
                    }
                }
                Ty::Unknown => Some(Ty::Unknown),
                other => {
                    diags.push(type_mismatch(file, "field base", "struct", other));
                    Some(Ty::Unknown)
                }
            }
        }
    }
}

fn duplicate(file: &str, message: &str) -> Diagnostic {
    Diagnostic::error(
        "E-DUPLICATE",
        message,
        file,
        0,
        0,
        "two items share one name",
        &["rename one of them"],
        "names/duplicate",
    )
}

/// A user-defined function reuses a builtin name. Builtins win every
/// call dispatch, so the definition could never execute.
///
/// Coded `W-SHADOW` (warning lineage) but surfaced as an error: the
/// diagnostics model has no passing-with-warnings channel (`check`
/// fails on any diagnostic; LSP/CLI treat every check diagnostic as a
/// failure), so an error is the only way to guarantee a builtin never
/// silently overrides user code.
fn shadowed_builtin(file: &str, name: &str) -> Diagnostic {
    Diagnostic::error(
        "W-SHADOW",
        &format!(
            "`{name}` shadows a builtin (builtins win every call; this definition would never run)"
        ),
        file,
        0,
        0,
        "user functions cannot reuse builtin names",
        &["rename the function"],
        "names/shadow",
    )
}

fn undefined(file: &str, name: &str) -> Diagnostic {
    Diagnostic::error(
        "E-UNDEFINED",
        &format!("undefined variable `{name}`"),
        file,
        0,
        0,
        "name is not a parameter or a prior `let` binding",
        &["bind it with `let` first", "check the spelling"],
        "names/scope",
    )
}

/// The file-alias scope enclosing `caller`, if the program uses D1 file
/// aliases and the caller's file can be identified: the exact file index
/// for loader-merged programs (NodeId prefix), or the single scope of a
/// single-file program. `None` keeps method-call checking byte-identical.
fn alias_scope_for<'a>(
    scopes: &'a [crate::ast::AliasScope],
    caller: &FunctionDecl,
) -> Option<&'a crate::ast::AliasScope> {
    if scopes.is_empty() {
        return None;
    }
    if let Some(&idx) = caller.id.path.first() {
        if let Some(s) = scopes.iter().find(|s| s.file_idx == idx) {
            return Some(s);
        }
    }
    if scopes.len() == 1 {
        return Some(&scopes[0]);
    }
    None
}

fn arity_mismatch(file: &str, caller: &str, callee: &str, want: usize, got: usize) -> Diagnostic {
    Diagnostic::error(
        "E-ARITY",
        &format!("`{caller}` calls `{callee}` with {got} args, want {want}"),
        file,
        0,
        0,
        "call arity must match the callee parameter list",
        &["pass the right number of arguments"],
        "calls/arity",
    )
}

/// Non-exhaustive `match`: a real diagnostic naming every missing variant.
fn match_exhaustive(file: &str, enum_name: &str, missing: &[String]) -> Diagnostic {
    let want: Vec<String> = missing
        .iter()
        .map(|m| format!("{enum_name}::{m}"))
        .collect();
    Diagnostic::error(
        "E-MATCH-EXHAUSTIVE",
        &format!(
            "non-exhaustive match on `{enum_name}`: missing {}",
            want.join(", ")
        ),
        file,
        0,
        0,
        &format!("match on `{enum_name}` does not cover all variants"),
        &[
            "add arms for the missing variants",
            "add a wildcard `_` arm",
        ],
        "match/exhaustiveness",
    )
}

fn enum_payload_arity(
    file: &str,
    enum_name: &str,
    variant: &str,
    want: usize,
    got: usize,
) -> Diagnostic {
    Diagnostic::error(
        "E-ARITY",
        &format!("`{enum_name}::{variant}` carries {want} payloads, got {got}"),
        file,
        0,
        0,
        "enum constructor arity must match the variant declaration",
        &["pass the right number of payload values"],
        "calls/arity",
    )
}

/// Struct literal omitting declared fields (F9): every declared field must
/// be present. Previously only provided fields were validated (unknown
/// names, value types), so `P { x: 1 }` for a two-field `P` checked clean
/// and failed only at runtime with `E-RUNTIME unknown struct field`.
fn struct_literal_missing(file: &str, struct_name: &str, missing: &[String]) -> Diagnostic {
    let fields: Vec<String> = missing
        .iter()
        .map(|m| format!("`{struct_name}.{m}`"))
        .collect();
    let noun = if missing.len() == 1 {
        "field"
    } else {
        "fields"
    };
    Diagnostic::error(
        "E-ARITY",
        &format!(
            "`{struct_name}` literal is missing {noun} {}",
            fields.join(", ")
        ),
        file,
        0,
        0,
        "struct literals must provide every declared field",
        &["add the missing fields"],
        "calls/arity",
    )
}

fn match_binding_arity(
    file: &str,
    enum_name: &str,
    variant: &str,
    want: usize,
    got: usize,
) -> Diagnostic {
    Diagnostic::error(
        "E-ARITY",
        &format!(
            "pattern `{enum_name}::{variant}` binds {got} names but the variant carries {want}"
        ),
        file,
        0,
        0,
        "match bindings must match the variant payload",
        &["bind exactly the payload fields"],
        "match/bindings",
    )
}

/// Unify one formal (possibly containing `Ty::Param`) against an actual
/// argument type, extending `subst`. `subst` maps each type parameter to
/// its inferred concrete type plus where it was bound (e.g. "arg 0" or
/// "`Pair.first`"), so a later conflict can name both. `subst` is always
/// fresh per call or construction site, so instantiations never leak
/// across sites. Returns false after pushing an `E-TYPE` diagnostic on
/// conflict.
fn unify_generic(
    file: &str,
    formal: &Ty,
    actual: &Ty,
    subst: &mut HashMap<String, (Ty, String)>,
    what: &str,
    origin: &str,
    diags: &mut Vec<Diagnostic>,
) -> bool {
    match formal {
        Ty::Param(p) => match actual {
            // Dynamic actuals constrain nothing; identical variables agree.
            Ty::Unknown | Ty::Param(_) => true,
            _ => match subst.get(p) {
                None => {
                    subst.insert(p.clone(), (actual.clone(), origin.to_string()));
                    true
                }
                Some((bound, prior_origin)) => {
                    if assignable(actual, bound) {
                        true
                    } else {
                        diags.push(generic_mismatch(file, what, p, bound, prior_origin, actual));
                        false
                    }
                }
            },
        },
        _ => {
            if assignable(actual, formal) {
                true
            } else {
                diags.push(type_mismatch(file, what, &formal.name(), actual));
                false
            }
        }
    }
}

/// Substitute inferred bindings into a generic return type. Unbound
/// variables (nothing constrained them) become `Unknown`, never an error.
fn substitute_ty(ty: &Ty, subst: &HashMap<String, (Ty, String)>) -> Ty {
    match ty {
        Ty::Param(p) => subst.get(p).cloned().map(|(t, _)| t).unwrap_or(Ty::Unknown),
        Ty::Closure { params, ret } => Ty::Closure {
            params: params.iter().map(|p| substitute_ty(p, subst)).collect(),
            ret: Box::new(substitute_ty(ret, subst)),
        },
        _ => ty.clone(),
    }
}

/// Reject duplicated type-parameter names (`fn f<T, T>`).
fn check_type_params_dup(
    file: &str,
    owner: &str,
    type_params: &[String],
    diags: &mut Vec<Diagnostic>,
) {
    let mut seen: Vec<&String> = Vec::new();
    for t in type_params {
        if seen.contains(&t) {
            diags.push(duplicate(
                file,
                &format!("duplicate type parameter `{t}` in `{owner}`"),
            ));
        } else {
            seen.push(t);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_expr(
    file: &str,
    e: &Expr,
    caller: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    enums: &EnumTable,
    diags: &mut Vec<Diagnostic>,
    _depth: usize,
    awaited: &mut Vec<String>,
    defined: &HashMap<String, Ty>,
    group_stack: &mut Vec<Vec<PendingSpawn>>,
    cx: &mut Ctx,
) -> Ty {
    match e {
        Expr::Int { value, .. } => {
            // `run` narrows to i32: reject literals that would wrap.
            if *value > i32::MAX as i64 {
                diags.push(type_mismatch(
                    file,
                    "integer literal",
                    "i32 range",
                    &Ty::Int,
                ));
                return Ty::Unknown;
            }
            Ty::Int
        }
        Expr::Float { .. } => Ty::Float,
        Expr::Str { .. } => Ty::Str,
        Expr::Bool { .. } => Ty::Bool,
        Expr::Spawn { call, .. } => {
            // Reached only for non-direct positions (`let`-direct is
            // handled by the caller): an unawaitable thread. Reject.
            diags.push(Diagnostic::spawn_position(file));
            check_expr(
                file,
                call,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            )
        }
        Expr::Await { name, .. } => {
            // `await` of an undefined handle is never diagnosed previously:
            // `fn main() -> i32 { return await b }` checked clean and failed
            // only at runtime. Emit E-UNDEFINED here (the task-leak check
            // still fires separately when inside a group).
            if !defined.contains_key(name) {
                diags.push(undefined(file, name));
            }
            if !awaited.contains(name) {
                awaited.push(name.clone());
            }
            defined.get(name).cloned().unwrap_or(Ty::Unknown)
        }
        Expr::Call {
            func,
            type_args,
            args,
            ..
        } => {
            let mut arg_tys = Vec::with_capacity(args.len());
            for a in args {
                arg_tys.push(check_expr(
                    file,
                    a,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    _depth,
                    awaited,
                    defined,
                    group_stack,
                    cx,
                ));
            }
            if is_builtin(func) {
                if !type_args.is_empty() {
                    diags.push(Diagnostic::error(
                        "E-ARITY",
                        &format!(
                            "`{func}` takes no explicit type arguments, got {}",
                            type_args.len()
                        ),
                        file,
                        0,
                        0,
                        "builtin functions are not generic",
                        &["remove the explicit type arguments"],
                        "calls/arity",
                    ));
                    return Ty::Unknown;
                }
                // `push`/`pop`/`insert` mutating a temporary (`push([1,2], 3)`,
                // `[1,2].push(3)`) silently no-ops at runtime (mutates a
                // discarded temp). Require a variable receiver.
                if (func == "push" || func == "pop" || func == "insert") && !args.is_empty() {
                    match &args[0] {
                        Expr::Var { .. } => {}
                        _ => {
                            diags.push(Diagnostic::error(
                                "E-TYPE",
                                &format!("`{func}()` must mutate a variable, not a temporary"),
                                file,
                                0,
                                0,
                                "mutating a temporary value is discarded",
                                &["bind the array to a variable first"],
                                "types/mutation",
                            ));
                        }
                    }
                }
                // FOUNDATION-3 Part 2A: simulated-async network calls
                // require the `async` effect (E-EFFECT-MISMATCH
                // otherwise). The pool parks the caller, so calling from
                // a sync fn would silently serialize group drains; the
                // gate makes async usage visible in the signature.
                // Fan-out helpers spawned from within a task_group
                // satisfy this via their own `async` annotation —
                // lexical group containment is NOT required at the call
                // site, since a spawned helper runs within the group's
                // lifetime dynamically. `cancel` stays unchecked.
                if (func == "http_get_async" || func == "http_post_async")
                    && !has_effect(&caller.effects, Effect::Async)
                {
                    let (s, en) = caller.name_span;
                    diags.push(Diagnostic::error(
                        "E-EFFECT-MISMATCH",
                        &format!("`{}` calls `{func}` without `async`", caller.name),
                        file,
                        s,
                        en,
                        "simulated-async network calls require visible `async` effect in caller signature",
                        &["declare `async` in the function signature"],
                        "effects/visibility",
                    ));
                }
                return check_builtin_call(file, func, &arg_tys, diags);
            }
            // A closure-typed binding dispatches dynamically at runtime
            // (values-first: it shadows any same-named function, exactly
            // as it executes — see `MirOp::Call`). Checked before static
            // signatures; only closure values take this path, so every
            // pre-closure program checks exactly as before.
            if let Some(Ty::Closure { params, ret }) = defined.get(func) {
                if !type_args.is_empty() {
                    diags.push(Diagnostic::error(
                        "E-ARITY",
                        &format!(
                            "closure `{func}` takes no explicit type arguments, got {}",
                            type_args.len()
                        ),
                        file,
                        0,
                        0,
                        "closures are not generic",
                        &["remove the explicit type arguments"],
                        "calls/arity",
                    ));
                    return Ty::Unknown;
                }
                if args.len() != params.len() {
                    diags.push(arity_mismatch(
                        file,
                        &caller.name,
                        func,
                        params.len(),
                        args.len(),
                    ));
                    return Ty::Unknown;
                }
                for (i, (got, want)) in arg_tys.iter().zip(params.iter()).enumerate() {
                    if !assignable(got, want) {
                        diags.push(type_mismatch(
                            file,
                            &format!("`{func}` arg {i}"),
                            &want.name(),
                            got,
                        ));
                    }
                }
                return ret.as_ref().clone();
            }
            if let Some(sig) = sigs.get(func) {
                if has_effect(&sig.effects, Effect::Throws)
                    && !has_effect(&caller.effects, Effect::Throws)
                {
                    let (s, en) = caller.name_span;
                    diags.push(Diagnostic::effect_mismatch(
                        file,
                        s,
                        en,
                        &caller.name,
                        func,
                        "throws",
                    ));
                }
                if args.len() != sig.arity {
                    diags.push(arity_mismatch(
                        file,
                        &caller.name,
                        func,
                        sig.arity,
                        args.len(),
                    ));
                    return Ty::Unknown;
                }
                if sig.type_params.is_empty() {
                    if !type_args.is_empty() {
                        diags.push(Diagnostic::error(
                            "E-ARITY",
                            &format!(
                                "`{func}` takes no explicit type arguments, got {}",
                                type_args.len()
                            ),
                            file,
                            0,
                            0,
                            "function is not generic",
                            &["remove the explicit type arguments"],
                            "calls/arity",
                        ));
                        return Ty::Unknown;
                    }
                    for (i, (got, want)) in arg_tys.iter().zip(sig.param_tys.iter()).enumerate() {
                        if !assignable(got, want) {
                            diags.push(type_mismatch(
                                file,
                                &format!("`{func}` arg {i}"),
                                &want.name(),
                                got,
                            ));
                        }
                    }
                    sig.return_ty.clone()
                } else {
                    // Generic call: explicit `<T, ...>` arguments seed the
                    // substitution when present; otherwise infer fresh from
                    // the value arguments. Either way each value argument
                    // then unifies against the substitution, so conflicts
                    // name the parameter and its binding site (F13).
                    let mut subst: HashMap<String, (Ty, String)> = HashMap::new();
                    if !type_args.is_empty() {
                        if type_args.len() != sig.type_params.len() {
                            diags.push(Diagnostic::error(
                                "E-ARITY",
                                &format!(
                                    "`{func}` takes {} explicit type arguments, got {}",
                                    sig.type_params.len(),
                                    type_args.len()
                                ),
                                file,
                                0,
                                0,
                                "explicit generic arity must match the declaration",
                                &["pass the right number of type arguments"],
                                "calls/arity",
                            ));
                            return Ty::Unknown;
                        }
                        for (i, (param, arg_str)) in
                            sig.type_params.iter().zip(type_args.iter()).enumerate()
                        {
                            let resolved =
                                resolve_body_ty(arg_str, &caller.type_params, enums, structs);
                            subst.insert(
                                param.clone(),
                                (resolved, format!("explicit type argument {i}")),
                            );
                        }
                    }
                    let mut ok = true;
                    for (i, (got, want)) in arg_tys.iter().zip(sig.param_tys.iter()).enumerate() {
                        if !unify_generic(
                            file,
                            want,
                            got,
                            &mut subst,
                            &format!("`{func}` arg {i}"),
                            &format!("arg {i}"),
                            diags,
                        ) {
                            ok = false;
                        }
                    }
                    if ok {
                        substitute_ty(&sig.return_ty, &subst)
                    } else {
                        Ty::Unknown
                    }
                }
            } else if let Some(other) = defined.get(func) {
                // A bound non-closure variable is not callable: reject here
                // instead of failing at runtime.
                diags.push(Diagnostic::error(
                    "E-TYPE",
                    &format!("`{func}` is {}, not callable", other.name()),
                    file,
                    0,
                    0,
                    "only functions and closures can be called",
                    &["call a function or closure value instead"],
                    "types/mismatch",
                ));
                Ty::Unknown
            } else {
                diags.push(undefined(file, func));
                Ty::Unknown
            }
        }
        Expr::Var { name, .. } => {
            if let Some(ty) = defined.get(name) {
                ty.clone()
            } else {
                diags.push(undefined(file, name));
                Ty::Unknown
            }
        }
        Expr::ArrayLit { elems, .. } => {
            for el in elems {
                check_expr(
                    file,
                    el,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    _depth,
                    awaited,
                    defined,
                    group_stack,
                    cx,
                );
            }
            Ty::Array
        }
        Expr::StructLit { name, fields, .. } => {
            if !structs.contains(name) {
                diags.push(undefined(file, name));
                for (_, v) in fields {
                    check_expr(
                        file,
                        v,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        enums,
                        diags,
                        _depth,
                        awaited,
                        defined,
                        group_stack,
                        cx,
                    );
                }
                return Ty::Unknown;
            }
            if let Some(known) = struct_fields.get(name) {
                // Fresh substitution per literal: each construction site
                // instantiates the struct's type parameters independently.
                let mut subst: HashMap<String, (Ty, String)> = HashMap::new();
                for (fname, v) in fields {
                    let vty = check_expr(
                        file,
                        v,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        enums,
                        diags,
                        _depth,
                        awaited,
                        defined,
                        group_stack,
                        cx,
                    );
                    if let Some(want) = known.get(fname) {
                        let loc = format!("`{name}.{fname}`");
                        unify_generic(file, want, &vty, &mut subst, &loc, &loc, diags);
                    } else {
                        diags.push(undefined(file, fname));
                    }
                }
                // F9: omitted fields are a compile-time error, not a
                // runtime `E-RUNTIME`. Extra fields already report
                // `E-UNDEFINED` above; silence on the missing side was the
                // hole.
                let mut missing: Vec<String> = known
                    .keys()
                    .filter(|k| !fields.iter().any(|(n, _)| n == *k))
                    .cloned()
                    .collect();
                missing.sort();
                if !missing.is_empty() {
                    diags.push(struct_literal_missing(file, name, &missing));
                }
            } else {
                for (_, v) in fields {
                    check_expr(
                        file,
                        v,
                        caller,
                        sigs,
                        structs,
                        struct_fields,
                        enums,
                        diags,
                        _depth,
                        awaited,
                        defined,
                        group_stack,
                        cx,
                    );
                }
            }
            Ty::Struct(name.clone())
        }
        Expr::Index { base, index, .. } => {
            let b = check_expr(
                file,
                base,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let idx = check_expr(
                file,
                index,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            match &b {
                Ty::Array => {
                    if !matches!(idx, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch(file, "array index", "i32", &idx));
                    }
                    Ty::Unknown
                }
                Ty::Map => Ty::Unknown,
                Ty::Str => {
                    if !matches!(idx, Ty::Int | Ty::Unknown) {
                        diags.push(type_mismatch(file, "string index", "i32", &idx));
                    }
                    Ty::Str
                }
                Ty::Unknown => Ty::Unknown,
                other => {
                    diags.push(type_mismatch(file, "index base", "array/map/str", other));
                    Ty::Unknown
                }
            }
        }
        Expr::MapLit { entries, .. } => {
            for (_, v) in entries {
                check_expr(
                    file,
                    v,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    _depth,
                    awaited,
                    defined,
                    group_stack,
                    cx,
                );
            }
            Ty::Map
        }
        Expr::Field { base, field, .. } => {
            let b = check_expr(
                file,
                base,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            match &b {
                Ty::Struct(name) => {
                    if let Some(fields) = struct_fields.get(name) {
                        if let Some(ty) = fields.get(field) {
                            // A generic field's instantiation is unknown at
                            // the use site (instantiations are per literal),
                            // so it stays dynamic instead of constraining
                            // operators downstream.
                            match ty {
                                Ty::Param(_) => Ty::Unknown,
                                _ => ty.clone(),
                            }
                        } else {
                            diags.push(undefined(file, field));
                            Ty::Unknown
                        }
                    } else {
                        Ty::Unknown
                    }
                }
                Ty::Unknown => Ty::Unknown,
                other => {
                    diags.push(type_mismatch(file, "field base", "struct", other));
                    Ty::Unknown
                }
            }
        }
        Expr::MethodCall {
            base, method, args, ..
        } => {
            // D1: `n.f(...)` where `n` is bound to nothing and the file
            // imports aliases names the available aliases, so a misspelled
            // or missing alias is a clear error with a suggestion. (A
            // DECLARED alias never reaches the checker — the loader
            // rewrites `m.f` to a plain call.)
            if let Expr::Var { name: recv, .. } = base.as_ref() {
                if !defined.contains_key(recv) {
                    // Borrow ends before the mutable args walk below.
                    let suggestion: Option<String> =
                        alias_scope_for(&cx.alias_scopes, caller)
                            .filter(|s| !s.aliases.is_empty())
                            .map(|s| {
                                format!(
                                    "available file aliases: {}",
                                    s.aliases
                                        .iter()
                                        .map(|a| format!(
                                            "`{}` (from \"{}\")",
                                            a.alias, a.target_raw
                                        ))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            });
                    if let Some(sugg) = suggestion {
                        let mut arg_tys = Vec::with_capacity(args.len());
                        for a in args {
                            arg_tys.push(check_expr(
                                file,
                                a,
                                caller,
                                sigs,
                                structs,
                                struct_fields,
                                enums,
                                diags,
                                _depth,
                                awaited,
                                defined,
                                group_stack,
                                cx,
                            ));
                        }
                        let fixes =
                            vec![
                                "bind the receiver with `let` first".to_string(),
                                "check the spelling".to_string(),
                                sugg,
                            ];
                        let fix_refs: Vec<&str> =
                            fixes.iter().map(String::as_str).collect();
                        diags.push(Diagnostic::error(
                            "E-UNDEFINED",
                            &format!("undefined variable `{recv}`"),
                            file,
                            0,
                            0,
                            "name is not a parameter or a prior `let` binding",
                            &fix_refs,
                            "names/scope",
                        ));
                        return check_method_call(file, &Ty::Unknown, method, &arg_tys, diags);
                    }
                }
            }
            let b = check_expr(
                file,
                base,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let mut arg_tys = Vec::with_capacity(args.len());
            for a in args {
                arg_tys.push(check_expr(
                    file,
                    a,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    _depth,
                    awaited,
                    defined,
                    group_stack,
                    cx,
                ));
            }
            if method == "push" || method == "pop" || method == "insert" {
                // `arr.push`/`arr.pop`/`arr.insert` mutate in place: require
                // a variable receiver, not a temporary (`[1,2].push(3)`).
                match base.as_ref() {
                    Expr::Var { .. } => {}
                    _ => {
                        diags.push(Diagnostic::error(
                            "E-TYPE",
                            &format!("`.{method}()` must mutate a variable, not a temporary"),
                            file,
                            0,
                            0,
                            "mutating a temporary value is discarded",
                            &["bind the array to a variable first"],
                            "types/mutation",
                        ));
                    }
                }
            }
            check_method_call(file, &b, method, &arg_tys, diags)
        }
        Expr::Add { left, right, .. } => {
            let l = check_expr(
                file,
                left,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let r = check_expr(
                file,
                right,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            match (&l, &r) {
                (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                (Ty::Int, Ty::Int) => Ty::Int,
                (Ty::Float, Ty::Float) | (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Ty::Float,
                (Ty::Str, _) | (_, Ty::Str) => Ty::Str,
                _ => {
                    diags.push(type_mismatch(file, "`+` operands", "i32/f64/str", &l));
                    Ty::Unknown
                }
            }
        }
        Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. } => {
            let op = match e {
                Expr::Sub { .. } => "`-`",
                Expr::Mul { .. } => "`*`",
                Expr::Div { .. } => "`/`",
                _ => "`%`",
            };
            let l = check_expr(
                file,
                left,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let r = check_expr(
                file,
                right,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            if matches!(e, Expr::Mod { .. }) {
                // `%` is integers only.
                let ok = matches!(
                    (&l, &r),
                    (Ty::Unknown, _) | (_, Ty::Unknown) | (Ty::Int, Ty::Int)
                );
                if !ok {
                    diags.push(type_mismatch(file, "`%` operands", "i32", &l));
                }
                return Ty::Int;
            }
            match (&l, &r) {
                (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                (Ty::Int, Ty::Int) => Ty::Int,
                (Ty::Float, Ty::Float) | (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Ty::Float,
                _ => {
                    diags.push(type_mismatch(
                        file,
                        &format!("{op} operands"),
                        "i32/f64",
                        &l,
                    ));
                    Ty::Unknown
                }
            }
        }
        Expr::Eq { left, right, .. } | Expr::NotEq { left, right, .. } => {
            let l = check_expr(
                file,
                left,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let r = check_expr(
                file,
                right,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            if !equality_ok(&l, &r) {
                diags.push(type_mismatch(file, "`==` operands", &l.name(), &r));
            }
            Ty::Bool
        }
        Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. } => {
            let l = check_expr(
                file,
                left,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let r = check_expr(
                file,
                right,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let ok = matches!(
                (&l, &r),
                (Ty::Unknown, _)
                    | (_, Ty::Unknown)
                    | (Ty::Int, Ty::Int)
                    | (Ty::Float, Ty::Float)
                    | (Ty::Int, Ty::Float)
                    | (Ty::Float, Ty::Int)
                    | (Ty::Str, Ty::Str)
            );
            if !ok {
                diags.push(type_mismatch(
                    file,
                    "comparison operands",
                    "i32/f64/str",
                    &l,
                ));
            }
            Ty::Bool
        }
        Expr::And { left, right, .. } | Expr::Or { left, right, .. } => {
            let l = check_expr(
                file,
                left,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            let r = check_expr(
                file,
                right,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            for (ty, what) in [(&l, "left `&&`/`||`"), (&r, "right `&&`/`||`")] {
                if !matches!(ty, Ty::Bool | Ty::Int | Ty::Unknown) {
                    diags.push(type_mismatch(file, what, "bool", ty));
                }
            }
            Ty::Bool
        }
        Expr::Not { inner, .. } => {
            let t = check_expr(
                file,
                inner,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            if !matches!(t, Ty::Bool | Ty::Int | Ty::Unknown) {
                diags.push(type_mismatch(file, "`!` operand", "bool", &t));
            }
            Ty::Bool
        }
        Expr::Neg { inner, .. } => {
            // `-2147483648` is valid i32::MIN although the positive
            // literal alone exceeds the range: check the negated value.
            if let Expr::Int { value, .. } = inner.as_ref() {
                if *value > 2147483648 {
                    diags.push(type_mismatch(
                        file,
                        "integer literal",
                        "i32 range",
                        &Ty::Int,
                    ));
                    return Ty::Unknown;
                }
                return Ty::Int;
            }
            let t = check_expr(
                file,
                inner,
                caller,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                _depth,
                awaited,
                defined,
                group_stack,
                cx,
            );
            match &t {
                Ty::Int => Ty::Int,
                Ty::Float => Ty::Float,
                Ty::Unknown => Ty::Unknown,
                other => {
                    diags.push(type_mismatch(file, "unary `-` operand", "i32/f64", other));
                    Ty::Unknown
                }
            }
        }
        Expr::EnumCtor {
            enum_name,
            variant,
            args,
            ..
        } => {
            let payload: Option<Vec<Ty>> =
                enums.get(enum_name).and_then(|vs| vs.get(variant)).cloned();
            match payload {
                None => {
                    if enums.contains_key(enum_name) {
                        diags.push(undefined(file, &format!("{enum_name}::{variant}")));
                    } else {
                        diags.push(undefined(file, enum_name));
                    }
                    for a in args {
                        check_expr(
                            file,
                            a,
                            caller,
                            sigs,
                            structs,
                            struct_fields,
                            enums,
                            diags,
                            _depth,
                            awaited,
                            defined,
                            group_stack,
                            cx,
                        );
                    }
                    Ty::Unknown
                }
                Some(wants) => {
                    if args.len() != wants.len() {
                        diags.push(enum_payload_arity(
                            file,
                            enum_name,
                            variant,
                            wants.len(),
                            args.len(),
                        ));
                    }
                    // Fresh substitution per construction site.
                    let mut subst: HashMap<String, (Ty, String)> = HashMap::new();
                    for (idx, (a, want)) in args.iter().zip(wants.iter()).enumerate() {
                        let got = check_expr(
                            file,
                            a,
                            caller,
                            sigs,
                            structs,
                            struct_fields,
                            enums,
                            diags,
                            _depth,
                            awaited,
                            defined,
                            group_stack,
                            cx,
                        );
                        unify_generic(
                            file,
                            want,
                            &got,
                            &mut subst,
                            &format!("`{enum_name}::{variant}` payload {idx}"),
                            &format!("payload {idx}"),
                            diags,
                        );
                    }
                    for a in args.iter().skip(wants.len()) {
                        check_expr(
                            file,
                            a,
                            caller,
                            sigs,
                            structs,
                            struct_fields,
                            enums,
                            diags,
                            _depth,
                            awaited,
                            defined,
                            group_stack,
                            cx,
                        );
                    }
                    Ty::Enum(enum_name.clone())
                }
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => check_match(
            file,
            scrutinee,
            arms,
            caller,
            sigs,
            structs,
            struct_fields,
            enums,
            diags,
            _depth,
            awaited,
            defined,
            group_stack,
            cx,
        ),
        Expr::Closure {
            id,
            params,
            return_ty,
            body,
        } => {
            // Anonymous function value. The body checks against a synthetic
            // caller (pre-registered in `sigs`) so `return` uses the
            // literal's own declared type, not the enclosing function's.
            // Task and loop context are fresh: the closure may outlive its
            // definition site (stored, passed, returned), so `spawn`/
            // `await` inside is rejected loudly and `break` cannot cross
            // into the closure.
            let mut clos_defined = defined.clone();
            let mut seen: Vec<String> = Vec::new();
            let mut param_tys = Vec::with_capacity(params.len());
            for p in params {
                if seen.contains(&p.name) {
                    diags.push(duplicate(
                        file,
                        &format!("duplicate parameter `{}` in closure", p.name),
                    ));
                } else {
                    seen.push(p.name.clone());
                }
                let ty = resolve_body_ty(&p.ty, &caller.type_params, enums, structs);
                param_tys.push(ty.clone());
                clos_defined.insert(p.name.clone(), ty);
            }
            let ret_ty = resolve_body_ty(return_ty, &caller.type_params, enums, structs);
            let synth = FunctionDecl {
                id: id.clone(),
                name: crate::closures::closure_fn_name(id),
                name_span: caller.name_span,
                is_pub: false,
                type_params: caller.type_params.clone(),
                params: params.clone(),
                return_ty: return_ty.clone(),
                return_ty_omitted: false,
                effects: Vec::new(),
                body: body.clone(),
            };
            let mut fresh_groups: Vec<Vec<PendingSpawn>> = Vec::new();
            let mut fresh_cx = Ctx::default();
            fresh_cx.alias_scopes = cx.alias_scopes.clone();
            check_block(
                file,
                body,
                &synth,
                sigs,
                structs,
                struct_fields,
                enums,
                diags,
                0,
                &mut fresh_groups,
                &mut clos_defined,
                &mut fresh_cx,
            );
            Ty::Closure {
                params: param_tys,
                ret: Box::new(ret_ty),
            }
        }
    }
}

/// Check one match arm's setup statements plus its trailing body.
/// `arm_defined` enters holding the outer scope plus the arm's payload
/// bindings; setup `let`s extend it for the guard and trailing body (new
/// bindings never leak past the arm). The guard sees the bindings and
/// setup lets and must be `bool`/`int` like any condition; it is only
/// ever evaluated when the pattern matched (enforced in MIR lowering).
/// Spawn discipline mirrors `if` branches — an arm may not run, so its
/// spawns must be awaited inside it — and `break`/`continue` cannot
/// cross the arm boundary (same rule as `try` slices, via
/// `match_arm_depths`). Awaits inside setup statements never propagate
/// outward (the arm may not run); the trailing body's existing
/// propagation is unchanged.
#[allow(clippy::too_many_arguments)]
fn check_arm_body(
    file: &str,
    arm: &MatchArm,
    arm_defined: &mut HashMap<String, Ty>,
    caller: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    enums: &EnumTable,
    diags: &mut Vec<Diagnostic>,
    depth: usize,
    awaited: &mut Vec<String>,
    group_stack: &mut Vec<Vec<PendingSpawn>>,
    cx: &mut Ctx,
) -> Ty {
    if !arm.stmts.is_empty() {
        let pending_before = group_stack.last().map(|v| v.len()).unwrap_or(0);
        cx.match_arm_depths.push(cx.loop_depth);
        let arm_block = Block {
            id: arm.id.clone(),
            stmts: arm.stmts.clone(),
        };
        let arm_awaited = check_block(
            file,
            &arm_block,
            caller,
            sigs,
            structs,
            struct_fields,
            enums,
            diags,
            depth + 1,
            group_stack,
            arm_defined,
            cx,
        );
        cx.match_arm_depths.pop();
        if let Some(pending) = group_stack.last() {
            for p in pending.iter().skip(pending_before) {
                if !arm_awaited.contains(&p.handle) {
                    diags.push(Diagnostic::task_leak(file, p.span.0, p.span.1, &p.handle));
                }
            }
        }
    }
    if let Some(g) = &arm.guard {
        let g_ty = check_expr(
            file,
            g,
            caller,
            sigs,
            structs,
            struct_fields,
            enums,
            diags,
            depth,
            awaited,
            arm_defined,
            group_stack,
            cx,
        );
        require_condition(file, &g_ty, "match guard", diags);
    }
    check_expr(
        file,
        &arm.body,
        caller,
        sigs,
        structs,
        struct_fields,
        enums,
        diags,
        depth,
        awaited,
        arm_defined,
        group_stack,
        cx,
    )
}

/// Every variable name a pattern binding list (recursively) binds.
/// `_` binds nothing.
fn arm_bound_names(bindings: &[MatchBinding]) -> Vec<String> {
    let mut out = Vec::new();
    for b in bindings {
        b.bound_names(&mut out);
    }
    out
}

/// Bind one pattern element against one payload type, recursing into
/// nested patterns. Unknown nested targets bind `Unknown` (dynamic code
/// falls through at runtime on tag mismatch); statically impossible
/// nests (a field known to be a different enum, or a non-enum) are loud
/// `E-TYPE`, mirroring the top-level cross-enum rule.
fn bind_pattern(
    file: &str,
    b: &MatchBinding,
    w: &Ty,
    arm_defined: &mut HashMap<String, Ty>,
    enums: &EnumTable,
    diags: &mut Vec<Diagnostic>,
) {
    let bind_unknowns = |b: &MatchBinding, arm_defined: &mut HashMap<String, Ty>| {
        let mut names = Vec::new();
        b.bound_names(&mut names);
        for n in names {
            arm_defined.insert(n, Ty::Unknown);
        }
    };
    match b {
        MatchBinding::Ignore => {}
        MatchBinding::Bind(name) => {
            // Payloads of generic type parameters erase to dynamic at
            // the use site (same rule as field access): the arm cannot
            // know the instantiation.
            match w {
                Ty::Param(_) => arm_defined.insert(name.clone(), Ty::Unknown),
                _ => arm_defined.insert(name.clone(), w.clone()),
            };
        }
        MatchBinding::Nested {
            enum_name,
            variant,
            bindings,
        } => match enums.get(enum_name) {
            None => {
                diags.push(undefined(file, enum_name));
                bind_unknowns(b, arm_defined);
            }
            Some(variants) => match variants.get(variant) {
                None => {
                    diags.push(undefined(file, &format!("{enum_name}::{variant}")));
                    bind_unknowns(b, arm_defined);
                }
                Some(payloads) => {
                    let clash = match w {
                        Ty::Enum(other) => other != enum_name,
                        Ty::Unknown | Ty::Param(_) => false,
                        _ => true,
                    };
                    if clash {
                        diags.push(type_mismatch(
                            file,
                            "match pattern",
                            &format!("field of `{enum_name}`"),
                            w,
                        ));
                        bind_unknowns(b, arm_defined);
                        return;
                    }
                    if bindings.len() != payloads.len() {
                        diags.push(match_binding_arity(
                            file,
                            enum_name,
                            variant,
                            payloads.len(),
                            bindings.len(),
                        ));
                    }
                    for (sb, sw) in bindings.iter().zip(payloads.iter()) {
                        bind_pattern(file, sb, sw, arm_defined, enums, diags);
                    }
                    for sb in bindings.iter().skip(payloads.len()) {
                        bind_unknowns(sb, arm_defined);
                    }
                }
            },
        },
    }
}

/// Check a `match` expression: arm shapes, binding types, branch result
/// agreement, and exhaustiveness over the scrutinee enum's variants.
/// Returns the common result type (`Unknown` when nothing is known).
#[allow(clippy::too_many_arguments)]
fn check_match(
    file: &str,
    scrutinee: &Expr,
    arms: &[MatchArm],
    caller: &FunctionDecl,
    sigs: &HashMap<String, Sig>,
    structs: &[String],
    struct_fields: &HashMap<String, HashMap<String, Ty>>,
    enums: &EnumTable,
    diags: &mut Vec<Diagnostic>,
    depth: usize,
    awaited: &mut Vec<String>,
    defined: &HashMap<String, Ty>,
    group_stack: &mut Vec<Vec<PendingSpawn>>,
    cx: &mut Ctx,
) -> Ty {
    let s_ty = check_expr(
        file,
        scrutinee,
        caller,
        sigs,
        structs,
        struct_fields,
        enums,
        diags,
        depth,
        awaited,
        defined,
        group_stack,
        cx,
    );
    match &s_ty {
        Ty::Enum(ename) => {
            let variants: HashMap<String, Vec<Ty>> = enums.get(ename).cloned().unwrap_or_default();
            let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut has_wildcard = false;
            let mut result: Option<Ty> = None;
            for arm in arms {
                // Arm bindings are scoped to the arm body only.
                let mut arm_defined = defined.clone();
                if arm.is_wildcard() {
                    // Only an unguarded wildcard covers: a guarded one may
                    // fall through, so exhaustiveness ignores it.
                    if arm.guard.is_none() {
                        has_wildcard = true;
                    }
                } else {
                    let aname = arm.enum_name.as_deref().unwrap_or("");
                    let avar = arm.variant.as_deref().unwrap_or("");
                    if aname != ename.as_str() {
                        diags.push(type_mismatch(
                            file,
                            "match arm",
                            &format!("variant of `{ename}`"),
                            &Ty::Enum(aname.to_string()),
                        ));
                        for b in arm_bound_names(&arm.bindings) {
                            arm_defined.insert(b, Ty::Unknown);
                        }
                    } else if let Some(wants) = variants.get(avar) {
                        if arm.bindings.len() != wants.len() {
                            diags.push(match_binding_arity(
                                file,
                                ename,
                                avar,
                                wants.len(),
                                arm.bindings.len(),
                            ));
                        }
                        for (b, w) in arm.bindings.iter().zip(wants.iter()) {
                            bind_pattern(file, b, w, &mut arm_defined, enums, diags);
                        }
                        for b in arm.bindings.iter().skip(wants.len()) {
                            let mut names = Vec::new();
                            b.bound_names(&mut names);
                            for n in names {
                                arm_defined.insert(n, Ty::Unknown);
                            }
                        }
                        // Only a plain arm covers its variant: a guard may
                        // be false and a nested pattern may mismatch, so
                        // both fall through and never satisfy
                        // exhaustiveness alone.
                        if arm.guard.is_none() && !arm.has_nested_pattern() {
                            covered.insert(avar.to_string());
                        }
                    } else {
                        diags.push(undefined(file, &format!("{ename}::{avar}")));
                        for b in arm_bound_names(&arm.bindings) {
                            arm_defined.insert(b, Ty::Unknown);
                        }
                    }
                }
                let body_ty = check_arm_body(
                    file,
                    arm,
                    &mut arm_defined,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    awaited,
                    group_stack,
                    cx,
                );
                match &result {
                    None => result = Some(body_ty),
                    Some(t0) => {
                        // Order-independent unification: Int/Float mix to
                        // Float either way; otherwise require assignability
                        // in either direction (previously Int-then-Float
                        // errored while Float-then-Int passed).
                        let unified = unify_match_arms(t0, &body_ty);
                        match unified {
                            Some(u) => result = Some(u),
                            None => {
                                diags.push(type_mismatch(file, "match arm", &t0.name(), &body_ty));
                            }
                        }
                    }
                }
            }
            // Duplicate arms and arms after `_` are dead code: diagnose.
            // Only plain arms cover: a guarded arm may fall through when
            // its guard is false and a nested-pattern arm may fall through
            // on nested-tag mismatch, so later same-pattern arms are still
            // reachable (no duplicate), and arms after a guarded wildcard
            // still run (no unreachable). An arm after an UNGUARDED
            // wildcard never runs, guarded or not; an arm after a plain
            // (unguarded, non-nested) same pattern never runs.
            {
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut seen_wildcard = false;
                for arm in arms {
                    if seen_wildcard {
                        diags.push(Diagnostic::error(
                            "E-MATCH-UNREACHABLE",
                            "unreachable match arm after `_` wildcard",
                            file,
                            0,
                            0,
                            "arms after the wildcard never run",
                            &["remove the dead arm", "move it before `_`"],
                            "match/unreachable",
                        ));
                        break;
                    }
                    if arm.is_wildcard() {
                        if arm.guard.is_none() {
                            seen_wildcard = true;
                        }
                    } else {
                        let key = format!(
                            "{}::{}",
                            arm.enum_name.as_deref().unwrap_or(""),
                            arm.variant.as_deref().unwrap_or("")
                        );
                        if arm.guard.is_none() && !arm.has_nested_pattern() {
                            if !seen.insert(key.clone()) {
                                diags.push(Diagnostic::error(
                                    "E-MATCH-DUPLICATE",
                                    &format!("duplicate match arm `{key}`"),
                                    file,
                                    0,
                                    0,
                                    "duplicate pattern never runs",
                                    &["remove the duplicate arm"],
                                    "match/duplicate",
                                ));
                            }
                        } else if seen.contains(&key) {
                            // A guarded or nested-pattern arm after a plain
                            // same pattern never runs (the earlier arm
                            // always takes it).
                            diags.push(Diagnostic::error(
                                "E-MATCH-DUPLICATE",
                                &format!("duplicate match arm `{key}`"),
                                file,
                                0,
                                0,
                                "duplicate pattern never runs",
                                &["remove the duplicate arm"],
                                "match/duplicate",
                            ));
                        }
                    }
                }
            }
            if !has_wildcard {
                let mut missing: Vec<String> = variants
                    .keys()
                    .filter(|v| !covered.contains(*v))
                    .cloned()
                    .collect();
                // `covered` is a set now, so this filter is linear overall.
                missing.sort();
                if !missing.is_empty() {
                    diags.push(match_exhaustive(file, ename, &missing));
                }
            }
            result.unwrap_or(Ty::Unknown)
        }
        Ty::Unknown => {
            // Dynamic scrutinee: validate arm shapes, skip exhaustiveness.
            let mut result: Option<Ty> = None;
            for arm in arms {
                let mut arm_defined = defined.clone();
                if !arm.is_wildcard() {
                    let aname = arm.enum_name.as_deref().unwrap_or("");
                    let avar = arm.variant.as_deref().unwrap_or("");
                    let known = enums.get(aname).and_then(|vs| vs.get(avar)).is_some();
                    if !known {
                        diags.push(undefined(file, &format!("{aname}::{avar}")));
                    }
                    for b in arm_bound_names(&arm.bindings) {
                        arm_defined.insert(b, Ty::Unknown);
                    }
                }
                let body_ty = check_arm_body(
                    file,
                    arm,
                    &mut arm_defined,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    awaited,
                    group_stack,
                    cx,
                );
                if result.is_none() {
                    result = Some(body_ty);
                }
            }
            result.unwrap_or(Ty::Unknown)
        }
        other => {
            diags.push(type_mismatch(file, "match scrutinee", "enum", other));
            for arm in arms {
                let mut arm_defined = defined.clone();
                for b in arm_bound_names(&arm.bindings) {
                    arm_defined.insert(b, Ty::Unknown);
                }
                check_arm_body(
                    file,
                    arm,
                    &mut arm_defined,
                    caller,
                    sigs,
                    structs,
                    struct_fields,
                    enums,
                    diags,
                    depth,
                    awaited,
                    group_stack,
                    cx,
                );
            }
            Ty::Unknown
        }
    }
}

/// `==` allows numeric mixes, identical types, or anything-Unknown.
/// Type variables are wildcards too (instantiation unknown in the body).
fn equality_ok(l: &Ty, r: &Ty) -> bool {
    if matches!(l, Ty::Unknown | Ty::Param(_)) || matches!(r, Ty::Unknown | Ty::Param(_)) {
        return true;
    }
    if l == r {
        return true;
    }
    matches!((l, r), (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int))
}

fn check_builtin_call(file: &str, func: &str, args: &[Ty], diags: &mut Vec<Diagnostic>) -> Ty {
    // `format(...)` is variadic (any count including zero): every value
    // renders via the same rule as `str()`/`print`, joined with one space.
    if func == "format" {
        return Ty::Str;
    }
    let arity = match func {
        "len" | "pop" | "keys" => 1,
        "push" | "range" | "write_file" | "append_file" | "run_process" | "spawn" | "run" => 2,
        "insert" => 3,
        "http_post" | "http_post_async" => 3,
        "str" | "int" | "float" => 1,
        "assert" => 1,
        "read_line" => 0,
        "parse_int" | "parse_float" => 1,
        "read_file" | "exists" | "env" | "get_env" | "remove_file" | "http_get" | "http_get_async" => 1,
        "regex_is_match" | "regex_find" => 2,
        "time_sleep" | "time_elapsed" | "sleep_ms" => 1,
        "time_now" | "now_ms" | "now_iso" => 0,
        "args" | "cwd" => 0,
        "exit" => 1,
        "set_env" | "rename_file" | "copy_file" => 2,
        "list_dir" | "make_dir" | "make_dirs" | "is_dir" | "is_file" | "file_size"
        | "ord" | "chr" | "sort" => 1,
        "slice" => 3,
        "sqrt" | "sin" | "cos" | "tan" | "log" | "exp" | "floor" | "ceil" | "abs"
        | "sha256" => 1,
        "pow" | "atan2" | "min" | "max" | "random_int" | "hmac_sha256" => 2,
        "random_float" => 0,
        _ => return Ty::Unknown,
    };
    if args.len() != arity {
        diags.push(Diagnostic::error(
            "E-ARITY",
            &format!("`{func}` takes {arity} arguments, got {}", args.len()),
            file,
            0,
            0,
            "builtin arity must match",
            &["pass the right number of arguments"],
            "calls/arity",
        ));
        return Ty::Unknown;
    }
    match func {
        "len" => {
            if !matches!(&args[0], Ty::Array | Ty::Str | Ty::Map | Ty::Unknown) {
                diags.push(type_mismatch(
                    file,
                    "`len()` argument",
                    "array/str/map",
                    &args[0],
                ));
            }
            Ty::Int
        }
        "push" => {
            if !matches!(&args[0], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(file, "`push()` target", "array", &args[0]));
            }
            Ty::Int
        }
        "insert" => {
            if !matches!(&args[0], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(file, "`insert()` target", "array", &args[0]));
            }
            if !matches!(&args[1], Ty::Int | Ty::Unknown) {
                diags.push(type_mismatch(file, "`insert()` index", "i32", &args[1]));
            }
            // Any value can be inserted (mirrors `push`, which does not
            // check its element type either). Returns the new length.
            Ty::Int
        }
        "pop" => {
            if !matches!(&args[0], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(file, "`pop()` target", "array", &args[0]));
            }
            Ty::Unknown
        }
        "range" => {
            for a in args {
                if !matches!(a, Ty::Int | Ty::Unknown) {
                    diags.push(type_mismatch(file, "`range()` bound", "i32", a));
                }
            }
            Ty::Array
        }
        "str" => Ty::Str,
        "int" => Ty::Int,
        "float" => Ty::Float,
        // Phase 2 Batch B: float math. Ints widen at runtime; anything
        // else is `E-TYPE` here (literals) and in `exec_builtin`
        // (`Unknown`-typed values that arrive misshapen).
        "sqrt" | "sin" | "cos" | "tan" | "log" | "exp" | "floor" | "ceil" => {
            if !matches!(&args[0], Ty::Int | Ty::Float | Ty::Unknown) {
                diags.push(type_mismatch(file, &format!("`{func}()` argument"), "int or float", &args[0]));
            }
            Ty::Float
        }
        "pow" | "atan2" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Int | Ty::Float | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`{func}()` arg {i}"),
                        "int or float",
                        a,
                    ));
                }
            }
            Ty::Float
        }
        "abs" => {
            if !matches!(&args[0], Ty::Int | Ty::Float | Ty::Unknown) {
                diags.push(type_mismatch(file, "`abs()` argument", "int or float", &args[0]));
            }
            // Ints stay ints at runtime; floats stay floats.
            match &args[0] {
                Ty::Int => Ty::Int,
                _ => Ty::Float,
            }
        }
        "min" | "max" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Int | Ty::Float | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`{func}()` arg {i}"),
                        "int or float",
                        a,
                    ));
                }
            }
            // Both-int stays int; anything else widens to float.
            if matches!(&args[0], Ty::Int) && matches!(&args[1], Ty::Int) {
                Ty::Int
            } else {
                Ty::Float
            }
        }
        "random_int" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Int | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`random_int()` bound {i}"),
                        "int",
                        a,
                    ));
                }
            }
            Ty::Int
        }
        "random_float" => Ty::Float,
        "sha256" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`sha256()` text", "str", &args[0]));
            }
            Ty::Str
        }
        "hmac_sha256" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Str | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`hmac_sha256()` arg {i}"),
                        "str",
                        a,
                    ));
                }
            }
            Ty::Str
        }
        "read_line" => Ty::Str,
        "parse_int" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`parse_int()` text", "str", &args[0]));
            }
            Ty::Int
        }
        "parse_float" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`parse_float()` text", "str", &args[0]));
            }
            Ty::Float
        }
        "keys" => {
            if !matches!(&args[0], Ty::Map | Ty::Unknown) {
                diags.push(type_mismatch(file, "`keys()` argument", "map", &args[0]));
            }
            Ty::Array
        }
        "assert" => {
            if !matches!(&args[0], Ty::Bool | Ty::Int | Ty::Unknown) {
                diags.push(type_mismatch(file, "`assert()` argument", "bool", &args[0]));
            }
            Ty::Int
        }
        "read_file" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`read_file()` path", "str", &args[0]));
            }
            Ty::Str
        }
        "write_file" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Str | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`write_file()` arg {i}"),
                        "str",
                        a,
                    ));
                }
            }
            Ty::Int
        }
        "append_file" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Str | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`append_file()` arg {i}"),
                        "str",
                        a,
                    ));
                }
            }
            Ty::Int
        }
        "exists" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`exists()` path", "str", &args[0]));
            }
            Ty::Bool
        }
        "remove_file" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`remove_file()` path", "str", &args[0]));
            }
            Ty::Int
        }
        "env" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`env()` name", "str", &args[0]));
            }
            Ty::Str
        }
        "get_env" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`get_env()` name", "str", &args[0]));
            }
            Ty::Str
        }
        "run_process" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`run_process()` cmd", "str", &args[0]));
            }
            if !matches!(&args[1], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(
                    file,
                    "`run_process()` args",
                    "array",
                    &args[1],
                ));
            }
            Ty::Map
        }
        "spawn" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`spawn()` cmd", "str", &args[0]));
            }
            if !matches!(&args[1], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(
                    file,
                    "`spawn()` args",
                    "array",
                    &args[1],
                ));
            }
            Ty::Map
        }
        "run" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`run()` cmd", "str", &args[0]));
            }
            if !matches!(&args[1], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(
                    file,
                    "`run()` args",
                    "array",
                    &args[1],
                ));
            }
            Ty::Int
        }
        "regex_is_match" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Str | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`regex_is_match()` arg {i}"),
                        "str",
                        a,
                    ));
                }
            }
            Ty::Bool
        }
        "regex_find" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Str | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`regex_find()` arg {i}"),
                        "str",
                        a,
                    ));
                }
            }
            Ty::Map
        }
        "time_sleep" => {
            if !matches!(&args[0], Ty::Int | Ty::Float | Ty::Unknown) {
                diags.push(type_mismatch(file, "`time_sleep()` seconds", "number", &args[0]));
            }
            Ty::Int
        }
        "sleep_ms" => {
            if !matches!(&args[0], Ty::Int | Ty::Float | Ty::Unknown) {
                diags.push(type_mismatch(file, "`sleep_ms()` millis", "number", &args[0]));
            }
            Ty::Int
        }
        "time_now" => Ty::Float,
        "now_ms" => Ty::Int,
        "now_iso" => Ty::Str,
        "time_elapsed" => {
            if !matches!(&args[0], Ty::Int | Ty::Float | Ty::Unknown) {
                diags.push(type_mismatch(file, "`time_elapsed()` since", "number", &args[0]));
            }
            Ty::Float
        }
        "http_get" | "http_get_async" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, &format!("`{func}()` url"), "str", &args[0]));
            }
            Ty::Map
        }
        "http_post" | "http_post_async" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, &format!("`{func}()` url"), "str", &args[0]));
            }
            if !matches!(&args[1], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, &format!("`{func}()` body"), "str", &args[1]));
            }
            if !matches!(&args[2], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(
                    file,
                    &format!("`{func}()` headers"),
                    "array",
                    &args[2],
                ));
            }
            Ty::Map
        }
        "args" => Ty::Array,
        "exit" => {
            if !matches!(&args[0], Ty::Int | Ty::Unknown) {
                diags.push(type_mismatch(file, "`exit()` code", "i32", &args[0]));
            }
            Ty::Int
        }
        "cwd" => Ty::Str,
        "set_env" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Str | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`set_env()` arg {i}"),
                        "str",
                        a,
                    ));
                }
            }
            Ty::Int
        }
        "list_dir" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`list_dir()` path", "str", &args[0]));
            }
            Ty::Array
        }
        "make_dir" | "make_dirs" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, &format!("`{func}()` path"), "str", &args[0]));
            }
            Ty::Int
        }
        "is_dir" | "is_file" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, &format!("`{func}()` path"), "str", &args[0]));
            }
            Ty::Bool
        }
        "rename_file" | "copy_file" => {
            for (i, a) in args.iter().enumerate() {
                if !matches!(a, Ty::Str | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`{func}()` arg {i}"),
                        "str",
                        a,
                    ));
                }
            }
            Ty::Int
        }
        "file_size" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`file_size()` path", "str", &args[0]));
            }
            Ty::Int
        }
        "ord" => {
            if !matches!(&args[0], Ty::Str | Ty::Unknown) {
                diags.push(type_mismatch(file, "`ord()` string", "str", &args[0]));
            }
            Ty::Int
        }
        "chr" => {
            if !matches!(&args[0], Ty::Int | Ty::Unknown) {
                diags.push(type_mismatch(file, "`chr()` codepoint", "i32", &args[0]));
            }
            Ty::Str
        }
        "slice" => {
            if !matches!(&args[0], Ty::Str | Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(file, "`slice()` value", "str/array", &args[0]));
            }
            for (i, a) in args.iter().enumerate().skip(1) {
                if !matches!(a, Ty::Int | Ty::Unknown) {
                    diags.push(type_mismatch(
                        file,
                        &format!("`slice()` arg {i}"),
                        "i32",
                        a,
                    ));
                }
            }
            if matches!(&args[0], Ty::Array) {
                Ty::Array
            } else {
                Ty::Str
            }
        }
        "sort" => {
            if !matches!(&args[0], Ty::Array | Ty::Unknown) {
                diags.push(type_mismatch(file, "`sort()` list", "array", &args[0]));
            }
            Ty::Array
        }
        _ => Ty::Unknown,
    }
}

/// Method-call result types by receiver type. Unknown receiver -> Unknown.
fn check_method_call(
    file: &str,
    base: &Ty,
    method: &str,
    args: &[Ty],
    diags: &mut Vec<Diagnostic>,
) -> Ty {
    match base {
        Ty::Unknown => Ty::Unknown,
        Ty::Str => match method {
            "len" | "upper" | "lower" | "trim" | "chars" | "split" | "contains" | "starts_with"
            | "ends_with" | "replace" => {
                let want = match method {
                    "replace" => 2,
                    "split" | "contains" | "starts_with" | "ends_with" => 1,
                    _ => 0,
                };
                if args.len() != want {
                    diags.push(method_arity(file, "str", method, want, args.len()));
                    return Ty::Unknown;
                }
                match method {
                    "len" => Ty::Int,
                    "split" | "chars" => Ty::Array,
                    "contains" | "starts_with" | "ends_with" => Ty::Bool,
                    _ => Ty::Str,
                }
            }
            _ => {
                diags.push(type_mismatch(
                    file,
                    "string method",
                    "known str method",
                    base,
                ));
                Ty::Unknown
            }
        },
        Ty::Array => match method {
            "len" | "push" | "pop" | "contains" | "join" | "insert" => {
                let want = match method {
                    "push" | "contains" | "join" => 1,
                    "insert" => 2,
                    _ => 0,
                };
                if args.len() != want {
                    diags.push(method_arity(file, "array", method, want, args.len()));
                    return Ty::Unknown;
                }
                match method {
                    "len" | "push" | "insert" => Ty::Int,
                    "contains" => Ty::Bool,
                    "join" => Ty::Str,
                    _ => Ty::Unknown,
                }
            }
            _ => {
                diags.push(type_mismatch(
                    file,
                    "array method",
                    "known array method",
                    base,
                ));
                Ty::Unknown
            }
        },
        Ty::Map => match method {
            "len" | "keys" | "contains" => {
                let want = if method == "contains" { 1 } else { 0 };
                if args.len() != want {
                    diags.push(method_arity(file, "map", method, want, args.len()));
                    return Ty::Unknown;
                }
                match method {
                    "len" => Ty::Int,
                    "keys" => Ty::Array,
                    _ => Ty::Bool,
                }
            }
            _ => {
                diags.push(type_mismatch(file, "map method", "known map method", base));
                Ty::Unknown
            }
        },
        other => {
            diags.push(type_mismatch(
                file,
                "method receiver",
                "str/array/map",
                other,
            ));
            Ty::Unknown
        }
    }
}

/// Builtin functions available in every program.
pub fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "len"
            | "push"
            | "pop"
            | "range"
            | "str"
            | "int"
            | "float"
            | "keys"
            | "assert"
            | "read_file"
            | "write_file"
            | "append_file"
            | "exists"
            | "remove_file"
            | "env"
            | "run_process"
            | "regex_is_match"
            | "regex_find"
            | "time_sleep"
            | "time_now"
            | "time_elapsed"
            | "http_get"
            | "http_post"
            | "http_get_async"
            | "http_post_async"
            | "read_line"
            | "parse_int"
            | "parse_float"
            | "format"
            | "insert"
            | "args"
            | "exit"
            | "cwd"
            | "set_env"
            | "list_dir"
            | "make_dir"
            | "make_dirs"
            | "is_dir"
            | "is_file"
            | "rename_file"
            | "copy_file"
            | "file_size"
            | "ord"
            | "chr"
            | "slice"
            | "sort"
            | "sqrt"
            | "pow"
            | "abs"
            | "min"
            | "max"
            | "floor"
            | "ceil"
            | "sin"
            | "cos"
            | "tan"
            | "atan2"
            | "log"
            | "exp"
            | "random_int"
            | "random_float"
            | "sha256"
            | "hmac_sha256"
            | "spawn"
            | "run"
            | "now_ms"
            | "now_iso"
            | "sleep_ms"
            | "get_env"
    )
}

fn expr_has_spawn(e: &Expr) -> bool {
    match e {
        Expr::Spawn { .. } => true,
        Expr::Add { left, right, .. }
        | Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. }
        | Expr::Eq { left, right, .. }
        | Expr::NotEq { left, right, .. }
        | Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. }
        | Expr::And { left, right, .. }
        | Expr::Or { left, right, .. } => expr_has_spawn(left) || expr_has_spawn(right),
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => expr_has_spawn(inner),
        Expr::Call { args, .. } => args.iter().any(expr_has_spawn),
        Expr::ArrayLit { elems, .. } => elems.iter().any(expr_has_spawn),
        Expr::MapLit { entries, .. } => entries.iter().any(|(_, v)| expr_has_spawn(v)),
        Expr::Index { base, index, .. } => expr_has_spawn(base) || expr_has_spawn(index),
        Expr::Field { base, .. } => expr_has_spawn(base),
        Expr::MethodCall { base, args, .. } => {
            expr_has_spawn(base) || args.iter().any(expr_has_spawn)
        }
        Expr::EnumCtor { args, .. } => args.iter().any(expr_has_spawn),
        Expr::Match {
            scrutinee, arms, ..
        } => expr_has_spawn(scrutinee) || arms.iter().any(|a| expr_has_spawn(&a.body)),
        _ => false,
    }
}

fn expr_span_hint(e: &Expr) -> (usize, usize) {
    match e {
        Expr::Spawn { call, .. } => expr_span_hint(call),
        _ => (0, 0),
    }
}

/// Order-independent match-arm unification: identical types stay,
/// Int/Float mix to Float either way, Unknown/Param are wildcards.
/// Returns None when arms disagree (diagnostic at call site).
fn unify_match_arms(a: &Ty, b: &Ty) -> Option<Ty> {
    if a == b {
        return Some(a.clone());
    }
    if matches!(a, Ty::Unknown | Ty::Param(_)) {
        return Some(b.clone());
    }
    if matches!(b, Ty::Unknown | Ty::Param(_)) {
        return Some(a.clone());
    }
    match (a, b) {
        (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Some(Ty::Float),
        _ => {
            if assignable(b, a) || assignable(a, b) {
                Some(a.clone())
            } else {
                None
            }
        }
    }
}

/// `await` with no enclosing `task_group`.
fn await_outside_group(file: &str) -> Diagnostic {
    Diagnostic::error(
        "E-AWAIT-OUTSIDE-GROUP",
        "await outside of any task_group",
        file,
        0,
        0,
        "await joins a child task tied to the enclosing task_group",
        &["wrap the await in a task_group block"],
        "structured-concurrency/scope",
    )
}

/// True when `e` contains an `await` anywhere (for outside-group checks).
fn expr_contains_await(e: &Expr) -> bool {
    match e {
        Expr::Await { .. } => true,
        Expr::Add { left, right, .. }
        | Expr::Sub { left, right, .. }
        | Expr::Mul { left, right, .. }
        | Expr::Div { left, right, .. }
        | Expr::Mod { left, right, .. }
        | Expr::Eq { left, right, .. }
        | Expr::NotEq { left, right, .. }
        | Expr::Lt { left, right, .. }
        | Expr::LtEq { left, right, .. }
        | Expr::Gt { left, right, .. }
        | Expr::GtEq { left, right, .. }
        | Expr::And { left, right, .. }
        | Expr::Or { left, right, .. } => expr_contains_await(left) || expr_contains_await(right),
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => expr_contains_await(inner),
        Expr::Call { args, .. } => args.iter().any(expr_contains_await),
        Expr::ArrayLit { elems, .. } => elems.iter().any(expr_contains_await),
        Expr::MapLit { entries, .. } => entries.iter().any(|(_, v)| expr_contains_await(v)),
        Expr::StructLit { fields, .. } => fields.iter().any(|(_, v)| expr_contains_await(v)),
        Expr::EnumCtor { args, .. } => args.iter().any(expr_contains_await),
        Expr::Match {
            scrutinee, arms, ..
        } => expr_contains_await(scrutinee) || arms.iter().any(|a| expr_contains_await(&a.body)),
        Expr::Index { base, index, .. } => expr_contains_await(base) || expr_contains_await(index),
        Expr::Field { base, .. } => expr_contains_await(base),
        Expr::MethodCall { base, args, .. } => {
            expr_contains_await(base) || args.iter().any(expr_contains_await)
        }
        Expr::Spawn { call, .. } => expr_contains_await(call),
        _ => false,
    }
}

fn stmt_uses_tasks(s: &crate::ast::Stmt) -> bool {
    use crate::ast::Stmt;
    match s {
        Stmt::TaskGroup(_) => true,
        Stmt::Let(l) => expr_contains_await(&l.value) || expr_has_spawn(&l.value),
        Stmt::Assign(a) => expr_contains_await(&a.value),
        Stmt::Return(r) => expr_contains_await(&r.value),
        Stmt::Print(p) => expr_contains_await(&p.value),
        Stmt::Expr(e) => expr_contains_await(e) || expr_has_spawn(e),
        Stmt::If(i) => {
            expr_contains_await(&i.cond)
                || i.then_block.stmts.iter().any(stmt_uses_tasks)
                || i.else_block
                    .as_ref()
                    .map(|b| b.stmts.iter().any(stmt_uses_tasks))
                    .unwrap_or(false)
        }
        Stmt::While(w) => expr_contains_await(&w.cond) || w.body.stmts.iter().any(stmt_uses_tasks),
        Stmt::ForRange(fr) => fr.body.stmts.iter().any(stmt_uses_tasks),
        Stmt::ForIn(fi) => {
            expr_contains_await(&fi.iter) || fi.body.stmts.iter().any(stmt_uses_tasks)
        }
        Stmt::TryCatch(t) => {
            t.body.stmts.iter().any(stmt_uses_tasks)
                || t.handler.stmts.iter().any(stmt_uses_tasks)
        }
        Stmt::Break(_) | Stmt::Continue(_) => false,
    }
}

fn fn_uses_tasks(f: &FunctionDecl) -> bool {
    f.body.stmts.iter().any(stmt_uses_tasks)
}

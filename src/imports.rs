//! Multi-file imports: whole-file merge plus selective imports.
//!
//! Two syntaxes, one loader:
//! - `import "lib.klang"` merges the whole file (duplicate `fn` names are
//!   an error, as before).
//! - `import { name, ... } from "lib.klang"` merges only the named
//!   top-level items (function/struct/enum) from the target file.
//!
//! Loading is a breadth-first walk with a canonical-path `seen` set, so
//! circular imports terminate (each file merges at most once per mode)
//! instead of hanging or overflowing the compiler stack — a known risk
//! class for this project. Every failure is a clean [`ImportError`] with
//! an `E-*` code (`E-IO-NOT-FOUND` for a missing file, `E-IMPORT` for a
//! missing name/unsafe path/duplicate), never a panic.
//!
//! Selectivity notes (pinned behavior):
//! - A name matching several top-level kinds (e.g. both a `fn` and a
//!   `struct`) imports every match; a name matching nothing — including
//!   a `mod` block, which is imported whole-file only — is `E-IMPORT`.
//! - Re-importing an already-merged item is idempotent (diamond imports
//!   merge once); a same-named item from a DIFFERENT file is `E-DUPLICATE`.
//! - Items the target file itself imports are still followed transitively
//!   (selective edges stay selective, whole edges stay whole), so an
//!   imported function whose sibling helper lives in a third file keeps
//!   working. But items merely *defined* in the target file that were NOT
//!   named are invisible: referencing them is `E-UNDEFINED` at check time.
//! - `mod` blocks are never split: selective import merges top-level
//!   items only.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use crate::ast::Program;
use crate::diagnostics::{self, Diagnostic};
use crate::parser::Parser;

/// One file the loader read (for the CLI `file:` preamble).
#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub path: String,
    pub bytes: usize,
}

/// A successfully loaded multi-file program plus per-file info.
#[derive(Debug, Clone)]
pub struct LoadedProgram {
    pub program: Program,
    pub files: Vec<LoadedFile>,
}

/// A clean load failure: always an `E-*` code, never a panic.
#[derive(Debug, Clone)]
pub struct ImportError {
    /// `E-IO-NOT-FOUND`, `E-IMPORT`, `E-DUPLICATE`, or `E-PARSE`.
    pub code: &'static str,
    pub message: String,
}

impl ImportError {
    fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }

    /// Render as a structured diagnostic. For `E-PARSE` the message
    /// already IS the inner diagnostic JSON, so it passes through
    /// unchanged (same bytes the single-file path prints).
    pub fn to_json(&self) -> String {
        if self.code == "E-PARSE" {
            return self.message.clone();
        }
        Diagnostic::error(
            self.code,
            &self.message,
            "import",
            0,
            0,
            "multi-file import failed",
            &[],
            "modules/import",
        )
        .to_json()
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_json())
    }
}

enum Work {
    Whole {
        path: String,
        importer: String,
        raw: String,
    },
    Selective {
        path: String,
        names: Vec<String>,
        importer: String,
        raw: String,
    },
    /// `import "p" as m`: parse + expand the target like a whole import
    /// (nested imports resolve against its own directory, vendor fallback
    /// included), but merge NOTHING flat here — Phase B renames its
    /// functions and rewrites the importer's `m.f` call sites instead.
    Alias {
        path: String,
        alias: String,
        importer: String,
        raw: String,
    },
}

/// Registry vendor context for one load: the project root (dir holding
/// `klang.toml`, if any) plus its registry deps. `None` when the entry
/// lives outside any project — then only relative imports resolve.
struct VendorCtx {
    root: PathBuf,
    deps: Vec<(String, String)>,
}

/// Load `entry` plus its transitive imports, with the entry file's
/// content supplied in memory (`entry_src`) instead of read from disk.
///
/// This is the language-server entry point (FOUNDATION-3 Part 2C): the
/// editor's dirty buffer is checked exactly as `klang check` would check
/// the saved file — same loader, same checker, same diagnostics — while
/// every *imported* file still resolves from disk. The override is keyed
/// on the entry's canonical path and pre-seeded before the BFS walk, so
/// `ensure_parsed` treats it as already parsed (imports enqueued from
/// the buffer's own `import` statements, cycles still terminate).
pub fn load_program_with_entry_source(
    entry: &str,
    entry_src: &str,
) -> Result<LoadedProgram, ImportError> {
    let canon = canonical(entry);
    let mut parsed: HashMap<String, Program> = HashMap::new();
    let mut path_of_idx: Vec<String> = Vec::new();
    let mut files: Vec<LoadedFile> = Vec::new();
    let mut p = Parser::new_with_file(entry_src, entry);
    let prog = p
        .parse_program()
        .map_err(|d| ImportError::new("E-PARSE", d.to_json()))?;
    path_of_idx.push(canon.clone());
    files.push(LoadedFile {
        path: entry.to_string(),
        bytes: entry_src.len(),
    });
    parsed.insert(canon, prog.with_file_prefix(0));
    load_from(entry, parsed, path_of_idx, files)
}

/// Load `entry` plus its transitive imports (whole and selective).
pub fn load_program(entry: &str) -> Result<LoadedProgram, ImportError> {
    load_from(
        entry,
        HashMap::new(),
        Vec::new(),
        Vec::new(),
    )
}

/// Shared BFS body for [`load_program`] and
/// [`load_program_with_entry_source`]. `parsed`/`path_of_idx`/`files`
/// may arrive pre-seeded (entry source override) or empty (read the
/// entry from disk like always).
fn load_from(
    entry: &str,
    mut parsed: HashMap<String, Program>,
    mut path_of_idx: Vec<String>,
    mut files: Vec<LoadedFile>,
) -> Result<LoadedProgram, ImportError> {
    let mut merged = Program {
        mods: vec![],
        enums: vec![],
        structs: vec![],
        imports: vec![],
        selective_imports: vec![],
        aliased_imports: vec![],
        alias_scopes: vec![],
        functions: vec![],
    };
    // Canonical path -> prefixed parse (cached so each file parses once,
    // which is also what makes cycles terminate). When the entry source
    // was supplied in memory, `parsed` already holds it under the entry's
    // canonical path, so the walk below reuses it instead of reading disk.
    let mut whole_done: HashSet<String> = HashSet::new();
    // Canonical paths whose own imports were already queued: a file's
    // imports are static, so expanding once (on first sight, whole or
    // selective) is enough. WITHOUT this, a selective cycle (A imports
    // {g} from B, B imports {f} from A) re-queues forever — a hang, not
    // a loud error. This set is what makes cycles terminate.
    let mut expanded: HashSet<String> = HashSet::new();
    // (canonical path, kind, name) already merged: keeps selective merges
    // AND later whole merges of the same file idempotent (diamond-safe).
    let mut merged_items: HashSet<(String, String, String)> = HashSet::new();
    let mut fn_names: HashSet<String> = HashSet::new();
    // Origin canonical path per merged function (parallel to
    // `merged.functions`): Phase B rewrites alias call sites in exactly
    // the functions of each importing file.
    let mut fn_origins: Vec<String> = Vec::new();
    // Origin canonical path per merged mod block (parallel to
    // `merged.mods`): same file-locality rule for `mod` members.
    let mut mod_origins: Vec<String> = Vec::new();
    // (importer canon, alias, target canon, raw): one entry per
    // `import "p" as m`, deduplicated. Resolved in Phase B, after the
    // flat walk, so targets are parsed no matter the edge order.
    let mut alias_edges: Vec<(String, String, String, String)> = Vec::new();

    let mut queue: VecDeque<Work> = VecDeque::from([Work::Whole {
        path: entry.to_string(),
        importer: entry.to_string(),
        raw: entry.to_string(),
    }]);
    // Vendor context, computed once from the entry: a `klang.toml` found
    // by walking up from the entry file enables `pkg/file.klang`
    // fallback imports for registry dependencies (direct + transitive
    // via klang.lock; local files always win downstream).
    let vendor: Option<VendorCtx> = {
        let start = std::path::Path::new(entry);
        crate::registry::find_project_root(start).and_then(|root| {
            let text = std::fs::read_to_string(root.join("klang.toml")).ok()?;
            let manifest = crate::package::Manifest::parse(&text).ok()?;
            let mut deps: Vec<(String, String)> = manifest
                .deps
                .into_iter()
                .chain(manifest.dev_deps)
                .filter(|(k, v)| {
                    crate::registry::parse_registry_dep(v).is_some()
                        || crate::registry::parse_registry_req(v, k).is_some()
                })
                .collect();
            // Transitive pins from the lock get synthetic entries so
            // `resolve_vendor_import` can see them even without a
            // direct manifest line.
            if let Ok(lock_text) = std::fs::read_to_string(root.join("klang.lock")) {
                for pin in crate::registry::parse_package_locks(&lock_text) {
                    if !deps.iter().any(|(n, _)| n == &pin.name) {
                        deps.push((
                            pin.name.clone(),
                            format!("registry:{}@{}", pin.name, pin.version),
                        ));
                    }
                }
            }
            if deps.is_empty() {
                None
            } else {
                Some(VendorCtx { root, deps })
            }
        })
    };

    while let Some(work) = queue.pop_front() {
        match work {
            Work::Whole { path, importer, raw } => {
                // B1: resolve to the REAL file first (vendor fallback) so
                // the canonical key and the base dir for nested imports
                // are the vendored location, not the logical path.
                let real = resolve_real(&path, &raw, &vendor).unwrap_or_else(|| path.clone());
                let canon = canonical(&real);
                if whole_done.contains(&canon) {
                    continue;
                }
                let real_path =
                    ensure_parsed(&path, &raw, &vendor, &mut parsed, &mut path_of_idx, &mut files)?;
                // `ensure_parsed` canonicalizes the real path; recompute
                // to stay in sync (same value as `canon` above).
                let canon = canonical(&real_path);
                if whole_done.contains(&canon) {
                    continue;
                }
                // Mark BEFORE merging so a self-importing file terminates.
                whole_done.insert(canon.clone());
                let prog = parsed.get(&canon).expect("just parsed").clone();
                if expanded.insert(canon.clone()) {
                    enqueue_imports(&prog, &real_path, &mut queue)?;
                }
                let mods_before = merged.mods.len();
                merged.mods.extend(prog.mods);
                for _ in mods_before..merged.mods.len() {
                    mod_origins.push(canon.clone());
                }
                merged.enums.extend(prog.enums);
                merged.structs.extend(prog.structs);
                for f in prog.functions {
                    insert_fn(
                        &mut merged.functions,
                        &mut fn_origins,
                        &mut fn_names,
                        &mut merged_items,
                        &canon,
                        f,
                        &importer,
                    )?;
                }
            }
            Work::Selective {
                path,
                names,
                importer,
                raw,
            } => {
                let real_path =
                    ensure_parsed(&path, &raw, &vendor, &mut parsed, &mut path_of_idx, &mut files)?;
                let canon = canonical(&real_path);
                let prog = parsed.get(&canon).expect("just parsed").clone();
                // The target's own imports still apply transitively (a
                // named function may need its file's other imports).
                if expanded.insert(canon.clone()) {
                    enqueue_imports(&prog, &real_path, &mut queue)?;
                }
                for name in &names {
                    import_named(
                        &prog,
                        &canon,
                        name,
                        &mut merged,
                        &mut fn_origins,
                        &mut fn_names,
                        &mut merged_items,
                        &importer,
                    )?;
                }
            }
            Work::Alias {
                path,
                alias,
                importer,
                raw,
            } => {
                // Parse + expand exactly like a whole import (nested
                // imports resolve against the target's own directory,
                // vendor fallback included), but merge nothing flat:
                // Phase B renames and rewrites instead.
                let real_path =
                    ensure_parsed(&path, &raw, &vendor, &mut parsed, &mut path_of_idx, &mut files)?;
                let target = canonical(&real_path);
                let importer_canon = canonical(&importer);
                if expanded.insert(target.clone()) {
                    enqueue_imports(&parsed.get(&target).expect("just parsed").clone(), &real_path, &mut queue)?;
                }
                let edge = (
                    importer_canon,
                    alias.clone(),
                    target.clone(),
                    raw.clone(),
                );
                if !alias_edges.contains(&edge) {
                    alias_edges.push(edge);
                }
            }
        }
    }
    resolve_aliases(
        &parsed,
        &path_of_idx,
        &mut merged,
        &mut fn_origins,
        &mut mod_origins,
        &mut fn_names,
        &mut merged_items,
        &alias_edges,
    )?;
    Ok(LoadedProgram {
        program: merged,
        files,
    })
}

fn canonical(path: &str) -> String {
    std::path::Path::new(path)
        .canonicalize()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// Read + parse one file (once per canonical path), assigning it the next
/// file index for globally unique `NodeId` prefixes. When the relative
/// read fails and the import names a registry dependency
/// (`pkg/file.klang`), falls back to the pinned vendor directory (local
/// files always win — this only runs after the relative read failed).
/// Returns the REAL path that was read, so callers can base nested
/// imports on the vendored location (B1 fix).
fn resolve_real(path: &str, raw: &str, vendor: &Option<VendorCtx>) -> Option<String> {
    if std::path::Path::new(path).is_file() {
        return Some(path.to_string());
    }
    vendor_fallback(raw, vendor)
}

fn ensure_parsed(
    path: &str,
    raw: &str,
    vendor: &Option<VendorCtx>,
    parsed: &mut HashMap<String, Program>,
    path_of_idx: &mut Vec<String>,
    files: &mut Vec<LoadedFile>,
) -> Result<String, ImportError> {
    // Resolve the real file first so the canonical key is stable
    // across different logical importers of the same vendored file.
    let (real_path, src) = match std::fs::read_to_string(path) {
        Ok(src) => (path.to_string(), src),
        Err(first) => match vendor_fallback(raw, vendor) {
            Some(vpath) => match std::fs::read_to_string(&vpath) {
                Ok(src) => (vpath, src),
                Err(_) => {
                    return Err(ImportError::new(
                        "E-IO-NOT-FOUND",
                        format!(
                            "cannot read {}: {first}",
                            diagnostics::sanitize_for_terminal(path)
                        ),
                    ));
                }
            },
            None => {
                return Err(ImportError::new(
                    "E-IO-NOT-FOUND",
                    format!(
                        "cannot read {}: {first}",
                        diagnostics::sanitize_for_terminal(path)
                    ),
                ));
            }
        },
    };
    let canon_real = canonical(&real_path);
    if parsed.contains_key(&canon_real) {
        return Ok(real_path);
    }
    let mut p = Parser::new_with_file(&src, &real_path);
    let prog = p.parse_program().map_err(|d| ImportError::new("E-PARSE", d.to_json()))?;
    let idx = path_of_idx.len() as u32;
    path_of_idx.push(canon_real.clone());
    files.push(LoadedFile {
        path: real_path.clone(),
        bytes: src.len(),
    });
    parsed.insert(canon_real, prog.with_file_prefix(idx));
    Ok(real_path)
}

/// Vendor-dir fallback for one raw import string. Returns the vendored
/// path when the import's first segment names a registry dependency.
/// Returns `None` for ordinary relative imports (and for anything with
/// `..`/empty segments — those stay hard errors, never silent).
fn vendor_fallback(raw: &str, vendor: &Option<VendorCtx>) -> Option<String> {
    let ctx = vendor.as_ref()?;
    crate::registry::resolve_vendor_import(&ctx.root, &ctx.deps, raw)
        .map(|p| p.to_string_lossy().to_string())
}

/// Queue a file's own imports, resolved against its directory. Unsafe
/// paths are rejected here (same predicate as the repair model-output
/// screen), so untrusted files cannot pull in `../../…` or absolutes.
fn enqueue_imports(prog: &Program, from_path: &str, queue: &mut VecDeque<Work>) -> Result<(), ImportError> {
    let dir = std::path::Path::new(from_path)
        .parent()
        .map(|d| d.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    for imp in &prog.imports {
        if crate::repair::is_unsafe_import_path(imp) {
            return Err(ImportError::new(
                "E-IMPORT",
                format!(
                    "unsafe import `{}` from `{from_path}`",
                    diagnostics::sanitize_for_terminal(imp)
                ),
            ));
        }
        queue.push_back(Work::Whole {
            path: dir.join(imp).to_string_lossy().to_string(),
            importer: from_path.to_string(),
            raw: imp.clone(),
        });
    }
    for sel in &prog.selective_imports {
        if crate::repair::is_unsafe_import_path(&sel.path) {
            return Err(ImportError::new(
                "E-IMPORT",
                format!(
                    "unsafe import `{}` from `{from_path}`",
                    diagnostics::sanitize_for_terminal(&sel.path)
                ),
            ));
        }
        queue.push_back(Work::Selective {
            path: dir.join(&sel.path).to_string_lossy().to_string(),
            names: sel.names.clone(),
            importer: from_path.to_string(),
            raw: sel.path.clone(),
        });
    }
    for aliased in &prog.aliased_imports {
        if crate::repair::is_unsafe_import_path(&aliased.path) {
            return Err(ImportError::new(
                "E-IMPORT",
                format!(
                    "unsafe import `{}` from `{from_path}`",
                    diagnostics::sanitize_for_terminal(&aliased.path)
                ),
            ));
        }
        queue.push_back(Work::Alias {
            path: dir.join(&aliased.path).to_string_lossy().to_string(),
            alias: aliased.alias.clone(),
            importer: from_path.to_string(),
            raw: aliased.path.clone(),
        });
    }
    Ok(())
}

/// Merge one function, enforcing whole-program uniqueness. Re-merging the
/// same (file, name) is idempotent; a DIFFERENT file's same-named function
/// is a loud duplicate, never a silent overwrite.
fn insert_fn(
    merged: &mut Vec<crate::ast::FunctionDecl>,
    origins: &mut Vec<String>,
    fn_names: &mut HashSet<String>,
    merged_items: &mut HashSet<(String, String, String)>,
    canon: &str,
    f: crate::ast::FunctionDecl,
    _importer: &str,
) -> Result<(), ImportError> {
    if !fn_names.insert(f.name.clone()) {
        if merged_items.contains(&(canon.to_string(), "fn".to_string(), f.name.clone())) {
            return Ok(());
        }
        return Err(ImportError::new(
            "E-DUPLICATE",
            format!("duplicate function `{}`", f.name),
        ));
    }
    merged_items.insert((canon.to_string(), "fn".to_string(), f.name.clone()));
    origins.push(canon.to_string());
    merged.push(f);
    Ok(())
}

/// Merge one selectively-imported name (every top-level item with that
/// name, across functions/structs/enums).
fn import_named(
    target: &Program,
    canon: &str,
    name: &str,
    merged: &mut Program,
    fn_origins: &mut Vec<String>,
    fn_names: &mut HashSet<String>,
    merged_items: &mut HashSet<(String, String, String)>,
    importer: &str,
) -> Result<(), ImportError> {
    let mut hits = 0;
    for f in &target.functions {
        if f.name == name {
            hits += 1;
            insert_fn(
                &mut merged.functions,
                fn_origins,
                fn_names,
                merged_items,
                canon,
                f.clone(),
                importer,
            )?;
        }
    }
    for s in &target.structs {
        if s.name == name {
            hits += 1;
            let key = (canon.to_string(), "struct".to_string(), name.to_string());
            if !merged_items.insert(key) {
                continue;
            }
            merged.structs.push(s.clone());
        }
    }
    for e in &target.enums {
        if e.name == name {
            hits += 1;
            let key = (canon.to_string(), "enum".to_string(), name.to_string());
            if !merged_items.insert(key) {
                continue;
            }
            merged.enums.push(e.clone());
        }
    }
    if hits > 0 {
        return Ok(());
    }
    if target.mods.iter().any(|m| m.name == name) {
        return Err(ImportError::new(
            "E-IMPORT",
            format!(
                "undefined import `{name}` from `{importer}`: `{name}` is a module — import the whole file with `import \"...\"` instead"
            ),
        ));
    }
    Err(ImportError::new(
        "E-IMPORT",
        format!(
            "undefined import `{name}` from `{importer}`: no top-level function, struct, or enum named `{name}` (available: {})",
            available_names(target)
        ),
    ))
}

fn available_names(target: &Program) -> String {    let mut names: Vec<String> = Vec::new();
    names.extend(target.functions.iter().map(|f| f.name.clone()));
    names.extend(target.structs.iter().map(|s| s.name.clone()));
    names.extend(target.enums.iter().map(|e| e.name.clone()));
    if names.is_empty() {
        return "(none)".to_string();
    }
    names.sort();
    names.join(", ")
}

// ---------------------------------------------------------------------------
// D1 file aliases, Phase B: rename + rewrite (runs after the flat walk).
// ---------------------------------------------------------------------------

/// FNV-1a 64-bit (same hash family as the lockfile hasher).
fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Canonical internal name for one aliased function: `{orig}@{hex}`.
/// `@` is unspellable in user identifiers, so this can never collide with
/// a user-written name. The hash keys on canonical file identity — never
/// the alias spelling — so two aliases for one file share a single copy
/// (diamond-safe: file identity, not alias identity).
fn internal_name(canon: &str, orig: &str) -> String {
    format!("{orig}@{:016x}", fnv1a64(canon))
}

/// One resolved alias edge from the importer's point of view.
struct AliasFile {
    alias: String,
    target: String,
    raw: String,
    own_fns: Vec<String>,
}

/// Resolve every `import "p" as m` edge:
/// 1. reject alias/alias and alias/`let`-binding collisions loudly;
/// 2. merge the target's own functions under canonical internal names
///    (plus its structs/enums/mods flat, exactly as a whole import would);
/// 3. rewrite the importer's `m.f(...)` call sites to plain `Call`s.
///
/// Everything downstream (HIR sigs, MIR `Call`, runtime, JIT) then sees
/// ordinary functions — no checker, MIR, or runtime changes.
#[allow(clippy::too_many_arguments)]
fn resolve_aliases(
    parsed: &HashMap<String, Program>,
    path_of_idx: &[String],
    merged: &mut Program,
    fn_origins: &mut Vec<String>,
    mod_origins: &mut Vec<String>,
    fn_names: &mut HashSet<String>,
    merged_items: &mut HashSet<(String, String, String)>,
    alias_edges: &[(String, String, String, String)],
) -> Result<(), ImportError> {
    use crate::ast::{AliasDecl, AliasScope};
    let mut units_done: HashSet<String> = HashSet::new();
    let mut scopes: HashMap<u32, Vec<AliasDecl>> = HashMap::new();
    for (importer, alias, target, raw) in alias_edges {
        // Same alias bound to two different files in one importer.
        for (importer2, alias2, target2, _) in alias_edges {
            if importer == importer2 && alias == alias2 && target != target2 {
                return Err(ImportError::new(
                    "E-DUPLICATE",
                    format!(
                        "duplicate alias `{alias}` (already imports one file; cannot also import another)"
                    ),
                ));
            }
        }
        let target_prog = parsed.get(target).expect("alias target parsed").clone();
        let own_fns: Vec<String> = target_prog.functions.iter().map(|f| f.name.clone()).collect();
        let af = AliasFile {
            alias: alias.clone(),
            target: target.clone(),
            raw: raw.clone(),
            own_fns,
        };
        // Alias names share the binding scope with `let` names (D1 §4):
        // any `let`/parameter/loop variable with the alias name in the
        // importing file is a loud duplicate, never a silent pick-one.
        check_alias_collisions(merged, fn_origins, mod_origins, importer, alias)?;
        // Merge the target unit once per canonical file (two aliases for
        // one file share the copy): renamed functions plus flat
        // structs/enums/mods, exactly as a whole import would merge them.
        if units_done.insert(target.clone()) {
            for s in &target_prog.structs {
                merged.structs.push(s.clone());
            }
            for e in &target_prog.enums {
                merged.enums.push(e.clone());
            }
            for _ in &target_prog.mods {
                mod_origins.push(target.clone());
            }
            merged.mods.extend(target_prog.mods.clone());
            for f in &target_prog.functions {
                let mut renamed = f.clone();
                renamed.name = internal_name(target, &f.name);
                rewrite_self_calls(&mut renamed, &af);
                insert_renamed(merged, fn_origins, fn_names, merged_items, target, renamed)?;
            }
        }
        // Rewrite this importer's `alias.name(...)` call sites (in every
        // function or mod member originating from the importer file, flat
        // or renamed — both spell the alias the same way).
        rewrite_importer_calls(merged, fn_origins, mod_origins, importer, &af)?;
        // Record the scope for the checker's unknown-alias hint.
        if let Some(idx) = path_of_idx.iter().position(|p| p == importer) {
            scopes.entry(idx as u32).or_default().push(AliasDecl {
                alias: alias.clone(),
                target_raw: raw.clone(),
                target_canon: target.clone(),
            });
        }
        // Keep the merged program's import memory complete.
        let entry = crate::ast::AliasedImport {
            path: raw.clone(),
            alias: alias.clone(),
        };
        if !merged.aliased_imports.contains(&entry) {
            merged.aliased_imports.push(entry);
        }
    }
    let mut scopes: Vec<AliasScope> = scopes
        .into_iter()
        .map(|(file_idx, aliases)| AliasScope { file_idx, aliases })
        .collect();
    scopes.sort_by_key(|s| s.file_idx);
    merged.alias_scopes = scopes;
    Ok(())
}

/// Merge one renamed alias-target function. Renamed names carry `@`,
/// which user identifiers cannot spell, so they never collide with flat
/// names; the same (file, name) arriving twice (two aliases, one file)
/// hits the idempotency guard exactly like a diamond whole-import.
fn insert_renamed(
    merged: &mut Program,
    fn_origins: &mut Vec<String>,
    fn_names: &mut HashSet<String>,
    merged_items: &mut HashSet<(String, String, String)>,
    canon: &str,
    f: crate::ast::FunctionDecl,
) -> Result<(), ImportError> {
    if !fn_names.insert(f.name.clone()) {
        if merged_items.contains(&(canon.to_string(), "fn".to_string(), f.name.clone())) {
            return Ok(());
        }
        // Unreachable in practice (`@` names are unspellable), but loud
        // rather than silently overwriting if the hash ever collides.
        return Err(ImportError::new(
            "E-DUPLICATE",
            format!("duplicate function `{}`", f.name),
        ));
    }
    merged_items.insert((canon.to_string(), "fn".to_string(), f.name.clone()));
    fn_origins.push(canon.to_string());
    merged.functions.push(f);
    Ok(())
}

/// Reject an alias that shares its name with a `let` binding, parameter,
/// or loop variable anywhere in the importing file (D1 §4: aliases live
/// in the same binding scope as `let` names).
fn check_alias_collisions(
    merged: &Program,
    fn_origins: &[String],
    mod_origins: &[String],
    importer: &str,
    alias: &str,
) -> Result<(), ImportError> {
    for (f, origin) in merged.functions.iter().zip(fn_origins.iter()) {
        if origin != importer {
            continue;
        }
        for p in &f.params {
            if p.name == alias {
                return Err(ImportError::new(
                    "E-DUPLICATE",
                    format!(
                        "alias `{alias}` collides with parameter `{alias}` of function `{}` (aliases share the binding scope with `let` names; rename one)",
                        f.name
                    ),
                ));
            }
        }
        if let Some(binding) = find_let_binding(&f.body, alias) {
            return Err(ImportError::new(
                "E-DUPLICATE",
                format!(
                    "alias `{alias}` collides with {binding} in function `{}` (aliases share the binding scope with `let` names; rename one)",
                    f.name
                ),
            ));
        }
    }
    for (m, origin) in merged.mods.iter().zip(mod_origins.iter()) {
        if origin != importer {
            continue;
        }
        for f in &m.functions {
            for p in &f.params {
                if p.name == alias {
                    return Err(ImportError::new(
                        "E-DUPLICATE",
                        format!(
                            "alias `{alias}` collides with parameter `{alias}` of function `{}::{}` (aliases share the binding scope with `let` names; rename one)",
                            m.name, f.name
                        ),
                    ));
                }
            }
            if let Some(binding) = find_let_binding(&f.body, alias) {
                return Err(ImportError::new(
                    "E-DUPLICATE",
                    format!(
                        "alias `{alias}` collides with {binding} in function `{}::{}` (aliases share the binding scope with `let` names; rename one)",
                        m.name, f.name
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// First `let`/loop binding with `name` in the block, described for errors.
fn find_let_binding(block: &crate::ast::Block, name: &str) -> Option<String> {
    use crate::ast::Stmt;
    for s in &block.stmts {
        match s {
            Stmt::Let(l) if l.name == name => {
                return Some(format!("`let {name}`"));
            }
            Stmt::ForRange(fr) => {
                if fr.var == name {
                    return Some(format!("loop variable `{name}`"));
                }
                if let Some(b) = find_let_binding(&fr.body, name) {
                    return Some(b);
                }
            }
            Stmt::ForIn(fi) => {
                if fi.var == name {
                    return Some(format!("loop variable `{name}`"));
                }
                if let Some(b) = find_let_binding(&fi.body, name) {
                    return Some(b);
                }
            }
            Stmt::TaskGroup(g) => {
                if let Some(b) = find_let_binding(&g.body, name) {
                    return Some(b);
                }
            }
            Stmt::If(i) => {
                if let Some(b) = find_let_binding(&i.then_block, name) {
                    return Some(b);
                }
                if let Some(e) = &i.else_block {
                    if let Some(b) = find_let_binding(e, name) {
                        return Some(b);
                    }
                }
            }
            Stmt::While(w) => {
                if let Some(b) = find_let_binding(&w.body, name) {
                    return Some(b);
                }
            }
            Stmt::TryCatch(t) => {
                if let Some(b) = find_let_binding(&t.body, name)
                    .or_else(|| find_let_binding(&t.handler, name))
                {
                    return Some(b);
                }
            }
            _ => {}
        }
    }
    None
}

/// In a renamed alias-target copy, unqualified calls to the target's own
/// functions resolve in the home file (D1 §2): rewrite them to the
/// internal names. Anything else (builtins, flat names, unknowns) is left
/// for the checker exactly as a whole import would leave it, so an aliased
/// call executes exactly the same function value as the flat call would.
fn rewrite_self_calls(f: &mut crate::ast::FunctionDecl, af: &AliasFile) {
    rewrite_block_self(&mut f.body, af);
    // `return_ty`/param types are nominal strings; structs merge flat.
}

/// Rewrite one importer's `alias.name(...)` call sites to plain `Call`s
/// against the canonical internal names. Only functions originating from
/// the importer file are visited (flat or renamed — both spell the alias
/// the same way). `alias.unknown` is `E-UNDEFINED` naming what the target
/// file actually defines.
fn rewrite_importer_calls(
    merged: &mut Program,
    fn_origins: &[String],
    mod_origins: &[String],
    importer: &str,
    af: &AliasFile,
) -> Result<(), ImportError> {
    let mut fns: Vec<&mut crate::ast::FunctionDecl> = merged
        .functions
        .iter_mut()
        .zip(fn_origins.iter())
        .filter(|(_, origin)| *origin == importer)
        .map(|(f, _)| f)
        .collect();
    for f in fns.iter_mut() {
        rewrite_block_alias(&mut f.body, af)?;
    }
    let mut mods: Vec<&mut crate::ast::ModDecl> = merged
        .mods
        .iter_mut()
        .zip(mod_origins.iter())
        .filter(|(_, origin)| *origin == importer)
        .map(|(m, _)| m)
        .collect();
    for m in mods.iter_mut() {
        for f in m.functions.iter_mut() {
            rewrite_block_alias(&mut f.body, af)?;
        }
    }
    Ok(())
}

fn rewrite_block_self(block: &mut crate::ast::Block, af: &AliasFile) {
    for s in &mut block.stmts {
        rewrite_stmt_self(s, af);
    }
}

fn rewrite_stmt_self(s: &mut crate::ast::Stmt, af: &AliasFile) {
    use crate::ast::Stmt;
    match s {
        Stmt::Let(l) => rewrite_expr_self(&mut l.value, af),
        Stmt::Assign(a) => {
            match &mut a.target {
                crate::ast::AssignTarget::Var { .. } => {}
                crate::ast::AssignTarget::Index { base, index } => {
                    rewrite_expr_self(base, af);
                    rewrite_expr_self(index, af);
                }
                crate::ast::AssignTarget::Field { base, .. } => {
                    rewrite_expr_self(base, af)
                }
            }
            rewrite_expr_self(&mut a.value, af);
        }
        Stmt::Return(r) => rewrite_expr_self(&mut r.value, af),
        Stmt::TaskGroup(g) => rewrite_block_self(&mut g.body, af),
        Stmt::If(i) => {
            rewrite_expr_self(&mut i.cond, af);
            rewrite_block_self(&mut i.then_block, af);
            if let Some(e) = &mut i.else_block {
                rewrite_block_self(e, af);
            }
        }
        Stmt::Print(p) => rewrite_expr_self(&mut p.value, af),
        Stmt::While(w) => {
            rewrite_expr_self(&mut w.cond, af);
            rewrite_block_self(&mut w.body, af);
        }
        Stmt::ForRange(fr) => {
            rewrite_expr_self(&mut fr.start, af);
            rewrite_expr_self(&mut fr.end, af);
            rewrite_block_self(&mut fr.body, af);
        }
        Stmt::ForIn(fi) => {
            rewrite_expr_self(&mut fi.iter, af);
            rewrite_block_self(&mut fi.body, af);
        }
        Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::TryCatch(t) => {
            rewrite_block_self(&mut t.body, af);
            rewrite_block_self(&mut t.handler, af);
        }
        Stmt::Expr(e) => rewrite_expr_self(e, af),
    }
}

fn rewrite_expr_self(e: &mut crate::ast::Expr, af: &AliasFile) {
    use crate::ast::Expr;
    match e {
        Expr::Call { func, args, .. } => {
            if af.own_fns.iter().any(|f| f == func) {
                *func = internal_name(&af.target, func);
            }
            for a in args.iter_mut() {
                rewrite_expr_self(a, af);
            }
        }
        _ => rewrite_expr_children_self(e, af),
    }
}

fn rewrite_expr_children_self(e: &mut crate::ast::Expr, af: &AliasFile) {
    use crate::ast::Expr;
    match e {
        Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Str { .. }
        | Expr::Bool { .. }
        | Expr::Var { .. }
        | Expr::Await { .. } => {}
        Expr::ArrayLit { elems, .. } => {
            for el in elems {
                rewrite_expr_self(el, af);
            }
        }
        Expr::MapLit { entries, .. } => {
            for (_, v) in entries {
                rewrite_expr_self(v, af);
            }
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields.iter_mut() {
                rewrite_expr_self(v, af);
            }
        }
        Expr::EnumCtor { args, .. } => {
            for a in args.iter_mut() {
                rewrite_expr_self(a, af);
            }
        }
        Expr::Match { scrutinee, arms, .. } => {
            rewrite_expr_self(scrutinee, af);
            for arm in arms.iter_mut() {
                for s in arm.stmts.iter_mut() {
                    rewrite_stmt_self(s, af);
                }
                if let Some(g) = arm.guard.as_mut() {
                    rewrite_expr_self(g, af);
                }
                rewrite_expr_self(&mut arm.body, af);
            }
        }
        Expr::Closure { params: _, body, .. } => {
            rewrite_block_self(body, af);
        }
        Expr::Index { base, index, .. } => {
            rewrite_expr_self(base, af);
            rewrite_expr_self(index, af);
        }
        Expr::Field { base, .. } => rewrite_expr_self(base, af),
        Expr::MethodCall { base, args, .. } => {
            rewrite_expr_self(base, af);
            for a in args.iter_mut() {
                rewrite_expr_self(a, af);
            }
        }
        Expr::Spawn { call, .. } => rewrite_expr_self(call, af),
        Expr::Call { args, .. } => {
            for a in args.iter_mut() {
                rewrite_expr_self(a, af);
            }
        }
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
        | Expr::Or { left, right, .. } => {
            rewrite_expr_self(left, af);
            rewrite_expr_self(right, af);
        }
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => {
            rewrite_expr_self(inner, af);
        }
    }
}

fn rewrite_block_alias(
    block: &mut crate::ast::Block,
    af: &AliasFile,
) -> Result<(), ImportError> {
    for s in &mut block.stmts {
        rewrite_stmt_alias(s, af)?;
    }
    Ok(())
}

fn rewrite_stmt_alias(
    s: &mut crate::ast::Stmt,
    af: &AliasFile,
) -> Result<(), ImportError> {
    use crate::ast::Stmt;
    match s {
        Stmt::Let(l) => rewrite_expr_alias(&mut l.value, af),
        Stmt::Assign(a) => {
            match &mut a.target {
                crate::ast::AssignTarget::Var { .. } => {}
                crate::ast::AssignTarget::Index { base, index } => {
                    rewrite_expr_alias(base, af)?;
                    rewrite_expr_alias(index, af)?;
                }
                crate::ast::AssignTarget::Field { base, .. } => {
                    rewrite_expr_alias(base, af)?
                }
            }
            rewrite_expr_alias(&mut a.value, af)
        }
        Stmt::Return(r) => rewrite_expr_alias(&mut r.value, af),
        Stmt::TaskGroup(g) => rewrite_block_alias(&mut g.body, af),
        Stmt::If(i) => {
            rewrite_expr_alias(&mut i.cond, af)?;
            rewrite_block_alias(&mut i.then_block, af)?;
            if let Some(e) = &mut i.else_block {
                rewrite_block_alias(e, af)?;
            }
            Ok(())
        }
        Stmt::Print(p) => rewrite_expr_alias(&mut p.value, af),
        Stmt::While(w) => {
            rewrite_expr_alias(&mut w.cond, af)?;
            rewrite_block_alias(&mut w.body, af)
        }
        Stmt::ForRange(fr) => {
            rewrite_expr_alias(&mut fr.start, af)?;
            rewrite_expr_alias(&mut fr.end, af)?;
            rewrite_block_alias(&mut fr.body, af)
        }
        Stmt::ForIn(fi) => {
            rewrite_expr_alias(&mut fi.iter, af)?;
            rewrite_block_alias(&mut fi.body, af)
        }
        Stmt::Break(_) | Stmt::Continue(_) => Ok(()),
        Stmt::TryCatch(t) => {
            rewrite_block_alias(&mut t.body, af)?;
            rewrite_block_alias(&mut t.handler, af)
        }
        Stmt::Expr(e) => rewrite_expr_alias(e, af),
    }
}

fn rewrite_expr_alias(
    e: &mut crate::ast::Expr,
    af: &AliasFile,
) -> Result<(), ImportError> {
    use crate::ast::Expr;
    // `alias.name(args)` with an alias receiver becomes a plain `Call`
    // against the canonical internal name (same NodeId, so spans and
    // structural identity carry over). Anything else keeps its shape.
    if let Expr::MethodCall { id, base, method, args } = e {
        if let Expr::Var { name, .. } = base.as_ref() {
            if *name == af.alias {
                if !af.own_fns.iter().any(|f| f == method) {
                    let mut avail = af.own_fns.clone();
                    avail.sort();
                    return Err(ImportError::new(
                        "E-UNDEFINED",
                        format!(
                            "`{}.{}` is not defined in `{}` (available: {})",
                            af.alias,
                            method,
                            af.raw,
                            if avail.is_empty() {
                                "(none)".to_string()
                            } else {
                                avail.join(", ")
                            }
                        ),
                    ));
                }
                let internal = internal_name(&af.target, method);
                let id = id.clone();
                let mut new_args = std::mem::take(args);
                for a in new_args.iter_mut() {
                    rewrite_expr_alias(a, af)?;
                }
                *e = Expr::Call {
                    id,
                    func: internal,
                    type_args: Vec::new(),
                    args: new_args,
                };
                return Ok(());
            }
        }
    }
    rewrite_expr_children_alias(e, af)
}

fn rewrite_expr_children_alias(
    e: &mut crate::ast::Expr,
    af: &AliasFile,
) -> Result<(), ImportError> {
    use crate::ast::Expr;
    match e {
        Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::Str { .. }
        | Expr::Bool { .. }
        | Expr::Var { .. }
        | Expr::Await { .. } => Ok(()),
        Expr::ArrayLit { elems, .. } => {
            for el in elems {
                rewrite_expr_alias(el, af)?;
            }
            Ok(())
        }
        Expr::MapLit { entries, .. } => {
            for (_, v) in entries {
                rewrite_expr_alias(v, af)?;
            }
            Ok(())
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields.iter_mut() {
                rewrite_expr_alias(v, af)?;
            }
            Ok(())
        }
        Expr::EnumCtor { args, .. } => {
            for a in args.iter_mut() {
                rewrite_expr_alias(a, af)?;
            }
            Ok(())
        }
        Expr::Match { scrutinee, arms, .. } => {
            rewrite_expr_alias(scrutinee, af)?;
            for arm in arms.iter_mut() {
                for s in arm.stmts.iter_mut() {
                    rewrite_stmt_alias(s, af)?;
                }
                if let Some(g) = arm.guard.as_mut() {
                    rewrite_expr_alias(g, af)?;
                }
                rewrite_expr_alias(&mut arm.body, af)?;
            }
            Ok(())
        }
        Expr::Closure { body, .. } => rewrite_block_alias(body, af),
        Expr::Index { base, index, .. } => {
            rewrite_expr_alias(base, af)?;
            rewrite_expr_alias(index, af)
        }
        Expr::Field { base, .. } => rewrite_expr_alias(base, af),
        Expr::MethodCall { base, args, .. } => {
            rewrite_expr_alias(base, af)?;
            for a in args.iter_mut() {
                rewrite_expr_alias(a, af)?;
            }
            Ok(())
        }
        Expr::Spawn { call, .. } => rewrite_expr_alias(call, af),
        Expr::Call { args, .. } => {
            for a in args.iter_mut() {
                rewrite_expr_alias(a, af)?;
            }
            Ok(())
        }
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
        | Expr::Or { left, right, .. } => {
            rewrite_expr_alias(left, af)?;
            rewrite_expr_alias(right, af)
        }
        Expr::Not { inner, .. } | Expr::Neg { inner, .. } => {
            rewrite_expr_alias(inner, af)
        }
    }
}

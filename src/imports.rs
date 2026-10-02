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

    let mut queue: VecDeque<Work> = VecDeque::from([Work::Whole {
        path: entry.to_string(),
        importer: entry.to_string(),
        raw: entry.to_string(),
    }]);
    // Vendor context, computed once from the entry: a `klang.toml` found
    // by walking up from the entry file enables `pkg/file.klang`
    // fallback imports for registry dependencies.
    let vendor: Option<VendorCtx> = {
        let start = std::path::Path::new(entry);
        crate::registry::find_project_root(start).and_then(|root| {
            let text = std::fs::read_to_string(root.join("klang.toml")).ok()?;
            let manifest = crate::package::Manifest::parse(&text).ok()?;
            let deps: Vec<(String, String)> = manifest
                .deps
                .into_iter()
                .filter(|(_, v)| crate::registry::parse_registry_dep(v).is_some())
                .collect();
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
                let canon = canonical(&path);
                if whole_done.contains(&canon) {
                    continue;
                }
                ensure_parsed(&path, &raw, &vendor, &canon, &mut parsed, &mut path_of_idx, &mut files)?;
                // Mark BEFORE merging so a self-importing file terminates.
                whole_done.insert(canon.clone());
                let prog = parsed.get(&canon).expect("just parsed").clone();
                if expanded.insert(canon.clone()) {
                    enqueue_imports(&prog, &path, &mut queue)?;
                }
                merged.mods.extend(prog.mods);
                merged.enums.extend(prog.enums);
                merged.structs.extend(prog.structs);
                for f in prog.functions {
                    insert_fn(
                        &mut merged.functions,
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
                let canon = canonical(&path);
                ensure_parsed(&path, &raw, &vendor, &canon, &mut parsed, &mut path_of_idx, &mut files)?;
                let prog = parsed.get(&canon).expect("just parsed").clone();
                // The target's own imports still apply transitively (a
                // named function may need its file's other imports).
                if expanded.insert(canon.clone()) {
                    enqueue_imports(&prog, &path, &mut queue)?;
                }
                for name in &names {
                    import_named(
                        &prog,
                        &canon,
                        name,
                        &mut merged,
                        &mut fn_names,
                        &mut merged_items,
                        &importer,
                    )?;
                }
            }
        }
    }
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
fn ensure_parsed(
    path: &str,
    raw: &str,
    vendor: &Option<VendorCtx>,
    canon: &str,
    parsed: &mut HashMap<String, Program>,
    path_of_idx: &mut Vec<String>,
    files: &mut Vec<LoadedFile>,
) -> Result<(), ImportError> {
    if parsed.contains_key(canon) {
        return Ok(());
    }
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
    let mut p = Parser::new_with_file(&src, &real_path);
    let prog = p.parse_program().map_err(|d| ImportError::new("E-PARSE", d.to_json()))?;
    let idx = path_of_idx.len() as u32;
    path_of_idx.push(canon.to_string());
    files.push(LoadedFile {
        path: real_path,
        bytes: src.len(),
    });
    parsed.insert(canon.to_string(), prog.with_file_prefix(idx));
    Ok(())
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
    Ok(())
}

/// Merge one function, enforcing whole-program uniqueness. Re-merging the
/// same (file, name) is idempotent; a DIFFERENT file's same-named function
/// is a loud duplicate, never a silent overwrite.
fn insert_fn(
    merged: &mut Vec<crate::ast::FunctionDecl>,
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

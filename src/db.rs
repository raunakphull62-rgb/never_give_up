//! Stage 2: incremental query database on real Salsa.
//!
//! Inputs are file texts (`SourceFile`). Tracked queries are `parse_file`
//! and `check_file`. Salsa memoizes per query key and backdates via
//! `PartialEq` (`Program` and `Diagnostic` are both `Eq`): a comment-only
//! edit re-runs `parse` but its equal output stops `check` from re-running
//! (early cutoff). Structural `NodeId.path` stability is what keeps those
//! equality comparisons meaningful across revisions.
//!
//! The public `Database` API is unchanged from the std-only foundation,
//! so all existing callers keep working untouched.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use salsa::Setter;

use crate::ast::Program;
use crate::diagnostics::Diagnostic;
use crate::hir::TypedHIR;
use crate::parser::Parser;

#[salsa::db]
trait KlangDb: salsa::Database {
    fn parse_count(&self) -> &AtomicU64;
    fn check_count(&self) -> &AtomicU64;
}

/// One source file: name (for diagnostics) + text (the real input).
#[salsa::input]
struct SourceFile {
    name: String,
    text: String,
}

/// Tracked query: parse. Re-runs only when `text` (or `name`) changes.
#[salsa::tracked(returns(clone))]
fn parse_file(db: &dyn KlangDb, file: SourceFile) -> Result<Program, Diagnostic> {
    db.parse_count().fetch_add(1, Ordering::SeqCst);
    let text = file.text(db).clone();
    let name = file.name(db).clone();
    let mut p = Parser::new_with_file(&text, &name);
    p.parse_program()
}

/// Tracked query: effect check. Re-runs when `parse_file`'s result
/// compares unequal (backdating skips comment-only edits).
#[salsa::tracked(returns(clone))]
fn check_file(db: &dyn KlangDb, file: SourceFile) -> Result<(), Vec<Diagnostic>> {
    db.check_count().fetch_add(1, Ordering::SeqCst);
    match parse_file(db, file) {
        Ok(prog) => match TypedHIR::check(prog) {
            Ok(_) => Ok(()),
            Err(diags) => Err(diags),
        },
        Err(d) => Err(vec![d]),
    }
}

#[salsa::db]
#[derive(Default)]
struct SalsaDb {
    storage: salsa::Storage<Self>,
    parse_runs: AtomicU64,
    check_runs: AtomicU64,
}

#[salsa::db]
impl salsa::Database for SalsaDb {}

#[salsa::db]
impl KlangDb for SalsaDb {
    fn parse_count(&self) -> &AtomicU64 {
        &self.parse_runs
    }
    fn check_count(&self) -> &AtomicU64 {
        &self.check_runs
    }
}

/// Incremental database: file inputs + Salsa-tracked queries.
/// Same API as the old hand-rolled memo, now with real early cutoff.
#[derive(Default)]
pub struct Database {
    inner: SalsaDb,
    files: HashMap<String, SourceFile>,
    revision: u64,
}

impl Database {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set file content. Bumps revision only when content actually changed.
    /// (Salsa setters always record a change, so this guard stays
    /// load-bearing for the no-op-write case.)
    pub fn set_file(&mut self, name: &str, content: &str) -> u64 {
        if let Some(f) = self.files.get(name) {
            if f.text(&self.inner) == content {
                return self.revision;
            }
            f.set_text(&mut self.inner).to(content.to_string());
        } else {
            let f = SourceFile::new(&self.inner, name.to_string(), content.to_string());
            self.files.insert(name.to_string(), f);
        }
        self.revision += 1;
        self.revision
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn parse_runs(&self) -> u64 {
        self.inner.parse_runs.load(Ordering::SeqCst)
    }

    pub fn check_runs(&self) -> u64 {
        self.inner.check_runs.load(Ordering::SeqCst)
    }

    /// Missing files read as empty text, exactly like before.
    fn file_or_empty(&mut self, name: &str) -> SourceFile {
        if let Some(f) = self.files.get(name) {
            return *f;
        }
        let f = SourceFile::new(&self.inner, name.to_string(), String::new());
        self.files.insert(name.to_string(), f);
        f
    }

    /// Tracked query: parse.
    pub fn parse(&mut self, name: &str) -> Result<Program, Diagnostic> {
        let f = self.file_or_empty(name);
        parse_file(&self.inner, f)
    }

    /// Tracked query: effect check.
    /// Returns `Ok(())` when clean, `Err(diags)` otherwise.
    pub fn check(&mut self, name: &str) -> Result<(), Vec<Diagnostic>> {
        let f = self.file_or_empty(name);
        check_file(&self.inner, f)
    }
}

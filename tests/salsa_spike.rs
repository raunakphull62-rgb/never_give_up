//! Phase-0 spike: prove Salsa 0.28 mechanics on this toolchain.
//! Answers: custom fields on the db struct, `returns` modes, trait
//! ceremony, memo-hit, recompute-on-edit, and backdating (downstream
//! skipped when an upstream result compares equal).

use std::sync::atomic::{AtomicU64, Ordering};

use salsa::Setter;

#[salsa::db]
trait SpikeDb: salsa::Database {
    fn runs_words(&self) -> &AtomicU64;
    fn runs_has(&self) -> &AtomicU64;
}

#[salsa::input]
struct SpikeFile {
    text: String,
}

#[salsa::tracked]
fn spike_words(db: &dyn SpikeDb, file: SpikeFile) -> usize {
    db.runs_words().fetch_add(1, Ordering::SeqCst);
    file.text(db).split_whitespace().count()
}

#[salsa::tracked]
fn spike_has_words(db: &dyn SpikeDb, file: SpikeFile) -> bool {
    db.runs_has().fetch_add(1, Ordering::SeqCst);
    *spike_words(db, file) > 0
}

#[salsa::db]
#[derive(Default)]
struct SpikeDatabase {
    storage: salsa::Storage<Self>,
    runs_words: AtomicU64,
    runs_has: AtomicU64,
}

#[salsa::db]
impl salsa::Database for SpikeDatabase {}

#[salsa::db]
impl SpikeDb for SpikeDatabase {
    fn runs_words(&self) -> &AtomicU64 {
        &self.runs_words
    }
    fn runs_has(&self) -> &AtomicU64 {
        &self.runs_has
    }
}

#[test]
fn spike_memo_hit_no_rerun() {
    let db = SpikeDatabase::default();
    let f = SpikeFile::new(&db, "hi there".to_string());
    assert!(spike_has_words(&db, f));
    assert_eq!(db.runs_words.load(Ordering::SeqCst), 1);
    assert_eq!(db.runs_has.load(Ordering::SeqCst), 1);
    // Same revision: both memoized.
    assert!(spike_has_words(&db, f));
    assert_eq!(db.runs_words.load(Ordering::SeqCst), 1);
    assert_eq!(db.runs_has.load(Ordering::SeqCst), 1);
    println!("spike memo-hit OK");
}

#[test]
fn spike_backdate_skips_downstream() {
    let mut db = SpikeDatabase::default();
    let f = SpikeFile::new(&db, "hi".to_string());
    assert!(spike_has_words(&db, f));
    assert_eq!((db.runs_words.load(Ordering::SeqCst), db.runs_has.load(Ordering::SeqCst)), (1, 1));
    // Whitespace-only edit: words re-runs (input changed) but the count
    // is equal, so backdating skips the downstream query.
    f.set_text(&mut db).to("hi   ".to_string());
    assert!(spike_has_words(&db, f));
    assert_eq!((db.runs_words.load(Ordering::SeqCst), db.runs_has.load(Ordering::SeqCst)), (2, 1));
    // Real change flipping the result: downstream re-runs.
    f.set_text(&mut db).to("   ".to_string());
    assert!(!spike_has_words(&db, f));
    assert_eq!((db.runs_words.load(Ordering::SeqCst), db.runs_has.load(Ordering::SeqCst)), (3, 2));
    println!("spike backdating OK");
}

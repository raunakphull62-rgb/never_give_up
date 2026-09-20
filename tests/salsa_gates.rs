//! Salsa gates: early cutoff the old content-keyed memo could not do.
//! A comment-only edit re-runs `parse` (input changed) but its equal
//! `Program` backdates, so `check` does NOT re-run.

use klang::db::Database;

const SRC: &str = "fn main() -> i32 { return 42 }";

fn runs(db: &Database) -> (u64, u64) {
    (db.parse_runs(), db.check_runs())
}

#[test]
fn salsa_comment_edit_skips_check() {
    let mut db = Database::new();
    db.set_file("m.klang", SRC);
    assert!(db.check("m.klang").is_ok());
    assert_eq!(runs(&db), (1, 1));
    // Memo hit: re-check without edits re-runs nothing.
    assert!(db.check("m.klang").is_ok());
    assert_eq!(runs(&db), (1, 1));
    // Trailing-comment edit: parse re-runs (input changed) but no node
    // or byte span moves, so the equal `Program` backdates and `check`
    // is cut off. (Leading edits shift `name_span`s and still recheck;
    // widening that needs span-relative interning, a later phase.)
    db.set_file("m.klang", "fn main() -> i32 { return 42 } // trailing\n");
    assert!(db.check("m.klang").is_ok());
    assert_eq!(runs(&db), (2, 1), "check must be cut off, got {:?}", runs(&db));
    println!("salsa early-cutoff OK: {:?}", runs(&db));
}

#[test]
fn salsa_semantic_edit_runs_both() {
    let mut db = Database::new();
    db.set_file("m.klang", SRC);
    assert!(db.check("m.klang").is_ok());
    db.set_file("m.klang", "fn main() -> i32 { return 43 }");
    assert!(db.check("m.klang").is_ok());
    assert_eq!(runs(&db), (2, 2));
    println!("salsa semantic-edit OK: {:?}", runs(&db));
}

#[test]
fn salsa_errors_are_memoized() {
    let mut db = Database::new();
    db.set_file("b.klang", "fn main() -> i32 { return nope }");
    let err = db.check("b.klang").expect_err("must fail");
    assert!(err.iter().any(|d| d.code == "E-UNDEFINED"));
    let r = runs(&db);
    // Re-check without edits: memoized, no recompute.
    let err2 = db.check("b.klang").expect_err("must still fail");
    assert_eq!(err, err2);
    assert_eq!(runs(&db), r);
    // Parse errors memoize too.
    db.set_file("p.klang", "fn broken( -> ");
    assert!(db.check("p.klang").is_err());
    let r2 = runs(&db);
    assert!(db.check("p.klang").is_err());
    assert_eq!(runs(&db), r2);
    println!("salsa error-memo OK");
}

#[test]
fn salsa_files_invalidate_independently() {
    let mut db = Database::new();
    db.set_file("a.klang", "fn main() -> i32 { return 1 }");
    db.set_file("b.klang", "fn main() -> i32 { return 2 }");
    assert!(db.check("a.klang").is_ok());
    assert!(db.check("b.klang").is_ok());
    assert_eq!(runs(&db), (2, 2));
    // Edit only B: A stays memoized.
    db.set_file("b.klang", "fn main() -> i32 { return 3 }");
    assert!(db.check("b.klang").is_ok());
    assert_eq!(runs(&db), (3, 3));
    assert!(db.check("a.klang").is_ok());
    assert_eq!(runs(&db), (3, 3), "A must not recompute, got {:?}", runs(&db));
    println!("salsa independence OK");
}

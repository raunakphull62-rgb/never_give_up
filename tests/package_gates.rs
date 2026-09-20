use klang::package::{lock_for, parse_lock, verify_lock, write_lock, Manifest};

#[test]
fn manifest_parses_local_path_deps() {
    let m = Manifest::parse(
        "name = \"demo\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\nmylib = \"./mylib.klang\"\n",
    )
    .expect("parses");
    assert_eq!(m.name, "demo");
    assert_eq!(m.entry, "main");
    assert_eq!(m.deps, vec![("mylib".to_string(), "./mylib.klang".to_string())]);
    // backcompat: no deps section
    let m2 = Manifest::parse("name = \"demo\"\n").expect("parses");
    assert!(m2.deps.is_empty());
}

#[test]
fn lock_roundtrip_and_verify() {
    let files = vec![
        ("main.klang".to_string(), "fn main() -> i32 { return 42 }".to_string()),
        ("lib.klang".to_string(), "fn triple(n: i32) -> i32 { return n * 3 }".to_string()),
    ];
    let locks = lock_for(&files);
    let text = write_lock(&locks);
    let back = parse_lock(&text);
    assert_eq!(locks, back);
    assert!(verify_lock(&files, &back).is_ok());
}

#[test]
fn lock_detects_tamper_and_missing() {
    let files = vec![("main.klang".to_string(), "fn main() -> i32 { return 1 }".to_string())];
    let locks = lock_for(&files);
    let tampered = vec![("main.klang".to_string(), "fn main() -> i32 { return 2 }".to_string())];
    let err = verify_lock(&tampered, &locks).expect_err("tamper must fail");
    assert!(err.contains("hash mismatch"), "{err}");
    let err = verify_lock(&[], &locks).expect_err("missing must fail");
    assert!(err.contains("missing locked file"), "{err}");
}

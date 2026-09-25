//! Phase 10 — v2 package gates: offline manifest parsing.

use klang::package::manifest::V2Manifest;

const V2: &str = r#"
name = "demo"
version = "0.1.0"
entry = "main"
klang = "2"
[dependencies]
mylib = "./mylib.klang"
[schemas]
Profile = "1:deadbeef"
[repair]
max_iters = "5"
scope = "function"
"#;

#[test]
fn v2_package_full_manifest() {
    let m = V2Manifest::parse(V2).expect("parses");
    assert_eq!(m.name, "demo");
    assert_eq!(m.klang, "2");
    assert_eq!(
        m.deps,
        vec![("mylib".to_string(), "./mylib.klang".to_string())]
    );
    assert_eq!(m.schemas.len(), 1);
    assert_eq!(m.schemas[0].name, "Profile");
    assert_eq!(m.schemas[0].version, "1");
    assert_eq!(m.schemas[0].hash, "deadbeef");
    assert_eq!(m.repair.max_iters, Some(5));
    assert_eq!(m.repair.scope.as_deref(), Some("function"));
}

#[test]
fn v2_package_v1_compat_defaults() {
    let m = V2Manifest::parse("name = \"demo\"\nversion = \"0.1.0\"\nentry = \"main\"\n")
        .expect("parses");
    assert_eq!(m.klang, "1");
    assert!(m.schemas.is_empty());
    assert_eq!(m.repair, Default::default());
}

#[test]
fn v2_package_bad_locks_rejected() {
    assert!(V2Manifest::parse("[schemas]\nProfile = \"1:not-hex!!\"\n").is_err());
    assert!(V2Manifest::parse("[schemas]\nProfile = \"noversion\"\n").is_err());
    assert!(V2Manifest::parse("[repair]\nscope = \"galaxy\"\n").is_err());
    assert!(V2Manifest::parse("[repair]\nmax_iters = \"99\"\n").is_err());
}

#[test]
fn v2_package_v1_still_parses() {
    let m = klang::package::Manifest::parse(
        "name = \"demo\"\nversion = \"0.1.0\"\nentry = \"combine\"\n",
    )
    .expect("v1 parses");
    assert_eq!(m.entry, "combine");
}

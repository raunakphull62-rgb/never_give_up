//! KLANG-PHASE1 — package_manager module gates.
//!
//! Offline coverage for the PRD surface (`src/package_manager/`):
//! semver re-exports + full constraint syntax, manifest parse/write,
//! lock read/write/generate, resolver struct + graph + topo, registry
//! client checksum, installer file ops, CLI spec splitting. Network
//! paths (publish/fetch against a registry) stay in
//! `tests/registry_gates.rs`.

use std::collections::{HashMap, HashSet};

use klang::package::resolver::{GraphNode, ResolutionGraph};
use klang::package_manager::cli::split_spec;
use klang::package_manager::lock_file::{LockFile, PackageEntry};
use klang::package_manager::manifest::{parse_manifest, write_manifest};
use klang::package_manager::package_installer::PackageInstaller;
use klang::package_manager::registry_client::RegistryClient;
use klang::package_manager::resolver::DependencyResolver;
use klang::package_manager::semver::{
    matches_constraint, max_satisfying, parse_constraint, version_cmp,
};

// ---------------------------------------------------------------------------
// semver (PRD §4)
// ---------------------------------------------------------------------------

#[test]
fn pm_semver_full_syntax() {
    // Every PRD operator parses and matches.
    let cases: &[(&str, &str, bool)] = &[
        ("1.2.3", "1.2.3", true),
        ("=1.2.3", "1.2.3", true),
        ("=1.2.3", "1.2.4", false),
        ("^1.2.3", "1.9.0", true),
        ("^0.0.3", "0.0.4", false),
        ("~1.2.3", "1.2.9", true),
        ("~1.2.3", "1.3.0", false),
        (">1.0.0", "1.0.0", false),
        (">1.0.0", "1.0.1", true),
        (">=1.0.0", "1.0.0", true),
        ("<2.0.0", "2.0.0", false),
        ("<=2.0.0", "2.0.0", true),
        ("*", "7.7.7", true),
        ("latest", "0.0.1", true),
        (">=1.0.0 <2.0.0", "1.5.0", true),
        (">=1.0.0 <2.0.0", "2.0.0", false),
    ];
    for (req, ver, want) in cases {
        let c = parse_constraint(req).expect("parses");
        assert_eq!(matches_constraint(ver, &c), *want, "{req} vs {ver}");
    }
    let v = ["1.0.0", "1.5.0", "2.0.0"];
    let c = parse_constraint(">=1.0.0 <2.0.0").unwrap();
    assert_eq!(
        max_satisfying(v.iter().copied(), &c).as_deref(),
        Some("1.5.0")
    );
    assert!(version_cmp("1.2.3", "1.2.4") == std::cmp::Ordering::Less);
}

// ---------------------------------------------------------------------------
// manifest (PRD §3)
// ---------------------------------------------------------------------------

const FULL_MANIFEST: &str = r#"[project]
name = "my-app"
version = "1.0.0"
entry = "main"
description = "A sample Klang application"
authors = ["Raunak Phull", "Klang Core"]

[dependencies]
collections = "1.2.0"
http = "^0.5.0"
json = "~1.1.0"
crypto = ">=2.0.0"
time = "*"

[dev-dependencies]
test = "0.1.0"

[build]
target = "release"
optimization-level = 3
"#;

#[test]
fn pm_manifest_parses_prd_shape() {
    let dir = std::env::temp_dir().join("klang-pm-manifest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("klang.toml");
    std::fs::write(&path, FULL_MANIFEST).unwrap();
    let m = parse_manifest(&path).expect("parses");
    assert_eq!(m.name, "my-app");
    assert_eq!(m.description, "A sample Klang application");
    assert_eq!(m.authors, vec!["Raunak Phull".to_string(), "Klang Core".to_string()]);
    assert_eq!(m.deps.len(), 5);
    assert_eq!(m.dev_deps, vec![("test".to_string(), "0.1.0".to_string())]);
    assert_eq!(m.build.target, "release");
    assert_eq!(m.build.optimization_level, 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pm_manifest_write_roundtrip() {
    let m = klang::package::Manifest::parse(FULL_MANIFEST).expect("parses");
    let text = m.write_manifest();
    let back = klang::package::Manifest::parse(&text).expect("reparses");
    assert_eq!(m, back);
    // Deterministic: same bytes twice.
    assert_eq!(text, back.write_manifest());
}

#[test]
fn pm_manifest_write_minimal() {
    let dir = std::env::temp_dir().join("klang-pm-init");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // `klang init` shape: fresh manifests stay minimal but complete.
    let made = klang::registry::init_project(&dir, "demo-proj").expect("inits");
    assert!(made.contains("demo-proj"), "{made}");
    let m = parse_manifest(&dir.join("klang.toml")).expect("parses");
    assert_eq!(m.name, "demo-proj");
    assert!(m.deps.is_empty() && m.dev_deps.is_empty());
    assert!(dir.join("src/main.klang").is_file());
    assert!(dir.join("tests").is_dir());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// lock file (PRD §3: line format, per plan decision)
// ---------------------------------------------------------------------------

#[test]
fn pm_lock_read_write_find() {
    let dir = std::env::temp_dir().join("klang-pm-lock");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("klang.lock");
    // Legacy file-hash lines must survive a rewrite untouched.
    std::fs::write(&path, "main.klang 0123456789abcdef\n").unwrap();
    let mut lock = LockFile::read(&path).expect("reads");
    assert!(lock.packages.is_empty());
    lock.packages.push(PackageEntry::new(
        "collections",
        "1.2.0",
        &"a".repeat(64),
    ));
    lock.write(&path).expect("writes");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("main.klang 0123456789abcdef"), "{text}");
    assert!(
        text.contains(&format!("package collections 1.2.0 {}", "a".repeat(64))),
        "{text}"
    );
    let back = LockFile::read(&path).expect("rereads");
    let hit = back.find("collections", "1.2.0").expect("found");
    assert_eq!(hit.source, "registry");
    assert!(back.find("collections", "9.9.9").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pm_lock_from_resolution() {
    let graph = ResolutionGraph {
        packages: vec![
            GraphNode {
                name: "b".to_string(),
                version: "1.0.0".to_string(),
                deps: vec![],
            },
            GraphNode {
                name: "a".to_string(),
                version: "2.0.0".to_string(),
                deps: vec![("b".to_string(), "^1.0".to_string())],
            },
        ],
        root_deps: vec!["a".to_string()],
        install_order: vec![
            ("b".to_string(), "1.0.0".to_string()),
            ("a".to_string(), "2.0.0".to_string()),
        ],
    };
    let hashes: HashMap<(String, String), String> = [
        (
            ("a".to_string(), "2.0.0".to_string()),
            "a".repeat(64),
        ),
        (
            ("b".to_string(), "1.0.0".to_string()),
            "b".repeat(64),
        ),
    ]
    .into_iter()
    .collect();
    let lock = LockFile::from_resolution(&graph, &hashes).expect("valid hashes");
    // Sorted by name regardless of input order.
    assert_eq!(lock.packages[0].name, "a");
    assert_eq!(lock.packages[1].name, "b");
    assert_eq!(lock.packages[0].checksum, "a".repeat(64));
}

// ---------------------------------------------------------------------------
// resolver struct + graph (PRD §5)
// ---------------------------------------------------------------------------

struct Stub {
    pkgs: HashMap<String, Vec<(String, String)>>,
    deps: HashMap<(String, String), Vec<(String, String)>>,
}

impl klang::package::resolver::MetaSource for Stub {
    fn versions(&self, name: &str) -> Option<Vec<(String, String)>> {
        self.pkgs.get(name).cloned()
    }
    fn deps_for(&self, name: &str, version: &str) -> Vec<(String, String)> {
        self.deps
            .get(&(name.to_string(), version.to_string()))
            .cloned()
            .unwrap_or_default()
    }
}

fn demo_stub() -> Stub {
    let mut pkgs = HashMap::new();
    pkgs.insert(
        "http".to_string(),
        vec![("0.5.2".to_string(), "s-http".to_string())],
    );
    pkgs.insert(
        "collections".to_string(),
        vec![
            ("1.2.0".to_string(), "s-c12".to_string()),
            ("2.0.0".to_string(), "s-c20".to_string()),
        ],
    );
    pkgs.insert(
        "itertools".to_string(),
        vec![("1.0.5".to_string(), "s-it".to_string())],
    );
    let mut deps = HashMap::new();
    deps.insert(
        ("http".to_string(), "0.5.2".to_string()),
        vec![("collections".to_string(), "^1.0".to_string())],
    );
    deps.insert(
        ("collections".to_string(), "1.2.0".to_string()),
        vec![("itertools".to_string(), "^1.0".to_string())],
    );
    Stub { pkgs, deps }
}

#[test]
fn pm_resolver_struct_end_to_end() {
    let stub = demo_stub();
    let r = DependencyResolver::new(&stub);
    let roots = vec![("http".to_string(), "^0.5.0".to_string(), "root".to_string())];
    let pins = r.resolve(&roots).expect("resolves");
    assert_eq!(pins.len(), 3);
    // Max-satisfying within the http→collections ^1.0 edge.
    let coll = pins.iter().find(|p| p.name == "collections").unwrap();
    assert_eq!(coll.version, "1.2.0");
    // Graph carries edges + dependency-first install order.
    let g = r.resolve_graph(&roots).expect("graph");
    assert_eq!(g.root_deps, vec!["http".to_string()]);
    let order: Vec<&str> = g.install_order.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(order, vec!["itertools", "collections", "http"]);
    r.detect_conflicts(&g).expect("clean");
    // Wrapper topo matches the graph order (B3: returns Result now).
    assert_eq!(r.topological_sort(&g.packages).expect("sorts"), g.install_order);
}

#[test]
fn pm_resolver_conflict_message() {
    let stub = demo_stub();
    let r = DependencyResolver::new(&stub);
    let err = r
        .resolve(&[
            ("collections".to_string(), "1.2.0".to_string(), "root".to_string()),
            ("collections".to_string(), "2.0.0".to_string(), "other".to_string()),
        ])
        .expect_err("conflicts");
    assert!(err.contains("version conflict"), "{err}");
}

#[test]
fn pm_detect_conflicts_drift_shape() {
    // PRD §5 error shape: "Conflict: A needs json@^1.0, ...".
    let graph = ResolutionGraph {
        packages: vec![
            GraphNode {
                name: "a".to_string(),
                version: "1.0.0".to_string(),
                deps: vec![("json".to_string(), "^1.0".to_string())],
            },
            GraphNode {
                name: "json".to_string(),
                version: "2.0.0".to_string(),
                deps: vec![],
            },
        ],
        root_deps: vec!["a".to_string()],
        install_order: vec![],
    };
    let stub = demo_stub();
    let err = DependencyResolver::new(&stub)
        .detect_conflicts(&graph)
        .expect_err("drift");
    assert!(err.contains("Conflict: a needs json@^1.0"), "{err}");
}

// ---------------------------------------------------------------------------
// registry client (offline surface)
// ---------------------------------------------------------------------------

#[test]
fn pm_registry_client_checksum() {
    let c = RegistryClient::new("http://127.0.0.1:9");
    assert_eq!(c.base_url(), "http://127.0.0.1:9");
    let data = b"abc";
    let sha = klang::registry::sha256_hex(data);
    assert!(c.verify_checksum(data, &sha).is_ok());
    assert!(c.verify_checksum(data, &"0".repeat(64)).is_err());
}

// ---------------------------------------------------------------------------
// installer file ops (offline)
// ---------------------------------------------------------------------------

#[test]
fn pm_installer_extract_link_cleanup() {
    let root = std::env::temp_dir().join("klang-pm-installer");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let inst = PackageInstaller::new(&root);
    assert_eq!(inst.root(), root.as_path());
    // Build a tiny archive with the real packer, then extract it.
    let files = vec![("lib.klang".to_string(), b"fn f() -> i32 { return 1 }".to_vec())];
    let archive = klang::registry::pack_archive(&files).expect("packs");
    inst.extract_to_cache(&archive, "demo", "1.0.0")
        .expect("extracts");
    let lib = root.join(".klang_pkgs/demo/1.0.0/lib.klang");
    assert!(lib.is_file());
    // link_or_copy: hard-link or copy, then cleanup removes the dir.
    let dest = root.join("linked.klang");
    inst.link_or_copy(&lib, &dest).expect("links");
    assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(&lib).unwrap());
    inst.cleanup_on_error("demo", "1.0.0");
    assert!(!root.join(".klang_pkgs/demo/1.0.0").exists());
    // Bad names never touch the disk.
    assert!(inst.extract_to_cache(&archive, "../evil", "1.0.0").is_err());
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------
// cli spec splitting
// ---------------------------------------------------------------------------

#[test]
fn pm_cli_split_spec() {
    assert_eq!(split_spec("collections"), ("collections".to_string(), "latest".to_string()));
    assert_eq!(
        split_spec("collections@1.2.0"),
        ("collections".to_string(), "1.2.0".to_string())
    );
    assert_eq!(
        split_spec("http@>=1.0.0 <2.0.0"),
        ("http".to_string(), ">=1.0.0 <2.0.0".to_string())
    );
    // Trailing `@` means latest (never an empty constraint).
    assert_eq!(split_spec("x@"), ("x".to_string(), "latest".to_string()));
}

// ---------------------------------------------------------------------------
// stdlib package layout sanity (PRD §7)
// ---------------------------------------------------------------------------

#[test]
fn pm_stdlib_layout() {
    // Every stdlib package ships klang.toml + root lib + root self-test
    // + src modules, with a valid X.Y.Z manifest name/version.
    let pkgs = [
        "collections", "io", "string", "math", "time", "json", "http", "crypto", "sql",
        "regex", "compress", "net", "sync", "logging", "testing", "itertools",
        "fs", "path", "os", "random", "csv", "text",
        "cli", "base64", "hex", "url",
    ];
    let mut seen = HashSet::new();
    for name in pkgs {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("stdlib-packages")
            .join(name);
        let toml = std::fs::read_to_string(dir.join("klang.toml"))
            .unwrap_or_else(|_| panic!("{name}: klang.toml missing"));
        let m = klang::package::Manifest::parse(&toml).expect("manifest parses");
        assert_eq!(m.name, name, "manifest name matches dir");
        assert!(klang::registry::valid_version(&m.version), "X.Y.Z");
        assert!(dir.join("lib.klang").is_file(), "{name}: lib.klang");
        assert!(
            dir.join(format!("{name}_test.klang")).is_file(),
            "{name}: self-test"
        );
        assert!(dir.join("src").is_dir(), "{name}: src/");
        assert!(dir.join("README.md").is_file(), "{name}: README");
        assert!(seen.insert(name), "unique");
    }
    assert_eq!(seen.len(), 26);
}

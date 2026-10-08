//! D1 file aliases: `import "path" as m` with `m.f(...)` call sites.
//!
//! The loader resolves the file (relative + vendor fallback, own imports
//! against its own directory) but merges nothing flat: the target's
//! functions land under canonical internal names and the importer's
//! `m.f(...)` sites rewrite to plain `Call`s. Everything downstream
//! (HIR sigs, MIR, runtime, JIT) sees ordinary functions. Aliases are
//! file-local; `as` is contextual; selective imports cannot take one.
//!
//! The silent failure here would be a call resolving to the WRONG item
//! (overwrite instead of duplicate error, home-file scoping violated),
//! so several tests pin exact values around collisions, diamonds, and
//! nested/vendor resolution.

use std::collections::HashMap;
use std::path::PathBuf;

use klang::parser::Parser;

fn write_dir(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "klang-alias-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }
    dir
}

fn load_ok(dir: &PathBuf, entry: &str) -> klang::ast::Program {
    let path = dir.join(entry).to_string_lossy().to_string();
    klang::imports::load_program(&path)
        .unwrap_or_else(|e| panic!("load failed: {}", e.to_json()))
        .program
}

fn load_err(dir: &PathBuf, entry: &str) -> klang::imports::ImportError {
    let path = dir.join(entry).to_string_lossy().to_string();
    klang::imports::load_program(&path).expect_err("load must fail")
}

fn check_ok(prog: &klang::ast::Program) {
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
}

fn check_and_run(prog: &klang::ast::Program) -> (i32, Vec<String>) {
    check_ok(prog);
    let mir = klang::mir::lower(prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs")
}

/// Count MIR functions whose name is an internal alias renaming of `orig`
/// (`{orig}@{hex}`): proves file-identity (one copy) vs alias-identity.
fn renamed_copies(prog: &klang::ast::Program, orig: &str) -> usize {
    let mir = klang::mir::lower(prog);
    let prefix = format!("{orig}@");
    mir.functions
        .iter()
        .filter(|f| f.name.starts_with(&prefix))
        .count()
}

#[test]
fn two_files_same_fn_both_callable() {
    let dir = write_dir(
        "two-files",
        &[
            (
                "mathlib.klang",
                "fn max(a: i32, b: i32) -> i32 { if a < b { return b } return a }\nfn helper() -> i32 { return max(20, 21) }\n",
            ),
            (
                "otherlib.klang",
                "fn max(a: i32, b: i32) -> i32 { return a + b }\n",
            ),
            (
                "main.klang",
                "import \"mathlib.klang\" as m\nimport \"otherlib.klang\" as o\nfn main() -> i32 { print(m.max(1, 2)) print(o.max(1, 2)) print(m.helper()) return 0 }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, out) = check_and_run(&prog);
    assert_eq!(v, 0);
    assert_eq!(out, vec!["2".to_string(), "3".to_string(), "21".to_string()]);
    println!("two-file alias OK");
}

#[test]
fn same_file_two_aliases_share_one_copy() {
    let dir = write_dir(
        "two-aliases",
        &[
            ("c.klang", "fn f() -> i32 { return 40 }\n"),
            (
                "main.klang",
                "import \"c.klang\" as m\nimport \"c.klang\" as m2\nfn main() -> i32 { return m.f() + m2.f() }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    check_ok(&prog);
    assert_eq!(renamed_copies(&prog, "f"), 1, "one copy, two views");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 80);
    println!("two-aliases-one-copy OK");
}

#[test]
fn same_alias_two_files_is_duplicate() {
    let dir = write_dir(
        "dup-alias",
        &[
            ("a.klang", "fn f() -> i32 { return 1 }\n"),
            ("b.klang", "fn f() -> i32 { return 2 }\n"),
            (
                "main.klang",
                "import \"a.klang\" as m\nimport \"b.klang\" as m\nfn main() -> i32 { return 0 }\n",
            ),
        ],
    );
    let e = load_err(&dir, "main.klang");
    assert_eq!(e.code, "E-DUPLICATE", "{}", e.to_json());
    assert!(e.message.contains("`m`"), "names the alias: {}", e.to_json());
    println!("dup-alias OK");
}

#[test]
fn missing_aliased_file_is_eio_not_found() {
    let dir = write_dir(
        "missing",
        &[(
            "main.klang",
            "import \"ghost.klang\" as g\nfn main() -> i32 { return 0 }\n",
        )],
    );
    let e = load_err(&dir, "main.klang");
    assert_eq!(e.code, "E-IO-NOT-FOUND", "{}", e.to_json());
    println!("missing-alias-target OK");
}

#[test]
fn unknown_item_names_available() {
    let dir = write_dir(
        "unknown-item",
        &[
            (
                "mathlib.klang",
                "fn max(a: i32, b: i32) -> i32 { if a < b { return b } return a }\nfn helper() -> i32 { return 1 }\n",
            ),
            (
                "main.klang",
                "import \"mathlib.klang\" as m\nfn main() -> i32 { return m.nope(1) }\n",
            ),
        ],
    );
    let e = load_err(&dir, "main.klang");
    assert_eq!(e.code, "E-UNDEFINED", "{}", e.to_json());
    assert!(e.message.contains("`m.nope`"), "{}", e.to_json());
    assert!(e.message.contains("helper") && e.message.contains("max"), "{}", e.to_json());
    println!("unknown-item OK");
}

#[test]
fn unknown_alias_suggests_available_aliases() {
    let dir = write_dir(
        "unknown-alias",
        &[
            ("mathlib.klang", "fn max(a: i32, b: i32) -> i32 { return a }\n"),
            (
                "main.klang",
                "import \"mathlib.klang\" as m\nfn main() -> i32 { return n.max(1, 2) }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    let diags = klang::hir::TypedHIR::check(prog).expect_err("unknown alias fails");
    let d = diags.iter().find(|d| d.code == "E-UNDEFINED").expect("E-UNDEFINED");
    assert!(d.message.contains("`n`"), "names the receiver: {}", d.to_json());
    assert!(
        d.fixes.iter().any(|f| f.label.contains("`m`")),
        "suggests the available alias: {}",
        d.to_json()
    );
    println!("unknown-alias hint OK");
}

#[test]
fn alias_vs_let_and_param_collisions_are_duplicate() {
    for (tag, main) in [
        (
            "let",
            "import \"c.klang\" as m\nfn main() -> i32 { let m = 1 return m }\n",
        ),
        (
            "param",
            "import \"c.klang\" as m\nfn f(m: i32) -> i32 { return m }\nfn main() -> i32 { return f(1) }\n",
        ),
    ] {
        let dir = write_dir(
            &format!("collide-{tag}"),
            &[
                ("c.klang", "fn f() -> i32 { return 1 }\n"),
                ("main.klang", main),
            ],
        );
        let e = load_err(&dir, "main.klang");
        assert_eq!(e.code, "E-DUPLICATE", "{tag}: {}", e.to_json());
        assert!(e.message.contains("`m`"), "{tag}: {}", e.to_json());
    }
    println!("alias collisions OK");
}

#[test]
fn nested_relative_imports_resolve_in_target_dir() {
    let dir = write_dir(
        "nested",
        &[
            (
                "sub/lib.klang",
                "import \"helper.klang\"\nfn val() -> i32 { return helper() + 1 }\n",
            ),
            ("sub/helper.klang", "fn helper() -> i32 { return 41 }\n"),
            (
                "main.klang",
                "import \"sub/lib.klang\" as s\nfn main() -> i32 { return s.val() }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 42);
    println!("nested-alias OK");
}

#[test]
fn vendored_nested_alias_resolves_in_vendor_dir() {
    // A user file aliases a vendored package file whose own relative
    // import must resolve inside `.klang_pkgs/<name>/<version>/`, never
    // against the user directory (D1 §4). In-process load (like the
    // registry gates): no network, integrity is a fetch-path concern.
    let dir = write_dir(
        "vendored",
        &[
            ("klang.toml", "name = \"app\"\nversion = \"0.1.0\"\nentry = \"main\"\n[dependencies]\nos = \"registry:os@1.0.0\"\n"),
            ("klang.lock", "package os 1.0.0 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"),
            (
                ".klang_pkgs/os/1.0.0/lib.klang",
                "import \"helper.klang\"\nfn osval() -> i32 { return helper() + 1 }\n",
            ),
            (".klang_pkgs/os/1.0.0/helper.klang", "fn helper() -> i32 { return 41 }\n"),
            (
                "main.klang",
                "import \"os/lib.klang\" as os\nfn main() -> i32 { return os.osval() }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 42);
    println!("vendored-alias OK");
}

#[test]
fn diamond_alias_shares_one_copy() {
    let dir = write_dir(
        "diamond",
        &[
            ("c.klang", "fn f() -> i32 { return 40 }\n"),
            ("a.klang", "import \"c.klang\" as m\nfn fa() -> i32 { return m.f() }\n"),
            (
                "b.klang",
                "import \"c.klang\" as n\nfn fb() -> i32 { return n.f() + 1 }\n",
            ),
            (
                "main.klang",
                "import \"a.klang\"\nimport \"b.klang\"\nfn main() -> i32 { return fa() + fb() }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    check_ok(&prog);
    assert_eq!(renamed_copies(&prog, "f"), 1, "file identity, not alias identity");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 81);
    println!("diamond OK");
}

#[test]
fn checker_errors_flow_through_aliases() {
    let dir = write_dir(
        "checker",
        &[
            (
                "lib.klang",
                "fn add(a: i32, b: i32) -> i32 { return a + b }\nfn boom() -> i32 throws { return 1 }\nfn same<T>(a: T, b: T) -> T { return a }\n",
            ),
            (
                "arity.klang",
                "import \"lib.klang\" as l\nfn main() -> i32 { return l.add(1) }\n",
            ),
            (
                "type.klang",
                "import \"lib.klang\" as l\nfn main() -> i32 { return l.add(\"hi\", 1) }\n",
            ),
            (
                "effect.klang",
                "import \"lib.klang\" as l\nfn main() -> i32 { return l.boom() }\n",
            ),
            (
                "generic.klang",
                "import \"lib.klang\" as l\nfn main() -> i32 { return l.same(20, 22) }\n",
            ),
        ],
    );
    for (entry, code) in [("arity.klang", "E-ARITY"), ("type.klang", "E-TYPE")] {
        let prog = load_ok(&dir, entry);
        let diags = klang::hir::TypedHIR::check(prog).expect_err("must fail");
        assert!(diags.iter().any(|d| d.code == code), "{entry}: {code}");
    }
    let prog = load_ok(&dir, "effect.klang");
    let diags = klang::hir::TypedHIR::check(prog).expect_err("throws must fail");
    assert!(diags.iter().any(|d| d.code == "E-EFFECT-MISMATCH"));
    let prog = load_ok(&dir, "generic.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 20);
    println!("checker-through-alias OK");
}

#[test]
fn alias_does_not_shadow_method_calls_or_fields() {
    // Precedence finding: alias recognition keys ONLY on a bare `m.`
    // receiver spelling the alias. `s.m()` on a let-bound `s`, and `p.m`
    // field access on a struct value, are untouched by the rewrite.
    let dir = write_dir(
        "precedence",
        &[
            ("c.klang", "fn f() -> i32 { return 40 }\n"),
            (
                "main.klang",
                "struct S { m: i32 }\nimport \"c.klang\" as m\nfn get(p: S) -> i32 { return p.m }\nfn main() -> i32 { let s = \"hi\" let n = s.len() let p = S { m: 41 } return get(p) + m.f() + n }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, out) = check_and_run(&prog);
    assert_eq!(v, 83, "41 (field) + 40 (alias) + 2 (method)");
    assert!(out.is_empty());
    println!("precedence OK");
}

#[test]
fn as_is_contextual_and_selective_alias_rejected() {
    // A function literally named `as` still parses and runs.
    let mut p = Parser::new("fn as() -> i32 { return 1 }\nfn main() -> i32 { return as() }\n");
    let prog = p.parse_program().expect("`as` is contextual");
    let mir = klang::mir::lower(&prog);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 1);
    // Selective + alias is a clear parse error (aliases expose whole files).
    let mut p = Parser::new(
        "import { max } from \"m.klang\" as m\nfn main() -> i32 { return 0 }\n",
    );
    let d = p.parse_program().expect_err("selective+alias fails");
    assert_eq!(d.code, "E-PARSE", "{}", d.to_json());
    assert!(d.message.contains("cannot take an alias"), "{}", d.to_json());
    println!("contextual-as OK");
}

#[test]
fn fmt_round_trips_aliased_imports() {
    let src = "import \"b.klang\"\nimport \"m.klang\" as m\nimport { x } from \"s.klang\"\nfn main() -> i32 { return m.f() + x }\n";
    let prog = Parser::new(src).parse_program().expect("parses");
    let once = klang::fmt::fmt_program(&prog);
    assert!(once.contains("import \"m.klang\" as m\n"), "alias line kept:\n{once}");
    let twice = klang::fmt::fmt_program(
        &Parser::new(&once).parse_program().expect("re-parses"),
    );
    assert_eq!(once, twice, "fmt idempotent");
    println!("fmt-alias OK");
}

#[test]
fn whole_and_alias_are_two_views() {
    let dir = write_dir(
        "two-views",
        &[
            ("c.klang", "fn f() -> i32 { return 7 }\n"),
            (
                "main.klang",
                "import \"c.klang\"\nimport \"c.klang\" as m\nfn main() -> i32 { return f() + m.f() }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 14);
    println!("two-views OK");
}

#[test]
fn jit_runs_aliased_int_calls() {
    let dir = write_dir(
        "jit-alias",
        &[
            (
                "mathlib.klang",
                "fn max(a: i32, b: i32) -> i32 { if a < b { return b } return a }\n",
            ),
            (
                "main.klang",
                "import \"mathlib.klang\" as m\nfn main() -> i32 { return m.max(20, 22) }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    check_ok(&prog);
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("interp runs");
    assert_eq!(v, 22);
    let (jv, jout) = klang::jit::run_jit(&mir, "main", &[]).expect("jit runs");
    assert_eq!(jv as i32, v, "jit value == interp value");
    assert_eq!(jout, out, "jit output == interp output");
    println!("jit-alias OK");
}

#[test]
fn v2_has_no_imports_positive_control() {
    // The v2 grammar has no import syntax at all, so an aliased program
    // is rejected at the v2 boundary (parity harness allowlists this as
    // E-PARSE-V2; asserted here at the unit level).
    let d = klang::parser::v2::parse_v2_program(
        "import \"m.klang\" as m\nfn main() -> i32 { return 0 }\n",
    )
    .expect_err("v2 rejects imports");
    assert_eq!(d.code, "E-PARSE-V2", "{}", d.to_json());
    println!("v2-exclusion OK");
}

#[test]
fn alias_calls_work_from_mods_and_closures() {
    let dir = write_dir(
        "mod-closure",
        &[
            ("c.klang", "fn f() -> i32 { return 40 }\n"),
            (
                "main.klang",
                "import \"c.klang\" as m\nmod u { pub fn h() -> i32 { return m.f() + 1 } }\nfn main() -> i32 { let g = fn(x: i32) -> i32 { return x + m.f() } return u::h() + g(1) }\n",
            ),
        ],
    );
    let prog = load_ok(&dir, "main.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 41 + 41);
    println!("mod-closure alias OK");
}

#[test]
fn lsp_entry_source_path_resolves_aliases() {
    // The language server checks the dirty buffer through
    // `load_program_with_entry_source` (same loader, same checker): an
    // aliased program must load there too.
    let dir = write_dir(
        "lsp-path",
        &[
            ("c.klang", "fn f() -> i32 { return 40 }\n"),
            (
                "main.klang",
                "import \"c.klang\" as m\nfn main() -> i32 { return m.f() }\n",
            ),
        ],
    );
    let path = dir.join("main.klang").to_string_lossy().to_string();
    let src = std::fs::read_to_string(&path).unwrap();
    let loaded = klang::imports::load_program_with_entry_source(&path, &src)
        .unwrap_or_else(|e| panic!("lsp-path load failed: {}", e.to_json()));
    let (v, _) = check_and_run(&loaded.program);
    assert_eq!(v, 40);
    println!("lsp-path alias OK");
}

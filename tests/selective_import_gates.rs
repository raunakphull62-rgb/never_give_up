//! KLANG-FOUNDATION-2 Step 4 — selective multi-file imports.
//!
//! `import { name, ... } from "file.klang"` merges only the named
//! top-level items (function/struct/enum) from the target file, alongside
//! the existing whole-file `import "file.klang"` merge. Missing files are
//! `E-IO-NOT-FOUND`, missing names / unsafe paths / duplicates are
//! `E-IMPORT` / `E-DUPLICATE` — always clean `E-*` diagnostics, never a
//! panic. Circular imports terminate (cycle-safe BFS: each file merges at
//! most once per mode) instead of hanging or overflowing the compiler.
//!
//! Backend coverage note: imports resolve at load time, before either
//! backend sees the program, so "v2 backend" coverage is the documented
//! exclusion (v2 has no imports) plus a positive control, and the JIT
//! transparently runs merged int-only programs.
//!
//! BUG-A discipline: the silent failure here would be a name resolving
//! to the WRONG item (overwrite instead of duplicate error, or an
//! un-imported sibling silently visible), so several tests pin exact
//! errors/values around name collisions and visibility boundaries.

use std::collections::HashMap;
use std::path::PathBuf;

use klang::parser::Parser;

fn write_dir(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("klang-sel-{tag}"));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, content) in files {
        std::fs::write(dir.join(name), content).unwrap();
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

fn check_and_run(prog: &klang::ast::Program) -> (i32, Vec<String>) {
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(prog);
    klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs")
}

#[test]
fn selective_fn_import_runs() {
    // Basic case: file B imports one function by name and runs it.
    let dir = write_dir(
        "fn",
        &[
            ("a.klang", "fn triple(n: i32) -> i32 { return n * 3 }"),
            (
                "b.klang",
                "import { triple } from \"a.klang\"\nfn main() -> i32 { return triple(14) }",
            ),
        ],
    );
    let prog = load_ok(&dir, "b.klang");
    assert_eq!(prog.functions.len(), 2, "only triple + main merged");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 42);
}

#[test]
fn selective_struct_import_runs() {
    let dir = write_dir(
        "struct",
        &[
            (
                "a.klang",
                "struct Point { x: i32, y: i32 }\nfn ignored() -> i32 { return 0 }",
            ),
            (
                "b.klang",
                "import { Point } from \"a.klang\"\nfn main() -> i32 { let p = Point { x: 40, y: 2 } return p.x + p.y }",
            ),
        ],
    );
    let prog = load_ok(&dir, "b.klang");
    assert!(prog.functions.iter().any(|f| f.name == "main"));
    assert!(!prog.functions.iter().any(|f| f.name == "ignored"));
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 42);
}

#[test]
fn selective_enum_import_runs() {
    let dir = write_dir(
        "enum",
        &[
            ("a.klang", "enum Opt { Some(x: i32), None }"),
            (
                "b.klang",
                "import { Opt } from \"a.klang\"\nfn main() -> i32 { let v = Opt::Some(41) return match v { Opt::Some(n) => n + 1, Opt::None => 0 } }",
            ),
        ],
    );
    let prog = load_ok(&dir, "b.klang");
    let (v, _) = check_and_run(&prog);
    assert_eq!(v, 42);
}

#[test]
fn selective_multi_name_import() {
    // Several names in one list all merge.
    let dir = write_dir(
        "multi",
        &[
            ("a.klang", "fn t(n: i32) -> i32 { return n * 3 }\nfn u(n: i32) -> i32 { return n + 1 }"),
            (
                "b.klang",
                "import { t, u } from \"a.klang\"\nfn main() -> i32 { return t(13) + u(2) }",
            ),
        ],
    );
    let (v, _) = check_and_run(&load_ok(&dir, "b.klang"));
    assert_eq!(v, 42, "39 + 3");
}

#[test]
fn missing_file_is_eio_not_found() {
    let dir = write_dir(
        "nofile",
        &[(
            "b.klang",
            "import { triple } from \"ghost.klang\"\nfn main() -> i32 { return 1 }",
        )],
    );
    let e = load_err(&dir, "b.klang");
    assert_eq!(e.code, "E-IO-NOT-FOUND", "{}", e.to_json());
}

#[test]
fn missing_whole_file_is_eio_not_found() {
    // The whole-file form reports the same clean code (not a panic, not
    // a bare string anymore).
    let dir = write_dir(
        "nofile2",
        &[("b.klang", "import \"ghost.klang\"\nfn main() -> i32 { return 1 }")],
    );
    let e = load_err(&dir, "b.klang");
    assert_eq!(e.code, "E-IO-NOT-FOUND", "{}", e.to_json());
}

#[test]
fn missing_name_is_eimport_with_available() {
    let dir = write_dir(
        "noname",
        &[
            ("a.klang", "fn triple(n: i32) -> i32 { return n * 3 }"),
            (
                "b.klang",
                "import { nope } from \"a.klang\"\nfn main() -> i32 { return 1 }",
            ),
        ],
    );
    let e = load_err(&dir, "b.klang");
    assert_eq!(e.code, "E-IMPORT", "{}", e.to_json());
    assert!(e.message.contains("nope"), "{}", e.message);
    assert!(e.message.contains("triple"), "lists available: {}", e.message);
}

#[test]
fn circular_selective_imports_terminate_and_run() {
    // A <-> B selective cycle: terminates (no hang, no stack overflow)
    // and both directions resolve.
    let dir = write_dir(
        "cycle",
        &[
            (
                "a.klang",
                "import { g } from \"b.klang\"\nfn f() -> i32 { return 1 }\nfn main() -> i32 { return f() + g() }",
            ),
            ("b.klang", "import { f } from \"a.klang\"\nfn g() -> i32 { return 10 + f() }"),
        ],
    );
    let (v, _) = check_and_run(&load_ok(&dir, "a.klang"));
    assert_eq!(v, 12, "1 + (10 + 1)");
}

#[test]
fn self_import_terminates() {
    let dir = write_dir(
        "self",
        &[(
            "s.klang",
            "import { h } from \"s.klang\"\nfn h() -> i32 { return 41 }\nfn main() -> i32 { return h() + 1 }",
        )],
    );
    let (v, _) = check_and_run(&load_ok(&dir, "s.klang"));
    assert_eq!(v, 42);
}

#[test]
fn diamond_import_merges_once() {
    // Two files selectively import the same name; the app imports both
    // whole: `triple` merges exactly once (no duplicate error).
    let dir = write_dir(
        "diamond",
        &[
            ("lib.klang", "fn triple(n: i32) -> i32 { return n * 3 }"),
            (
                "b.klang",
                "import { triple } from \"lib.klang\"\nfn b1() -> i32 { return triple(1) }",
            ),
            (
                "c.klang",
                "import { triple } from \"lib.klang\"\nfn c1() -> i32 { return triple(2) }",
            ),
            (
                "app.klang",
                "import \"b.klang\"\nimport \"c.klang\"\nfn main() -> i32 { return b1() + c1() }",
            ),
        ],
    );
    let (v, _) = check_and_run(&load_ok(&dir, "app.klang"));
    assert_eq!(v, 9, "3 + 6");
}

#[test]
fn non_imported_sibling_is_invisible() {
    // BUG-A class: selectivity must actually hide what was not named. A
    // program referencing the un-imported sibling fails check loudly
    // (E-UNDEFINED) instead of silently resolving.
    let dir = write_dir(
        "hidden",
        &[
            ("a.klang", "fn triple(n: i32) -> i32 { return n * 3 }\nfn secret() -> i32 { return 999 }"),
            (
                "b.klang",
                "import { triple } from \"a.klang\"\nfn main() -> i32 { return triple(1) + secret() }",
            ),
        ],
    );
    let prog = load_ok(&dir, "b.klang");
    let err = klang::hir::TypedHIR::check(prog).expect_err("secret must be invisible");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&"E-UNDEFINED"), "{codes:?}");
}

#[test]
fn selective_plus_local_duplicate_is_error() {
    // Importing a name the importer already defines is a loud duplicate,
    // never a silent overwrite of the local definition.
    let dir = write_dir(
        "dup",
        &[
            ("a.klang", "fn triple(n: i32) -> i32 { return n * 3 }"),
            (
                "b.klang",
                "import { triple } from \"a.klang\"\nfn triple(n: i32) -> i32 { return 0 }\nfn main() -> i32 { return triple(14) }",
            ),
        ],
    );
    let e = load_err(&dir, "b.klang");
    assert_eq!(e.code, "E-DUPLICATE", "{}", e.to_json());
}

#[test]
fn same_name_from_different_files_is_duplicate() {
    let dir = write_dir(
        "dup2",
        &[
            ("a.klang", "fn f() -> i32 { return 1 }"),
            ("b.klang", "fn f() -> i32 { return 2 }"),
            (
                "app.klang",
                "import { f } from \"a.klang\"\nimport { f } from \"b.klang\"\nfn main() -> i32 { return f() }",
            ),
        ],
    );
    let e = load_err(&dir, "app.klang");
    assert_eq!(e.code, "E-DUPLICATE", "{}", e.to_json());
}

#[test]
fn transitive_whole_import_followed() {
    // The named function's own file imports a helper whole-file: the
    // helper follows transitively so the import keeps working.
    let dir = write_dir(
        "trans",
        &[
            ("c.klang", "fn helper() -> i32 { return 40 }"),
            ("a.klang", "import \"c.klang\"\nfn f() -> i32 { return helper() + 2 }"),
            (
                "b.klang",
                "import { f } from \"a.klang\"\nfn main() -> i32 { return f() }",
            ),
        ],
    );
    let (v, _) = check_and_run(&load_ok(&dir, "b.klang"));
    assert_eq!(v, 42);
}

#[test]
fn unsafe_selective_path_rejected() {
    let dir = write_dir(
        "unsafe",
        &[(
            "b.klang",
            "import { x } from \"../evil.klang\"\nfn main() -> i32 { return 1 }",
        )],
    );
    let e = load_err(&dir, "b.klang");
    assert_eq!(e.code, "E-IMPORT", "{}", e.to_json());
    assert!(e.message.contains("unsafe import"), "{}", e.message);
}

#[test]
fn module_name_selective_suggests_whole_import() {
    // `mod` blocks are never split: naming one is E-IMPORT pointing at
    // the whole-file form.
    let dir = write_dir(
        "modsel",
        &[
            ("a.klang", "mod m { pub fn f() -> i32 { return 1 } }"),
            (
                "b.klang",
                "import { m } from \"a.klang\"\nfn main() -> i32 { return m::f() }",
            ),
        ],
    );
    let e = load_err(&dir, "b.klang");
    assert_eq!(e.code, "E-IMPORT", "{}", e.to_json());
    assert!(e.message.contains("module"), "{}", e.message);
}

#[test]
fn repair_screen_covers_selective_form() {
    // The model-output unsafe-import screen sees the path after `from`.
    use klang::repair::splice::has_unsafe_import;
    assert!(has_unsafe_import("import { a } from \"../../evil.klang\"").is_some());
    assert!(has_unsafe_import("import { a } from \"/abs.klang\"").is_some());
    assert!(has_unsafe_import("import{a} from \"~/x.klang\"").is_some());
    assert!(has_unsafe_import("import { a, b } from \"lib.klang\"").is_none());
    assert!(has_unsafe_import("import { a } from \"./sub/mod.klang\"").is_none());
}

#[test]
fn selective_import_parse_errors_are_loud() {
    // Empty braces and a missing `from` are E-PARSE, never a silent
    // empty/whole import.
    for src in [
        "import {} from \"a.klang\"\nfn main() -> i32 { return 1 }",
        "import { a } \"a.klang\"\nfn main() -> i32 { return 1 }",
        "import { a } from 42\nfn main() -> i32 { return 1 }",
    ] {
        let mut p = Parser::new(src);
        let err = p.parse_program().expect_err("must fail parse");
        assert_eq!(err.code, "E-PARSE", "{}", err.to_json());
    }
    // Whole-file form still parses exactly as before.
    let prog = Parser::new("import \"a.klang\"\nfn main() -> i32 { return 1 }")
        .parse_program()
        .expect("parses");
    assert_eq!(prog.imports, vec!["a.klang".to_string()]);
    assert!(prog.selective_imports.is_empty());
}

#[test]
fn v2_has_no_imports() {
    // The v2 grammar has no imports at all (documented exclusion).
    let src = "import \"a.klang\"\nfn main() -> i32 {\n  return 0\n}\n";
    assert!(
        klang::parser::v2::parse_v2_program(src).is_err(),
        "v2 must not accept imports"
    );
}

#[test]
fn v2_programs_still_run() {
    // Positive v2 control: the shared toolchain is unaffected.
    let src = "fn main() -> i32 {\n  return 40 + 2\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let _mir = klang::mir::v2_lowering::lower_v2_program(
        &prog.schemas,
        &prog.echo_fns,
        &prog.echo_bodies,
        &prog.flows,
        &prog.functions,
    );
    let (v, _) = klang::runtime::v2::run_v2_program(&prog, "main").expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn jit_runs_selectively_imported_program() {
    // Imports resolve at load time: a merged int-only program JITs
    // transparently (imports never reach the backend).
    let dir = write_dir(
        "jit",
        &[
            ("a.klang", "fn triple(n: i32) -> i32 { return n * 3 }"),
            (
                "b.klang",
                "import { triple } from \"a.klang\"\nfn main() -> i32 { return triple(14) }",
            ),
        ],
    );
    let prog = load_ok(&dir, "b.klang");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
    let mir = klang::mir::lower(&prog);
    let (v, _) = klang::jit::run_jit(&mir, "main", &[]).expect("jits");
    assert_eq!(v, 42);
}

#[test]
fn fmt_round_trips_selective_import() {
    let src = "import { triple, u } from \"a.klang\"\nimport \"b.klang\"\nfn main() -> i32 { return 1 }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let once = klang::fmt::fmt_program(&prog);
    assert!(
        once.contains("import { triple, u } from \"a.klang\""),
        "fmt keeps selective import:\n{once}"
    );
    assert!(once.contains("import \"b.klang\""), "{once}");
    let twice = klang::fmt::fmt_program(&Parser::new(&once).parse_program().expect("re-parses"));
    assert_eq!(once, twice, "fmt idempotent");
}

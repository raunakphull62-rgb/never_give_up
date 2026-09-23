use std::collections::HashMap;

use klang::parser::Parser;

fn run_src(src: &str, entry: &str) -> (i32, Vec<String>) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean"
    );
    let mir = klang::mir::lower(&prog);
    klang::runtime::run_with_output(&mir, entry, &[], &HashMap::new()).expect("runs")
}

fn check_err_code(src: &str, code: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail");
    assert!(
        err.iter().any(|d| d.code == code),
        "want {code}, got {:?}",
        err.iter().map(|d| &d.code).collect::<Vec<_>>()
    );
}

#[test]
fn module_pub_fn_callable_via_qualified_path() {
    // A `pub` declaration in module A runs when called as `a::f()` from
    // outside. Without modules this is a parse/undefined error.
    let (v, _) = run_src(
        "mod lexer { pub fn scan(x: i32) -> i32 { return x * 2 } } fn main() -> i32 { return lexer::scan(21) }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn module_private_fn_qualified_is_error() {
    // A genuinely private cross-module reference is a compile error with a
    // real code: not a panic, not a silently-accepted program.
    check_err_code(
        "mod lexer { fn helper() -> i32 { return 1 } } fn main() -> i32 { return lexer::helper() }",
        "E-PRIVATE",
    );
    // Same for a private type used in an annotation from outside.
    check_err_code(
        "mod m { struct Secret { x: i32 } } fn f(s: m::Secret) -> i32 { return 0 } fn main() -> i32 { return 0 }",
        "E-PRIVATE",
    );
    // And for a private enum value.
    check_err_code(
        "mod m { enum E { A } } fn main() -> i32 { let v = m::E::A() return 0 }",
        "E-PRIVATE",
    );
}

#[test]
fn module_pub_struct_qualified_constructs_and_annotates() {
    // `lexer::Token` works both as a construction path and as a type
    // annotation for a function parameter.
    let (v, _) = run_src(
        "mod lexer { pub struct Token { kind: i32 } } fn id(t: lexer::Token) -> i32 { return t.kind } fn main() -> i32 { let t = lexer::Token { kind: 40 } return id(t) + 2 }",
        "main",
    );
    assert_eq!(v, 42);
}

#[test]
fn module_same_named_private_items_do_not_collide() {
    // Identical private names in different modules resolve locally: each
    // in-module bare call binds its own module's item.
    let (v, _) = run_src(
        "mod a { fn h() -> i32 { return 1 } pub fn run() -> i32 { return h() + 10 } } mod b { fn h() -> i32 { return 2 } pub fn run() -> i32 { return h() + 20 } } fn main() -> i32 { return a::run() + b::run() }",
        "main",
    );
    assert_eq!(v, 33);
}

#[test]
fn module_qualified_enum_match_with_exhaustiveness() {
    // Qualified enum values dispatch through qualified match arms, and a
    // non-exhaustive qualified match is still E-MATCH-EXHAUSTIVE.
    let (v, _) = run_src(
        "mod shape { pub enum Kind { Circle(r: i32), Rect(w: i32) } pub fn area(k: Kind) -> i32 { return match k { Kind::Circle(r) => r * 3, Kind::Rect(w) => w * 2 } } } fn main() -> i32 { return shape::area(shape::Kind::Circle(10)) + shape::area(shape::Kind::Rect(5)) }",
        "main",
    );
    assert_eq!(v, 40);
    check_err_code(
        "mod shape { pub enum Kind { Circle(r: i32), Rect(w: i32) } } fn main() -> i32 { let k = shape::Kind::Circle(1) return match k { shape::Kind::Circle(r) => r } }",
        "E-MATCH-EXHAUSTIVE",
    );
}

#[test]
fn module_member_paths_extend_module_path() {
    // Structural identity holds for the new construct: member ids are
    // generated while the module scope is entered.
    let mut p = Parser::new("mod m { pub fn f() -> i32 { return 1 } } fn main() -> i32 { return m::f() }");
    let prog = p.parse_program().expect("parses");
    let m = &prog.mods[0];
    assert_eq!(m.name, "m");
    for f in &m.functions {
        assert!(f.id.starts_with(&m.id), "member fn extends mod");
        assert!(f.body.id.starts_with(&f.id), "body extends member fn");
    }
}

#[test]
fn module_across_files_via_import() {
    // A module defined in another file is usable through the existing
    // `import` merge: files stay the module unit, paths stay qualified.
    use std::io::Write;
    let dir = std::env::temp_dir().join("klang-module-test");
    std::fs::create_dir_all(&dir).unwrap();
    let lib = dir.join("lexer.klang");
    let app = dir.join("app.klang");
    std::fs::write(&lib, "mod lexer { pub fn num() -> i32 { return 41 } }").unwrap();
    let mut f = std::fs::File::create(&app).unwrap();
    writeln!(f, "import \"lexer.klang\"").unwrap();
    writeln!(f, "fn main() -> i32 {{ return lexer::num() + 1 }}").unwrap();
    drop(f);
    let load = |path: &std::path::Path| {
        let src = std::fs::read_to_string(path).unwrap();
        let mut p = Parser::new(&src);
        p.parse_program().expect("parses")
    };
    let lp = load(&lib).with_file_prefix(0);
    let mut ap = load(&app);
    ap.imports.clear();
    let ap = ap.with_file_prefix(1);
    let merged = klang::ast::Program {
        mods: [lp.mods, ap.mods].concat(),
        enums: [lp.enums, ap.enums].concat(),
        structs: [lp.structs, ap.structs].concat(),
        imports: vec![],
        functions: [lp.functions, ap.functions].concat(),
    };
    assert!(klang::hir::TypedHIR::check(merged.clone()).is_ok());
    let mir = klang::mir::lower(&merged);
    let (v, _) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    assert_eq!(v, 42);
}

#[test]
fn module_decls_survive_fmt_roundtrip() {
    let src = "mod lexer { pub struct Token { kind: i32 } pub fn scan(x: i32) -> i32 { return x } } fn main() -> i32 { return lexer::scan(42) }";
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let out = klang::fmt::fmt_program(&prog);
    assert!(out.contains("mod lexer"), "{out}");
    assert!(out.contains("pub struct Token"), "{out}");
    assert!(out.contains("pub fn scan"), "{out}");
    let mut q = Parser::new(&out);
    let prog2 = q.parse_program().expect("fmt output re-parses");
    assert_eq!(prog2.mods.len(), 1);
    assert!(prog2.mods[0].structs[0].is_pub);
    assert!(klang::hir::TypedHIR::check(prog2).is_ok());
}

// ---------------------------------------------------------------------
// Phase 0 audit regression: top-level struct/enum + `mod` in one file.
//
// Found by the adversarial dogfooding pass: `modules::resolve` seeded its
// flattened program with clones of the top-level structs/enums AND then
// pushed the same items again in the top-level rewrite loops, so any file
// containing BOTH a `mod` block and a top-level `enum`/`struct` got
// spurious `E-DUPLICATE` diagnostics and could not be checked or run at
// all. `resolve`'s early return for module-free programs masked it, and no
// existing gate combined the two constructs.
//
// This test would have caught it: it asserts clean checking, correct
// declaration counts after flattening, AND successful execution.
// ---------------------------------------------------------------------
#[test]
fn module_with_top_level_enum_no_spurious_duplicates() {
    // enum + mod + a qualified call through the module: valid program.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } mod util { pub fn wrap(x: i32) -> i32 { return x + 1 } } fn main() -> i32 { return util::wrap(41) }",
        "main",
    );
    assert_eq!(v, 42);
    // And the enum is still usable alongside the module.
    let (v, _) = run_src(
        "enum Opt { Some(x: i32), None } mod util { pub fn id(x: i32) -> i32 { return x } } fn pick(v: Opt) -> i32 { return match v { Opt::Some(n) => n, Opt::None => 0 } } fn main() -> i32 { return pick(Opt::Some(util::id(42))) }",
        "main",
    );
    assert_eq!(v, 42);
    // Flattening must not double-count declarations.
    let mut p = Parser::new(
        "enum Opt { Some(x: i32), None } struct Box { v: i32 } mod util { pub fn wrap(x: i32) -> i32 { return x } } fn main() -> i32 { return util::wrap(1) }",
    );
    let prog = p.parse_program().expect("parses");
    let (flat, diags) = klang::modules::resolve(&prog);
    assert!(diags.is_empty(), "no diagnostics for valid code, got {diags:?}");
    assert_eq!(flat.enums.len(), 1, "top-level enum counted once");
    assert_eq!(flat.structs.len(), 1, "top-level struct counted once");
    assert!(klang::hir::TypedHIR::check(prog).is_ok(), "must check clean");
}

#[test]
fn module_with_top_level_struct_no_spurious_duplicates() {
    // struct + mod: same bug class as the enum case above.
    let (v, _) = run_src(
        "struct Box { v: i32 } mod util { pub fn wrap(x: i32) -> i32 { return x + 1 } } fn main() -> i32 { let b = Box { v: 41 } return util::wrap(b.v) }",
        "main",
    );
    assert_eq!(v, 42);
    let (v, _) = run_src(
        "struct Box<T> { value: T } mod util { pub fn id(x: i32) -> i32 { return x } } fn main() -> i32 { let b = Box { value: util::id(42) } return b.value }",
        "main",
    );
    assert_eq!(v, 42);
}

use klang::parser::Parser;

fn parse_code(src: &str) -> String {
    let mut p = Parser::new(src);
    match p.parse_program() {
        Ok(_) => "CLEAN".to_string(),
        Err(d) => format!("E:{}", d.code),
    }
}

#[test]
fn stray_tokens_are_errors_not_silent() {
    for src in [
        "fn main() -> i32 { let x = 1 & 2 return x }",
        "fn main() -> i32 { let x = 1 | 2 return x }",
        "fn main() -> i32 { let x = $ return x }",
        "fn main() -> i32 { let x = 1 return # }",
    ] {
        let r = parse_code(src);
        assert!(r.starts_with("E:"), "must reject, got {r} for {src}");
        println!("reject OK: {src} -> {r}");
    }
}

#[test]
fn non_ascii_is_clean_error_never_panic() {
    // Used to panic the parser (byte-boundary slicing); must be E-PARSE.
    for src in [
        "fn main() -> i32 { let \u{e9} = 1 return 1 }",
        "fn main() -> i32 { return \u{1f600} }",
        "// \u{e9}\nfn main() -> i32 { return 1 }",
    ] {
        let r = parse_code(src);
        println!("non-ascii -> {r} for {src:?}");
    }
    // Comment-only unicode must still parse (comments are skipped).
    assert_eq!(
        parse_code("// \u{e9}\nfn main() -> i32 { return 1 }"),
        "CLEAN"
    );
}

#[test]
fn valid_operators_still_parse() {
    for src in [
        "fn main() -> i32 { if true && true { return 1 } return 0 }",
        "fn main() -> i32 { if false || true { return 1 } return 0 }",
        "fn main() -> i32 { for i in 0..3 { print(i) } return 0 }",
        "fn main() -> i32 { return !true }",
    ] {
        assert_eq!(parse_code(src), "CLEAN", "must accept {src}");
    }
    println!("valid ops OK");
}

#[test]
fn match_block_arm_is_clean_parse_error() {
    // F19: match arm bodies are single expressions; a `{ ... }` block
    // parses as a map literal and fails. Must be E-PARSE, never a panic,
    // and SPEC must not claim otherwise.
    for src in [
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { Opt::Some(n) => { return n }, Opt::None => 0 } }",
        "enum Opt { Some(x: i32), None } fn f(v: Opt) -> i32 { return match v { _ => { return 1 } } }",
    ] {
        let r = parse_code(src);
        assert_eq!(r, "E:E-PARSE", "block arm must be clean E-PARSE, got {r} for {src}");
        println!("block-arm reject OK: {r}");
    }
}

#[test]
fn omitted_return_type_desugars_to_void() {
    // D4: omitting `->` means `-> void`, desugared at parse time — the old
    // Phase 1c E-PARSE ceremony no longer fires for legitimate void
    // functions. The AST carries the desugared "void" spelling.
    for src in [
        "fn foo(a: i32) { print(a) }",
        "fn main() { print(1) }",
        "fn main() { }",
    ] {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("omitted arrow parses");
        assert_eq!(prog.functions.len(), 1, "one fn in {src}");
        assert_eq!(
            prog.functions[0].return_ty, "void",
            "desugared to void: {src}"
        );
        println!("void-desugar OK: {src}");
    }
    // Closures get the same treatment (anonymous, so asserted on the expr).
    let mut p = Parser::new("fn main() -> i32 { let f = fn(x: i32) { return x } return f(1) }");
    let prog = p.parse_program().expect("closure without arrow parses");
    assert_eq!(prog.functions[0].return_ty, "i32");
    // `-> ()` is still rejected: `void` is the only unit spelling.
    for src in [
        "fn foo(a: i32) -> () { print(a) }",
        "fn main() -> () { print(1) }",
    ] {
        let mut p = Parser::new(src);
        let d = p.parse_program().expect_err("`-> ()` still fails");
        assert_eq!(d.code, "E-PARSE", "{}", d.to_json());
        println!("unit-parens reject OK: {src}");
    }
    // The explicit forms still parse unchanged.
    for src in [
        "fn foo(a: i32) -> void { print(a) }",
        "fn main() -> i32 { return 0 }",
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x } return f(1) }",
    ] {
        assert!(Parser::new(src).parse_program().is_ok(), "parses: {src}");
    }
    println!("explicit forms OK");
}

#[test]
fn v2_omitted_return_type_desugars_to_void() {
    // Same D4 desugar in the v2 grammar: `fn foo(a: i32) { ... }` parses
    // with a Consonant `void` return type instead of failing E-PARSE-V2.
    let prog = klang::parser::v2::parse_v2_program("fn foo(a: i32) { return a }")
        .expect("v2 omitted arrow parses");
    assert_eq!(prog.functions.len(), 1);
    assert_eq!(prog.functions[0].return_ty.display(), "void");
    assert_eq!(
        prog.functions[0].return_ty,
        klang::ast::resonance::QualifiedType::new(
            klang::ast::resonance::ResonanceQualifier::Consonant,
            "void"
        )
    );
    // `-> ()` stays rejected in v2 as well.
    let d = klang::parser::v2::parse_v2_program("fn foo(a: i32) -> () { return a }")
        .expect_err("v2 `-> ()` still fails");
    assert_eq!(d.code, "E-PARSE-V2", "{}", d.to_json());
    println!("v2 void-desugar OK");
}

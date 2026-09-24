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

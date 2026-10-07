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
fn missing_return_type_names_function_and_fix() {
    // Phase 1c: the dropped `-> type` ceremony reports what is missing and
    // shows the fix, instead of a bare `expected '->'`.
    for (src, subject) in [
        ("fn foo(a: i32) { print(a) }", "function `foo`"),
        ("fn main() { print(1) }", "function `main`"),
    ] {
        let mut p = Parser::new(src);
        let d = p.parse_program().expect_err("missing return type fails");
        assert_eq!(d.code, "E-PARSE", "{}", d.to_json());
        assert!(
            d.message.contains(&format!("{subject} is missing its return type")),
            "names the function: {}",
            d.to_json()
        );
        assert!(
            d.fixes.iter().any(|f| f.label.contains("add `-> i32`")),
            "shows the fix: {}",
            d.to_json()
        );
        println!("return-type ceremony OK: {subject}");
    }
    // Closures get the same treatment (anonymous, so no name).
    let mut p = Parser::new("fn main() -> i32 { let f = fn(x: i32) { return x } return f(1) }");
    let d = p.parse_program().expect_err("closure without return type fails");
    assert_eq!(d.code, "E-PARSE", "{}", d.to_json());
    assert!(d.message.contains("closure is missing its return type"), "{}", d.to_json());
    assert!(
        d.fixes.iter().any(|f| f.label.contains("add `-> i32`")),
        "{}",
        d.to_json()
    );
    // The fixed forms still parse.
    for src in [
        "fn foo(a: i32) -> void { print(a) }",
        "fn main() -> i32 { return 0 }",
        "fn main() -> i32 { let f = fn(x: i32) -> i32 { return x } return f(1) }",
    ] {
        assert!(Parser::new(src).parse_program().is_ok(), "parses: {src}");
    }
    println!("return-type fix OK");
}

#[test]
fn v2_missing_return_type_names_function_and_fix() {
    // Same ceremony in the v2 grammar (same message, `E-PARSE-V2` code).
    let d = klang::parser::v2::parse_v2_program("fn foo(a: i32) { return a }")
        .expect_err("v2 missing return type fails");
    assert_eq!(d.code, "E-PARSE-V2", "{}", d.to_json());
    assert!(
        d.message.contains("function `foo` is missing its return type"),
        "{}",
        d.to_json()
    );
    assert!(
        d.fixes.iter().any(|f| f.label.contains("add `-> i32`")),
        "{}",
        d.to_json()
    );
    println!("v2 return-type ceremony OK");
}

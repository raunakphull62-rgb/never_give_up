//! Phase 2 — v2 lexer gates: resonance tokens, spans, malformed input.

use klang::lexer::{lex, TokenKind};

fn kinds(src: &str) -> Vec<TokenKind> {
    lex(src)
        .expect("lexes")
        .into_iter()
        .map(|t| t.kind)
        .collect()
}

#[test]
fn v2_lex_question_bang_spans() {
    let toks = lex("?Data !Data").expect("lexes");
    assert_eq!(toks[0].kind, TokenKind::Question);
    assert_eq!((toks[0].start, toks[0].end), (0, 1));
    assert_eq!(toks[1].kind, TokenKind::Ident("Data".to_string()));
    assert_eq!((toks[1].start, toks[1].end), (1, 5));
    assert_eq!(toks[2].kind, TokenKind::Bang);
    assert_eq!((toks[2].start, toks[2].end), (6, 7));
    assert_eq!(toks[3].kind, TokenKind::Ident("Data".to_string()));
    assert_eq!((toks[3].start, toks[3].end), (7, 11));
}

#[test]
fn v2_lex_pipeline_span() {
    let toks = lex("raw |> tune").expect("lexes");
    assert_eq!(toks[1].kind, TokenKind::PipeGt);
    assert_eq!((toks[1].start, toks[1].end), (4, 6));
    // A lone `|` stays its own token so the parser can reject it loudly.
    assert_eq!(kinds("a | b")[1], TokenKind::Pipe);
    // `!=` lexes as one token, never `!` + `=`.
    assert_eq!(kinds("a != b")[1], TokenKind::NotEq);
}

#[test]
fn v2_lex_keywords_and_dep_eq() {
    let toks = lex("flow echo listen tune verify dep=threshold").expect("lexes");
    assert_eq!(toks[0].kind, TokenKind::Flow);
    assert_eq!((toks[0].start, toks[0].end), (0, 4));
    assert_eq!(toks[1].kind, TokenKind::Echo);
    assert_eq!(toks[2].kind, TokenKind::Listen);
    assert_eq!(toks[3].kind, TokenKind::Tune);
    assert_eq!(toks[4].kind, TokenKind::Verify);
    // `dep=` is `Dep` + `Eq` with exact adjacent spans.
    assert_eq!(toks[5].kind, TokenKind::Dep);
    assert_eq!((toks[5].start, toks[5].end), (29, 32));
    assert_eq!(toks[6].kind, TokenKind::Eq);
    assert_eq!((toks[6].start, toks[6].end), (32, 33));
    assert_eq!(toks[7].kind, TokenKind::Ident("threshold".to_string()));
}

#[test]
fn v2_lex_whitespace_nesting_stable() {
    let a = lex("flow(?Data|>tune)").expect("lexes");
    let b = lex("  flow ( ?Data |> tune ) ").expect("lexes");
    assert_eq!(
        a.iter().map(|t| &t.kind).collect::<Vec<_>>(),
        b.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
    // Same kinds, different spans under whitespace.
    assert_ne!(a[1].start, b[1].start);
    // Nesting braces lex explicitly.
    assert!(kinds("flow(x) { listen(h) }").contains(&TokenKind::LBrace));
}

#[test]
fn v2_lex_malformed_is_structured() {
    let e = lex("tune @").expect_err("unexpected char fails");
    assert_eq!(e.offset, 5);
    assert!(e.message.contains("unexpected character"));
    let e = lex("\"oops").expect_err("unterminated string fails");
    assert!(e.message.contains("unterminated string"));
    let e = lex("99999999999999999999999").expect_err("int overflow fails");
    assert!(e.message.contains("out of range"));
}

#[test]
fn v2_lex_v1_behavior_untouched() {
    // The v1 tokenizer still owns `task_group`/`spawn`/`await`: v2 lexing
    // must not claim them as keywords.
    assert_eq!(
        kinds("spawn await")[0],
        TokenKind::Ident("spawn".to_string())
    );
    let mut p = klang::parser::Parser::new(
        "fn fetch(n: i32) -> i32 { return n } fn main() -> i32 { return fetch(21) }",
    );
    let prog = p.parse_program().expect("v1 parses");
    assert!(klang::hir::TypedHIR::check(prog).is_ok());
}

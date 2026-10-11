//! Shared v2 check front end (`klang check-v2` / `check --lang v2`).
//!
//! Leading-construct dispatch: `flow...` parses as a Flow declaration, `echo...` as an Echo
//! declaration, `listen...` as a listen expression; anything else gets the
//! v2 lexer check plus full-program parse and check-time `tune`/`verify`
//! validation (constants rejected here with the same codes the runtime
//! emits, dynamic values left to the runtime).

use crate::diagnostics::Diagnostic;

/// Parse + check one v2 source, returning the diagnostics (empty = clean).
pub fn v2_check_source(source: &str) -> Vec<Diagnostic> {
    v2_check_source_with_file(source, "input.v2")
}

/// Whole-program v2 check with a real filename for diagnostics.
/// Snippet paths (flow/echo/listen) behave exactly as before; full
/// programs additionally parse and get check-time `tune`/`verify`
/// validation — constants rejected here with the same codes the runtime
/// emits, dynamic values left to the runtime.
pub fn v2_check_source_with_file(source: &str, file: &str) -> Vec<Diagnostic> {
    let trimmed = source.trim_start();
    if trimmed.starts_with("flow") {
        return match crate::parser::flow::parse_flow_decl(source, None) {
            Ok(_) => vec![],
            Err(d) => vec![d],
        };
    }
    if trimmed.starts_with("echo") {
        return match crate::parser::echo::parse_echo_decl(source) {
            Ok(_) => vec![],
            Err(d) => vec![d],
        };
    }
    if trimmed.starts_with("listen") {
        return match crate::parser::echo::parse_listen(source) {
            Ok(_) => vec![],
            Err(d) => vec![d],
        };
    }
    match crate::lexer::lex(source) {
        Ok(_) => {}
        Err(e) => {
            return vec![Diagnostic::parse_error(
                file,
                e.offset,
                e.offset,
                &e.message,
            )]
        }
    }
    // Full-program path: parse, then check tune/verify constants.
    let prog = match crate::parser::v2::parse_v2_program(source) {
        Ok(prog) => prog,
        Err(d) => return vec![d],
    };
    crate::sema::v2_tune::check_program_tunes(&prog, file)
}

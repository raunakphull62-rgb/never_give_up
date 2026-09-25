//! Klang v2 lexer boundary (Phase 2).
//!
//! v1 tokenization lives in [`crate::parser`]. This module owns v2
//! resonance tokens (`?`, `!`, `|>`, `flow`, `echo`, `listen`, `tune`,
//! `verify`, `dep=`); see [`resonance::lex`].

/// v2 resonance tokens and spans.
pub mod resonance;

pub use resonance::{lex, Token, TokenKind};

/// v2 lexer error placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    /// Byte offset where lexing failed.
    pub offset: usize,
    /// Machine-readable reason.
    pub message: String,
}

/// v2 lexer result placeholder.
pub type Result<T> = std::result::Result<T, LexError>;

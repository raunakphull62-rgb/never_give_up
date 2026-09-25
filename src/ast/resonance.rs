//! Klang v2 resonance qualifiers (Phase 3).
//!
//! The qualifier is semantic data kept separate from the underlying
//! nominal type: `?Data`, `Data`, and `!Data` are three distinct types.

/// Resonance state of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResonanceQualifier {
    /// `?T` — Dissonant: shape T is possible but not proven safe.
    Dissonant,
    /// `T` — Consonant: ordinary compile-time-safe value.
    Consonant,
    /// `!T` — Harmonic: schema-validated value carrying runtime proof.
    Harmonic,
}

impl ResonanceQualifier {
    /// Prefix used in surface syntax (`?`, ``, `!`).
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Dissonant => "?",
            Self::Consonant => "",
            Self::Harmonic => "!",
        }
    }
}

/// A qualified type: resonance marker plus the underlying nominal name.
///
/// The qualifier is stored apart from `base` so later passes (sema, MIR,
/// diagnostics) can reason about proof state without string-sniffing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedType {
    /// Resonance state.
    pub qualifier: ResonanceQualifier,
    /// Nominal base type name (e.g. `Data`, `i32`).
    pub base: String,
}

impl QualifiedType {
    /// Build a qualified type from its parts.
    pub fn new(qualifier: ResonanceQualifier, base: &str) -> Self {
        Self {
            qualifier,
            base: base.to_string(),
        }
    }

    /// Parse surface syntax: `?Data`, `Data`, `!Data`.
    ///
    /// A single leading `?`/`!` sets the qualifier; anything else is
    /// Consonant. Empty input yields base `""` (callers reject it in the
    /// parser with a span; this constructor never panics).
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix('?') {
            Self::new(ResonanceQualifier::Dissonant, rest.trim())
        } else if let Some(rest) = text.strip_prefix('!') {
            Self::new(ResonanceQualifier::Harmonic, rest.trim())
        } else {
            Self::new(ResonanceQualifier::Consonant, text)
        }
    }

    /// Render back to surface syntax (`?Data`, `Data`, `!Data`).
    pub fn display(&self) -> String {
        format!("{}{}", self.qualifier.prefix(), self.base)
    }

    /// True for `?T`.
    pub fn is_dissonant(&self) -> bool {
        self.qualifier == ResonanceQualifier::Dissonant
    }

    /// True for `!T`.
    pub fn is_harmonic(&self) -> bool {
        self.qualifier == ResonanceQualifier::Harmonic
    }
}

//! Klang v2 standard library (Phase 6).
//!
//! `verify`/`tune` entry points live in [`verify`]; `net::echo_get` arrives
//! in Phase 10. Only [`verify`]/[`tune`](verify::tune) mint Harmonic proofs.

/// `verify`/`tune` runtime entry points.
pub mod verify;

/// Network boundary: `net::echo_get` behind a mockable transport.
pub mod net;

/// v2 stdlib error placeholder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdlibError {
    /// Machine-readable reason.
    pub message: String,
}

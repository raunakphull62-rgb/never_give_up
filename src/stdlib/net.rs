//! Klang v2 network boundary (Phase 10).
//!
//! `net::echo_get(url)` returns an `Echo<?Data>` without touching the
//! network: all I/O goes through the [`Transport`] trait, and unit tests
//! use [`MockTransport`]. A real transport (not in this milestone) would
//! implement [`Transport`] out of tree; the compiler itself never calls
//! one.

use crate::ai_safety::schema::DataValue;
use crate::runtime::echo::Echo;
use std::collections::HashMap;

/// Byte source behind `echo_get`. Object-safe so harnesses can inject fakes.
pub trait Transport {
    /// Fetch `url` as text, or a textual cause on failure.
    fn get(&self, url: &str) -> Result<String, String>;
}

/// In-memory fake transport: no sockets, no threads, deterministic.
#[derive(Debug, Default)]
pub struct MockTransport {
    responses: HashMap<String, Result<String, String>>,
}

impl MockTransport {
    /// Empty mock (every URL is unregistered until `when`/`when_fail`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a successful body for `url`.
    pub fn when(&mut self, url: &str, body: &str) {
        self.responses.insert(url.to_string(), Ok(body.to_string()));
    }

    /// Register a failure cause for `url`.
    pub fn when_fail(&mut self, url: &str, cause: &str) {
        self.responses
            .insert(url.to_string(), Err(cause.to_string()));
    }
}

impl Transport for MockTransport {
    fn get(&self, url: &str) -> Result<String, String> {
        self.responses
            .get(url)
            .cloned()
            .unwrap_or_else(|| Err(format!("no mock response for `{url}`")))
    }
}

/// A pending `net::echo_get(url)`: an unsettled `Echo<?Data>` plus the URL
/// it will be fulfilled from.
#[derive(Debug)]
pub struct PendingEcho {
    /// Requested URL.
    pub url: String,
    /// Dissonant payload type (always `?Data` at this boundary).
    pub inner_ty: String,
    echo: Echo<DataValue>,
}

impl PendingEcho {
    /// Handle name of the underlying Echo.
    pub fn handle(&self) -> &str {
        self.echo.handle()
    }

    /// Strong count of the underlying cell (1 while outstanding).
    pub fn strong(&self) -> usize {
        self.echo.strong()
    }

    /// Fulfill through `transport`: success stores the body as `Str`,
    /// failure stores the cause. Returns the still-unlistened Echo.
    pub fn fulfill(self, transport: &dyn Transport) -> Echo<DataValue> {
        match transport.get(&self.url) {
            Ok(body) => self.echo.complete(DataValue::Str(body)),
            Err(cause) => self.echo.fail(&cause),
        }
        self.echo
    }
}

/// `net::echo_get(url)`: start a background fetch as `Echo<?Data>`.
///
/// Never performs I/O itself; pair with [`PendingEcho::fulfill`] (tests)
/// or a real [`Transport`] (harness side).
pub fn echo_get(url: &str) -> PendingEcho {
    PendingEcho {
        url: url.to_string(),
        inner_ty: "?Data".to_string(),
        echo: Echo::new(&format!("echo_get({url})")),
    }
}

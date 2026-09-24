//! Model backends: the `ModelBackend` trait + mock.
//!
//! Phase 4 correction: Klang no longer dials a model directly. The
//! harness the user already runs (Cline, OpenCode, Claude Code, ...)
//! owns the model connection, provider config, and API keys; Klang
//! exposes verification (check / diagnostics / scope planning) for the
//! harness to call, via `klang mcp` or the library API. What remains
//! here is the mechanism side of the repair loop: one model call as an
//! abstract `complete(system, user) -> source text`, backed in-tree
//! only by the deterministic `MockBackend` used by tests and `--dry-run`
//! shape checks. There is deliberately no HTTP client, no endpoint
//! configuration, and no key handling anywhere in this crate.

/// Backend failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    MissingConfig(String),
    Http(String),
    Timeout,
    Parse(String),
    Api(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingConfig(s) => write!(f, "missing config: {s}"),
            Self::Http(s) => write!(f, "http error: {s}"),
            Self::Timeout => write!(f, "request timed out"),
            Self::Parse(s) => write!(f, "parse error: {s}"),
            Self::Api(s) => write!(f, "api error: {s}"),
        }
    }
}

/// One model call: system + user prompt -> raw Klang source text.
///
/// In production this is implemented by the harness driving the loop
/// (it already has a model connection); in-tree, by `MockBackend`.
/// The driver (`run_repair`) treats the response as untrusted text:
/// it is spliced, re-parsed, and fully re-checked, never executed.
pub trait ModelBackend {
    fn complete(&self, system: &str, user: &str) -> Result<String, ModelError>;
    /// How many calls have been made (mocks assert on this).
    fn calls(&self) -> usize {
        0
    }
}

/// Deterministic mock backend for tests and `--dry-run` shape checks.
/// Returns queued responses in order; repeats the last when exhausted.
pub struct MockBackend {
    pub responses: std::sync::Mutex<Vec<String>>,
    pub calls: std::sync::Mutex<usize>,
    pub fail_with: Option<String>,
}

impl MockBackend {
    pub fn new(responses: Vec<String>) -> Self {
        Self { responses: std::sync::Mutex::new(responses), calls: std::sync::Mutex::new(0), fail_with: None }
    }
    pub fn failing(msg: &str) -> Self {
        Self { responses: std::sync::Mutex::new(vec![]), calls: std::sync::Mutex::new(0), fail_with: Some(msg.to_string()) }
    }
}

impl ModelBackend for MockBackend {
    fn complete(&self, _system: &str, _user: &str) -> Result<String, ModelError> {
        *self.calls.lock().unwrap() += 1;
        if let Some(m) = &self.fail_with {
            return Err(ModelError::Http(m.clone()));
        }
        let mut q = self.responses.lock().unwrap();
        if q.is_empty() {
            return Err(ModelError::Parse("mock: no more queued responses".into()));
        }
        if q.len() > 1 {
            Ok(q.remove(0))
        } else {
            Ok(q[0].clone())
        }
    }
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

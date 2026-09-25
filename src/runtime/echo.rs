//! Klang v2 Echo runtime (Phase 8).
//!
//! Deterministic, single-threaded result storage: an Echo is unsettled
//! until [`Echo::complete`]/[`Echo::fail`] stores its outcome, and
//! [`Echo::listen`] consumes the handle exactly once. Failures are
//! lossless — the cause travels with the handle to the listener.
//! Ownership rides on an [`RcCell`]: strong count 1 while outstanding, 0
//! after `listen`/`join`. Dropping an Echo without listening leaks the
//! cell visibly (count stays 1 and [`live_count`](super::gc::rc::live_count)
//! stays elevated); the static checker ([`crate::sema::echo_lifetime`])
//! rejects that shape, this module makes it observable.

use super::gc::rc::RcCell;
use std::sync::{Arc, Mutex};

/// Lossless Echo failure delivered to the listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EchoError {
    /// Handle that failed.
    pub handle: String,
    /// Failure cause (never empty, never dropped).
    pub cause: String,
}

/// A live Echo handle with result storage.
#[derive(Debug)]
pub struct Echo<T> {
    handle: String,
    ownership: RcCell<()>,
    result: Arc<Mutex<Option<Result<T, String>>>>,
}

impl<T> Echo<T> {
    /// Create an unsettled Echo (one strong reference).
    pub fn new(handle: &str) -> Self {
        Self {
            handle: handle.to_string(),
            ownership: RcCell::new(()),
            result: Arc::new(Mutex::new(None)),
        }
    }

    /// Handle name.
    pub fn handle(&self) -> &str {
        &self.handle
    }

    /// Strong count of the ownership cell (1 while outstanding).
    pub fn strong(&self) -> usize {
        self.ownership.strong()
    }

    /// Settle with success. Later `listen` delivers the value.
    pub fn complete(&self, value: T) {
        *self.result.lock().expect("echo mutex") = Some(Ok(value));
    }

    /// Settle with failure. Later `listen` delivers [`EchoError`] with the
    /// cause intact.
    pub fn fail(&self, cause: &str) {
        *self.result.lock().expect("echo mutex") = Some(Err(cause.to_string()));
    }

    /// Consume the handle and retrieve the result.
    ///
    /// Unsettled Echoes report `not settled`; failures report the stored
    /// cause. Either way the cell releases exactly once, so a consumed
    /// Echo always leaves strong count 0.
    pub fn listen(self, _file: &str) -> Result<T, EchoError> {
        let out = self.result.lock().expect("echo mutex").take();
        self.ownership.release();
        match out {
            Some(Ok(v)) => Ok(v),
            Some(Err(cause)) => Err(EchoError {
                handle: self.handle.clone(),
                cause,
            }),
            None => Err(EchoError {
                handle: self.handle.clone(),
                cause: "echo was never settled".to_string(),
            }),
        }
    }

    /// Join is `listen` through a structured scope (same lossless path).
    pub fn join(self, file: &str) -> Result<T, EchoError> {
        self.listen(file)
    }
}

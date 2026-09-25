//! Klang v2 Echo lifetime tracking (Phase 8).
//!
//! Every Echo handle must be visibly `listen`ed, `transfer`red, or `join`ed
//! before its scope exits — on every branch and every early return. Handles
//! left `Created` at a scope exit are `E-ECHO-UNLISTENED` with a fix; a
//! `listen` of an unknown or already-consumed handle is
//! `E-ECHO-INVALID-LISTEN`. Must-analysis matches the v1 task checker:
//! only listens on **all** paths satisfy an outstanding Echo.

use crate::diagnostics::Diagnostic;
use std::collections::HashMap;

/// Lifetime state of one Echo handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoState {
    /// Created by calling an `echo fn`; not yet owned elsewhere.
    Created,
    /// Consumed by `listen`.
    Listened,
    /// Moved to an owner that will listen.
    Transferred,
    /// Joined by a structured scope.
    Joined,
}

/// Outstanding Echo handles in one lexical scope.
#[derive(Debug, Clone, Default)]
pub struct EchoScope {
    /// handle → (state, creation span).
    handles: HashMap<String, (EchoState, (usize, usize))>,
}

impl EchoScope {
    /// Empty scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `handle` as freshly created.
    pub fn create(&mut self, handle: &str, span: (usize, usize)) {
        self.handles
            .insert(handle.to_string(), (EchoState::Created, span));
    }

    /// Mark `handle` listened. Unknown or already-consumed handles are
    /// `E-ECHO-INVALID-LISTEN`, never silent no-ops.
    pub fn listen(
        &mut self,
        handle: &str,
        file: &str,
        start: usize,
        end: usize,
    ) -> Result<(), Diagnostic> {
        match self.handles.get_mut(handle) {
            Some((state, _)) if *state == EchoState::Created => {
                *state = EchoState::Listened;
                Ok(())
            }
            Some(_) => Err(crate::ai_safety::diagnostics::echo_invalid_listen(
                file,
                start,
                end,
                handle,
                "echo handle was already consumed",
            )),
            None => Err(crate::ai_safety::diagnostics::echo_invalid_listen(
                file,
                start,
                end,
                handle,
                "no echo handle with this name is outstanding",
            )),
        }
    }

    /// Move `handle` to an owner that will listen.
    pub fn transfer(
        &mut self,
        handle: &str,
        file: &str,
        start: usize,
        end: usize,
    ) -> Result<(), Diagnostic> {
        match self.handles.get_mut(handle) {
            Some((state, _)) if *state == EchoState::Created => {
                *state = EchoState::Transferred;
                Ok(())
            }
            Some(_) => Err(crate::ai_safety::diagnostics::echo_invalid_listen(
                file,
                start,
                end,
                handle,
                "echo handle was already consumed",
            )),
            None => Err(crate::ai_safety::diagnostics::echo_invalid_listen(
                file,
                start,
                end,
                handle,
                "no echo handle with this name is outstanding",
            )),
        }
    }

    /// State of one handle, if tracked.
    pub fn state(&self, handle: &str) -> Option<EchoState> {
        self.handles.get(handle).map(|(s, _)| *s)
    }

    /// Moving a `Created` handle out of its lexical scope without
    /// [`transfer`](Self::transfer) is `E-ECHO-OUTLIVES-SCOPE`: the handle
    /// would escape the owner that must settle it.
    pub fn move_out(
        &self,
        handle: &str,
        file: &str,
        start: usize,
        end: usize,
    ) -> Result<(), Diagnostic> {
        match self.handles.get(handle).map(|(s, _)| *s) {
            Some(EchoState::Created) => Err(crate::ai_safety::diagnostics::echo_outlives_scope(
                file, start, end, handle,
            )),
            _ => Ok(()),
        }
    }

    /// Check a scope exit (including early `return`): every handle must be
    /// listened, transferred, or joined. The first outstanding handle is
    /// `E-ECHO-UNLISTENED` with its creation span.
    pub fn check_end(&self, file: &str) -> Result<(), Diagnostic> {
        // Sorted names keep diagnostics deterministic.
        let mut names: Vec<&String> = self.handles.keys().collect();
        names.sort();
        for name in names {
            let (state, span) = &self.handles[name];
            if *state == EchoState::Created {
                return Err(crate::ai_safety::diagnostics::echo_unlistened(
                    file, span.0, span.1, name,
                ));
            }
        }
        Ok(())
    }
}

/// Check an `if/else` join: handles outstanding before the branch must be
/// listened on **both** sides; handles created inside one side must be
/// settled on that same side (checked here so callers get one call).
pub fn check_branches(
    before: &EchoScope,
    then_scope: &EchoScope,
    else_scope: Option<&EchoScope>,
    file: &str,
) -> Result<(), Diagnostic> {
    // Handles created inside `then` must settle inside `then`.
    then_scope.check_branch_inner(file, &before.handles)?;
    if let Some(else_s) = else_scope {
        else_s.check_branch_inner(file, &before.handles)?;
        // Handles from `before` need listens on both sides.
        let mut names: Vec<&String> = before.handles.keys().collect();
        names.sort();
        for name in names {
            let (pre, _) = &before.handles[name];
            if *pre != EchoState::Created {
                continue;
            }
            let then_ok = then_scope.settled(name);
            let else_ok = else_s.settled(name);
            if !(then_ok && else_ok) {
                let (_, span) = &before.handles[name];
                return Err(crate::ai_safety::diagnostics::echo_unlistened(
                    file, span.0, span.1, name,
                ));
            }
        }
        Ok(())
    } else {
        // No else: the then-side may not run, so nothing outstanding from
        // `before` is satisfied here; it stays outstanding for the outer
        // scope-end check. But a then-side that listens is still fine.
        Ok(())
    }
}

impl EchoScope {
    fn settled(&self, handle: &str) -> bool {
        matches!(
            self.handles.get(handle).map(|(s, _)| s),
            Some(EchoState::Listened | EchoState::Transferred | EchoState::Joined)
        )
    }

    fn check_branch_inner(
        &self,
        file: &str,
        outer: &HashMap<String, (EchoState, (usize, usize))>,
    ) -> Result<(), Diagnostic> {
        let mut names: Vec<&String> = self.handles.keys().collect();
        names.sort();
        for name in names {
            if outer.contains_key(name) {
                continue;
            }
            let (state, span) = &self.handles[name];
            if *state == EchoState::Created {
                return Err(crate::ai_safety::diagnostics::echo_unlistened(
                    file, span.0, span.1, name,
                ));
            }
        }
        Ok(())
    }
}

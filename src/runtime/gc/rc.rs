//! Klang v2 reference-counted heap cells (Phase 8).
//!
//! Explicit [`RcCell::retain`]/[`RcCell::release`] with deterministic value
//! release: the value drops exactly when the last strong reference
//! releases, never later. [`live_count`] exposes live cells so tests pin
//! success- and failure-path cleanup.
//!
//! Cycle policy (no silent leaks): cells form **no collector**. A retain
//! cycle (`A` retains `B` while `B` retains `A`) never reaches zero and
//! stays visible in [`live_count`]. Break cycles with [`WeakCell`] (which
//! never keeps a value alive) or by ordering `release` calls explicitly.

use std::sync::Arc;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex, Weak,
};

/// Live cell count (test/cycle-leak detector).
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// How many [`RcCell`] values are currently alive.
pub fn live_count() -> usize {
    LIVE.load(Ordering::SeqCst)
}

#[derive(Debug)]
struct Inner<T> {
    strong: usize,
    value: Option<T>,
}

/// A reference-counted heap cell.
#[derive(Debug)]
pub struct RcCell<T> {
    inner: Arc<Mutex<Inner<T>>>,
}

impl<T> RcCell<T> {
    /// Allocate one cell with a single strong reference.
    pub fn new(value: T) -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                strong: 1,
                value: Some(value),
            })),
        }
    }

    /// Add one strong reference (shares the same cell).
    ///
    /// Panics on a freed cell: retaining dead storage is a loud bug,
    /// never a silent resurrection.
    pub fn retain(&self) -> Self {
        let mut g = self.inner.lock().expect("rc mutex");
        assert!(g.value.is_some(), "retain on a freed cell");
        g.strong += 1;
        drop(g);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Release one strong reference. The value drops deterministically
    /// when the count reaches zero; further releases panic loudly.
    pub fn release(&self) {
        let mut g = self.inner.lock().expect("rc mutex");
        assert!(g.strong > 0, "release on a freed cell");
        g.strong -= 1;
        if g.strong == 0 {
            g.value = None;
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Current strong count (0 = freed).
    pub fn strong(&self) -> usize {
        self.inner.lock().expect("rc mutex").strong
    }

    /// True once the value has been released.
    pub fn is_freed(&self) -> bool {
        self.inner.lock().expect("rc mutex").value.is_none()
    }

    /// Non-owning weak reference (never keeps the value alive).
    pub fn downgrade(&self) -> WeakCell<T> {
        WeakCell {
            weak: Arc::downgrade(&self.inner),
        }
    }
}

/// A non-owning observer of an [`RcCell`].
#[derive(Debug)]
pub struct WeakCell<T> {
    weak: Weak<Mutex<Inner<T>>>,
}

impl<T> WeakCell<T> {
    /// Observe the cell if its value is still alive, else `None`.
    ///
    /// A live `WeakCell` after the value freed is the visible shape of a
    /// would-be cycle leak: the allocation outlives its usefulness.
    pub fn upgrade(&self) -> Option<RcCell<T>> {
        let arc = self.weak.upgrade()?;
        let g = arc.lock().expect("rc mutex");
        if g.value.is_none() {
            return None;
        }
        drop(g);
        Some(RcCell { inner: arc })
    }
}

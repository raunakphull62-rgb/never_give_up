//! Simulated async I/O: a bounded thread pool (FOUNDATION-3 Part 2A).
//!
//! Honest labeling (see `docs/foundation3-part2-design.md` §A): this is
//! NOT true async I/O — no epoll/kqueue, no executor. Blocking calls
//! run on a fixed set of pool threads while the calling Klang task
//! parks until its result arrives. What the pool buys over `spawn` +
//! blocking call is bounded thread usage: N concurrent fetches share
//! `POOL_THREADS` reusable threads instead of spawning N fresh 8 MiB
//! threads. Results and error codes are identical to the synchronous
//! versions (same code paths, same timeouts).
//!
//! Scope: network fetches only (`http_get_async`, `http_post_async`).
//! File/stdin/process/sleep builtins stay inline and blocking.
//!
//! Pool shape (fixed, documented, not tuned per call):
//! - `POOL_THREADS = 16` workers: enough for realistic fan-out, far
//!   under the 256-task cap philosophy; each worker runs the same
//!   blocking exchange the sync path runs (10 s connect / 60 s global
//!   backstop), so a stalled server fails loudly, never hangs the pool
//!   forever — but a full pool of stalled calls DOES delay queued work
//!   for up to the backstop (stated limit, same class as the existing
//!   group-drain delay for blocking builtins).
//! - Unbounded FIFO queue: submissions beyond worker count wait their
//!   turn (no rejection, no silent drop). Shutdown: workers live for
//!   the process lifetime (same as spawned task threads); a poisoned
//!   mutex is a loud internal error, never a silent result.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, OnceLock};

/// Fixed worker count (see module docs for why 16).
pub const POOL_THREADS: usize = 16;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct PoolInner {
    queue: Mutex<VecDeque<Job>>,
    work: Condvar,
}

struct Pool {
    inner: std::sync::Arc<PoolInner>,
}

impl Pool {
    fn spawn() -> Self {
        let inner = std::sync::Arc::new(PoolInner {
            queue: Mutex::new(VecDeque::new()),
            work: Condvar::new(),
        });
        for i in 0..POOL_THREADS {
            let inner = inner.clone();
            // Workers run for the process lifetime (same as spawned task
            // threads): they park on the condvar when idle, so an idle
            // pool costs nothing but address space.
            std::thread::Builder::new()
                .name(format!("klang-pool-{i}"))
                .stack_size(crate::runtime::SPAWN_STACK_BYTES)
                .spawn(move || loop {
                    let job = {
                        let mut q = inner.queue.lock().expect("pool queue lock");
                        while q.is_empty() {
                            q = inner.work.wait(q).expect("pool condvar");
                        }
                        q.pop_front().expect("nonempty queue")
                    };
                    job();
                })
                .expect("pool worker spawns");
        }
        Self { inner }
    }

    fn global() -> &'static Pool {
        static POOL: OnceLock<Pool> = OnceLock::new();
        POOL.get_or_init(Pool::spawn)
    }

    fn submit(&'static self, job: Job) {
        let mut q = self.inner.queue.lock().expect("pool queue lock");
        q.push_back(job);
        self.inner.work.notify_one();
    }
}

/// Run `f` on the shared pool and park the caller until it completes.
/// The result (including `Err` diagnostics) is exactly what `f` would
/// have produced inline — the pool changes *where* it runs, never
/// *what* it computes.
pub fn run_on_pool<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    Pool::global().submit(Box::new(move || {
        let _ = tx.send(f());
    }));
    rx.recv().map_err(|_| "async pool worker vanished".to_string())
}

//! `klang repair`: compiler-in-the-loop repair mechanism (Phase 4).
//!
//! Phase 4 correction: Klang no longer calls a model. The harness the
//! user already runs owns the model connection; what lives here is the
//! verification mechanism the harness drives — via `klang mcp`
//! (`klang_check`, `klang_scope_plan`, ...) or the library API — plus
//! the `klang repair --dry-run` prompt planner that shows exactly what
//! signal a harness would get.
//!
//! Layout: `config` (attempt budget + scope), `backend` (`ModelBackend`
//! trait + deterministic mock), `prompt` (system/user template),
//! `scope` (function-vs-file planner), `splice` (deterministic
//! recombination + full re-check), `driver` (bounded loop + logging),
//! `log` (`.klang-repair-log.json` writer).
//!
//! No network, no endpoints, no keys: nothing in this module can dial
//! out. JSON stays hand-rolled like `diagnostics::Diagnostic::to_json`.
//! No new Cargo dependencies.

pub mod backend;
pub mod config;
pub mod driver;
pub mod log;
pub mod prompt;
pub mod scope;
pub mod splice;

pub use backend::{MockBackend, ModelBackend, ModelError};
pub use config::{RepairCliOverrides, RepairConfig, RepairFileConfig, Scope};
pub use driver::{AttemptLog, RepairOutcome, run_repair};
pub use log::write_log;
pub use prompt::{build_system_prompt, build_user_prompt};
pub use scope::plan_scope;
pub use splice::{check_candidate, is_unsafe_import_path};

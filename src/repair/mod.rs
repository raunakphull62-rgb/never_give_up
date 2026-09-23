//! `klang repair`: compiler-in-the-loop LLM repair (PRD v1).
//!
//! Given Klang source that fails `check`, drive an LLM through bounded,
//! diagnostic-guided regeneration until the code checks clean or the
//! attempt budget is exhausted.
//!
//! Layout: `config` (flag->file->env precedence), `backend`
//! (`ModelBackend` + OpenAI-compatible `curl` impl + mock), `prompt`
//! (system/user template), `scope` (function-vs-file planner), `splice`
//! (deterministic recombination + full re-check), `driver` (bounded loop
//! + logging), `log` (`.klang-repair-log.json` writer).
//!
//! Std-only: HTTP goes through the system `curl` binary (TLS included),
//! JSON is hand-rolled like `diagnostics::Diagnostic::to_json`. No new
//! Cargo dependencies.

pub mod backend;
pub mod config;
pub mod driver;
pub mod log;
pub mod prompt;
pub mod scope;
pub mod splice;

pub use backend::{MockBackend, ModelBackend, ModelError, OpenAiCurlBackend};
pub use config::{RepairCliOverrides, RepairConfig, RepairFileConfig, Scope};
pub use driver::{AttemptLog, RepairOutcome, run_repair};
pub use log::write_log;
pub use prompt::{build_system_prompt, build_user_prompt};
pub use scope::plan_scope;
pub use splice::check_candidate;

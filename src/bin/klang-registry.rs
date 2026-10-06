//! `klang-registry`: the hosted package-registry service (FOUNDATION-3
//! Part 2B, v1 + PRD §4 owner-only publishing). Serves the package index
//! + content-addressed archives from a data directory (local disk) or a
//! Backblaze B2 bucket (S3-compatible API); see `klang::registry` for the
//! protocol and `klang::registry_storage` for the backends.
//!
//! No TLS here (localhost or a TLS-terminating proxy only) and a single
//! owner bearer token — the documented v1 scope. Configuration via flags
//! or environment (`PORT`, `REGISTRY_DATA_DIR`, `KLANG_REGISTRY_TOKEN`
//! / `REGISTRY_ADMIN_TOKEN`, plus `KLANG_STORAGE` / `B2_*` for B2).
//!
//! S1 (fail closed): the server starts even without a token, but every
//! write returns `503 publishing disabled: server token not configured`
//! until a ≥32-char token is configured. Reads always work.
//!
//! Storage: `KLANG_STORAGE=b2` selects the B2 backend (needs `B2_KEY_ID`,
//! `B2_APP_KEY`, `B2_BUCKET`, `B2_ENDPOINT`, optionally `B2_REGION`).
//! Anything else (or unset) selects local disk. With `b2` and a missing
//! variable the server exits at startup naming the variable — values are
//! never printed.

use std::net::TcpListener;

fn usage() -> ! {
    eprintln!("usage: klang-registry [--port N] [--data-dir PATH] [--admin-token TOKEN]");
    eprintln!("  env: PORT, REGISTRY_DATA_DIR (default ./registry-data), KLANG_REGISTRY_TOKEN or REGISTRY_ADMIN_TOKEN (publishing disabled when unset/<32 chars)");
    eprintln!("  storage env: KLANG_STORAGE (local|b2, default local), B2_KEY_ID, B2_APP_KEY, B2_BUCKET, B2_ENDPOINT, B2_REGION (optional)");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut port: Option<u16> = std::env::var("PORT").ok().and_then(|v| v.parse().ok());
    let mut data_dir: Option<String> = std::env::var("REGISTRY_DATA_DIR").ok();
    let mut token: Option<String> = std::env::var("REGISTRY_ADMIN_TOKEN").ok();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                port = args.get(i).and_then(|v| v.parse().ok()).or_else(|| usage());
            }
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).cloned().or_else(|| usage());
            }
            "--admin-token" => {
                i += 1;
                token = args.get(i).cloned().or_else(|| usage());
            }
            "--help" | "-h" => usage(),
            _ => usage(),
        }
        i += 1;
    }
    let port = port.unwrap_or(klang::registry::DEFAULT_PORT);
    let data_dir = data_dir.unwrap_or_else(|| "./registry-data".to_string());
    let token = token.filter(|t| !t.is_empty()).unwrap_or_default();
    if !klang::registry::publishing_enabled(&token) {
        eprintln!("klang-registry: WARNING publishing disabled (set KLANG_REGISTRY_TOKEN to a >=32-char secret to enable writes); reads still work");
    }
    let listener = TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|e| {
        eprintln!("klang-registry: cannot bind 0.0.0.0:{port}: {e}");
        std::process::exit(1);
    });
    // Backend selection happens once at startup. A `b2` misconfiguration
    // exits here with the missing variable named (values never printed).
    let backend = klang::registry_storage::backend_from_env().unwrap_or_else(|e| {
        eprintln!("klang-registry: storage misconfigured: {e}");
        std::process::exit(1);
    });
    let backend_label = klang::registry_storage::backend_name(&backend);
    let storage = klang::registry_storage::open_storage(&backend, std::path::Path::new(&data_dir))
        .unwrap_or_else(|e| {
            eprintln!("klang-registry: cannot open storage backend `{backend_label}`: {e}");
            std::process::exit(1);
        });
    // Only the backend name is logged — never tokens, keys, or buckets.
    eprintln!("klang-registry: serving {data_dir} on 0.0.0.0:{port} (storage: {backend_label})");
    let cfg = klang::registry::RegistryConfig {
        data_dir: data_dir.into(),
        admin_token: token,
    };
    let label: &'static str = if matches!(backend, klang::registry_storage::StorageBackend::B2(_)) {
        "b2"
    } else {
        "local"
    };
    if let Err(e) = klang::registry::serve_with_storage(listener, cfg, storage, label) {
        eprintln!("klang-registry: {e}");
        std::process::exit(1);
    }
}

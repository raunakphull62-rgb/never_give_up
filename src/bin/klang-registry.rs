//! `klang-registry`: the hosted package-registry service (FOUNDATION-3
//! Part 2B, v1). Serves the package index + content-addressed archives
//! from a data directory; see `klang::registry` for the protocol.
//!
//! No TLS here (localhost or a TLS-terminating proxy only) and a single
//! admin bearer token — the documented v1 scope. Configuration via flags
//! or environment (`PORT`, `REGISTRY_DATA_DIR`, `REGISTRY_ADMIN_TOKEN`).

use std::net::TcpListener;

fn usage() -> ! {
    eprintln!("usage: klang-registry [--port N] [--data-dir PATH] [--admin-token TOKEN]");
    eprintln!("  env: PORT, REGISTRY_DATA_DIR (default ./registry-data), REGISTRY_ADMIN_TOKEN (required)");
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
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        eprintln!("klang-registry: missing admin token (pass --admin-token or set REGISTRY_ADMIN_TOKEN)");
        std::process::exit(2);
    };
    let listener = TcpListener::bind(("0.0.0.0", port)).unwrap_or_else(|e| {
        eprintln!("klang-registry: cannot bind 0.0.0.0:{port}: {e}");
        std::process::exit(1);
    });
    eprintln!("klang-registry: serving {data_dir} on 0.0.0.0:{port}");
    let cfg = klang::registry::RegistryConfig {
        data_dir: data_dir.into(),
        admin_token: token,
    };
    if let Err(e) = klang::registry::serve(listener, cfg) {
        eprintln!("klang-registry: {e}");
        std::process::exit(1);
    }
}

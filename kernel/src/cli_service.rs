//! `kerna start` and `kerna status`: the runtime service (the local
//! dashboard control room) and an honest one-screen summary of everything
//! the product believes about this machine.

use crate::{cli_providers, credentials};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceState {
    pub pid: u32,
    pub port: u16,
    pub started_at: String,
}

fn state_path() -> std::path::PathBuf {
    let profile = crate::guard_routing::demo_profile_path();
    profile
        .parent()
        .map(|dir| dir.join("service.json"))
        .unwrap_or_else(|| std::env::temp_dir().join("kerna-service.json"))
}

pub fn port_is_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(400),
    )
    .is_ok()
}

fn read_state() -> Option<ServiceState> {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|text| serde_json::from_str::<ServiceState>(&text).ok())
}

/// Idempotently bring the detached dashboard service up. Returns its port and
/// whether this call started it.
fn ensure_up(port: u16) -> Result<(u16, bool)> {
    if let Some(state) = read_state() {
        if port_is_open(state.port) {
            return Ok((state.port, false));
        }
    }
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(exe);
    command.args(["dashboard", "--port", &port.to_string(), "--no-open"]);
    // The child must not inherit the launcher's working directory, or a
    // detached service keeps whichever repo folder started it open.
    if let Some(parent) = state_path().parent() {
        let _ = std::fs::create_dir_all(parent);
        command.current_dir(parent);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED | NEW_PROCESS_GROUP
    }
    let child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let state = ServiceState {
        pid: child.id(),
        port,
        started_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Some(parent) = state_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(state_path(), serde_json::to_vec_pretty(&state)?)?;
    // Don't claim "running" until the port actually answers; a crashed child
    // must surface as a failure, not a confident lie.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !port_is_open(port) {
        if std::time::Instant::now() >= deadline {
            let _ = std::fs::remove_file(state_path());
            anyhow::bail!(
                "the Kerna service started but never answered on port {port}; \
                 run `kerna advanced dashboard --port {port}` to see why"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok((port, true))
}

/// Starts the dashboard control room as a detached background service. The
/// native `kerna code` path needs no service; this exists for the plan's
/// `kerna start` and for watching approvals from the browser.
pub fn start(port: u16) -> Result<()> {
    let (port, started_now) = ensure_up(port)?;
    if started_now {
        println!("✓ Kerna is running on http://127.0.0.1:{port}/");
    } else {
        println!("Kerna is already running on http://127.0.0.1:{port}/");
    }
    Ok(())
}

/// The dashboard pointer shown on the `kerna code` start screen. A service
/// that can't come up is a hint, never a reason to block the governed path.
pub fn session_dashboard() {
    match ensure_up(8765) {
        Ok((port, started_now)) => {
            let suffix = if started_now {
                crate::cli_brand::accent(245, "  (started just now for this session)")
            } else {
                String::new()
            };
            println!(
                "{} {}{suffix}",
                crate::cli_brand::accent(245, "Dashboard"),
                crate::cli_brand::accent(208, &format!("http://127.0.0.1:{port}/"))
            );
        }
        Err(_) => println!(
            "{}",
            crate::cli_brand::accent(
                245,
                "Dashboard offline — run `kerna start` to watch receipts live"
            )
        ),
    }
}

fn evidence_bundles(count: usize) -> Vec<std::path::PathBuf> {
    let temp = std::env::temp_dir();
    let Ok(entries) = std::fs::read_dir(&temp) else {
        return Vec::new();
    };
    let mut found = entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("kerna-code-evidence-")
        })
        .filter_map(|entry| {
            let path = entry.path().join("native-code-evidence.json");
            std::fs::metadata(&path).ok().map(|meta| (meta, path))
        })
        .collect::<Vec<_>>();
    found.sort_by(|a, b| {
        b.0.modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .cmp(&a.0.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH))
    });
    found
        .into_iter()
        .take(count)
        .map(|(_, path)| path)
        .collect()
}

/// The one-screen "what does Kerna believe right now" view.
pub fn run() -> Result<()> {
    println!("Kerna {}", env!("CARGO_PKG_VERSION"));
    println!();
    print!("Cloud   ");
    let configured: Vec<_> = cli_providers::PROVIDERS
        .iter()
        .filter(|provider| cli_providers::source(provider) != credentials::CredentialSource::None)
        .map(|provider| cli_providers::label(provider))
        .collect();
    if configured.is_empty() {
        println!("— none configured   (run `kerna provider add`)");
    } else {
        println!("✓ {}", configured.join(", "));
    }
    // Only a profile the operator actually saved counts as configured; the
    // built-in default must not masquerade as a chosen local model.
    let local = if crate::guard_routing::demo_profile_path().is_file() {
        crate::guard_routing::load_demo_profile()
            .local_model
            .unwrap_or_else(|| "— none".to_string())
    } else {
        "— none".to_string()
    };
    println!("Local   {local}");
    let state = read_state();
    let running = state.as_ref().is_some_and(|state| port_is_open(state.port));
    match (&state, running) {
        (Some(state), true) => {
            println!("Service ✓ running on http://127.0.0.1:{}/", state.port)
        }
        _ => println!("Service stopped      (start with `kerna start`)"),
    }
    println!("Logs    {}", crate::cli_logs::log_path().display());
    let bundles = evidence_bundles(3);
    if bundles.is_empty() {
        println!("Evidence  none yet     (a run of `kerna code` writes a signed bundle)");
    } else {
        println!("Evidence {} recent bundle(s):", bundles.len());
        for bundle in bundles {
            println!("  {}", bundle.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_closed_port_is_reported_as_not_running() {
        assert!(!port_is_open(1));
    }

    #[test]
    fn service_state_round_trips_through_json() {
        let state = ServiceState {
            pid: 4242,
            port: 8765,
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        let text = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<ServiceState>(&text).unwrap(), state);
    }

    #[test]
    fn evidence_listing_never_panics_without_bundles() {
        let _ = evidence_bundles(3);
    }
}

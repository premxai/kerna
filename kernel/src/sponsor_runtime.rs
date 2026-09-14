use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MAX_CODE_BYTES: usize = 8 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_TIMEOUT_MS: u64 = 10_000;
/// Remote VM admission is separate from the guest command budget. Tenki can
/// legitimately take longer than a local Wasmer sandbox to provision a VM.
pub const TENKI_ADMISSION_TIMEOUT_MS: u64 = 60_000;
pub const TENKI_CLEANUP_GRACE_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBackend {
    Docker,
    Wasmer,
    Tenki,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRequest {
    pub backend: ExecutionBackend,
    pub language: String,
    pub code: String,
    pub timeout_ms: u64,
    #[serde(default)]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxOutcome {
    pub backend: ExecutionBackend,
    pub status: String,
    pub exit_code: i32,
    pub duration_ms: u128,
    pub output: String,
    pub output_sha256: String,
    pub package: String,
    pub network: String,
}

/// Return the stable, redacted reason for refusing a sandbox request before a
/// sponsor backend is contacted. This is deliberately conservative: Wasmer
/// and Tenki remain the isolation boundary, but Kerna must also demonstrate
/// that it owns admission policy rather than merely reporting a guest failure.
pub fn admission_reason(request: &SandboxRequest) -> Option<&'static str> {
    if request.language != "python"
        || request.code.is_empty()
        || request.code.len() > MAX_CODE_BYTES
        || request.timeout_ms == 0
        || request.timeout_ms > MAX_TIMEOUT_MS
        || request.backend == ExecutionBackend::Docker
    {
        return Some("invalid_or_unsupported_request");
    }
    if request.backend == ExecutionBackend::Tenki
        && request.auth_token.as_deref().unwrap_or_default().is_empty()
    {
        return Some("tenki_authentication_required");
    }

    let code = request.code.to_ascii_lowercase();
    let denied_patterns: &[(&str, &[&str])] = &[
        (
            "network_access",
            &[
                "urllib", "requests", "http://", "https://", "socket", "urlopen(", "ftplib", "dns",
            ],
        ),
        (
            "host_filesystem_access",
            &[
                "open(",
                "os.listdir",
                "os.walk",
                "pathlib",
                "/etc/",
                "c:/",
                "c:\\",
                "system32",
            ],
        ),
        (
            "environment_access",
            &["os.environ", "os.getenv", "getenv(", "environ["],
        ),
        (
            "process_escape",
            &[
                "subprocess",
                "os.system",
                "os.popen",
                "ctypes",
                "__import__(",
            ],
        ),
    ];
    for (reason, patterns) in denied_patterns {
        if patterns.iter().any(|pattern| code.contains(pattern)) {
            return Some(reason);
        }
    }
    None
}

/// Kerna's pre-execution admission gate. Callers should persist the request
/// receipt before invoking this function and must not call the backend when it
/// returns an error.
pub fn policy_check(request: &SandboxRequest) -> Result<()> {
    if let Some(reason) = admission_reason(request) {
        anyhow::bail!("Kerna policy denied sandbox request: {reason}");
    }
    validate(request)
}

#[derive(Debug, Deserialize)]
struct BridgeOutcome {
    status: String,
    exit_code: i32,
    output: String,
    package: String,
    network: String,
}

pub fn bridge_available() -> bool {
    bridge_path().is_some()
        && runtime_dir()
            .map(|dir| {
                dir.join("node_modules")
                    .join("@wasmer")
                    .join("sdk")
                    .exists()
            })
            .unwrap_or(false)
}

pub fn run(request: SandboxRequest) -> Result<SandboxOutcome> {
    policy_check(&request)?;
    let bridge = bridge_path().ok_or_else(|| {
        anyhow!("Kerna sponsor runtime bridge is unavailable; run the demo bootstrap")
    })?;
    let runtime = bridge
        .parent()
        .ok_or_else(|| anyhow!("invalid sponsor runtime bridge path"))?;
    let started = Instant::now();
    let mut child = Command::new("node")
        .arg(&bridge)
        .current_dir(runtime)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env(
            "SYSTEMROOT",
            std::env::var("SYSTEMROOT").unwrap_or_default(),
        )
        .env("KERNA_WASMER_CACHE", cache_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start the Kerna sponsor runtime bridge")?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("sponsor runtime stdin is unavailable"))?
        .write_all(&serde_json::to_vec(&request)?)?;
    let bridge_timeout_ms = if request.backend == ExecutionBackend::Tenki {
        request.timeout_ms + TENKI_ADMISSION_TIMEOUT_MS + TENKI_CLEANUP_GRACE_MS
    } else {
        request.timeout_ms + 1_000
    };
    let deadline = Instant::now() + Duration::from_millis(bridge_timeout_ms);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let boundary = if request.backend == ExecutionBackend::Tenki {
                "Tenki remote sandbox exceeded its bounded admission/execution window"
            } else {
                "sandbox exceeded its bounded timeout"
            };
            return Err(anyhow!(boundary));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "sponsor runtime failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .trim()
                .chars()
                .take(500)
                .collect::<String>()
        ));
    }
    let bridge: BridgeOutcome =
        serde_json::from_slice(&output.stdout).context("sponsor runtime returned invalid JSON")?;
    if bridge.output.len() > MAX_OUTPUT_BYTES {
        return Err(anyhow!(
            "sandbox output exceeded {} bytes",
            MAX_OUTPUT_BYTES
        ));
    }
    let output_sha256 = format!("{:x}", Sha256::digest(bridge.output.as_bytes()));
    Ok(SandboxOutcome {
        backend: request.backend,
        status: bridge.status,
        exit_code: bridge.exit_code,
        duration_ms: started.elapsed().as_millis(),
        output: bridge.output,
        output_sha256,
        package: bridge.package,
        network: bridge.network,
    })
}

fn validate(request: &SandboxRequest) -> Result<()> {
    if request.language != "python" {
        return Err(anyhow!("only the pinned Python WASIX runtime is supported"));
    }
    if request.code.is_empty() || request.code.len() > MAX_CODE_BYTES {
        return Err(anyhow!("code must contain 1..={MAX_CODE_BYTES} bytes"));
    }
    if request.timeout_ms == 0 || request.timeout_ms > MAX_TIMEOUT_MS {
        return Err(anyhow!("timeout_ms must be between 1 and {MAX_TIMEOUT_MS}"));
    }
    if request.backend == ExecutionBackend::Docker {
        return Err(anyhow!("this tool supports Wasmer or Tenki only"));
    }
    if request.backend == ExecutionBackend::Tenki
        && request.auth_token.as_deref().unwrap_or_default().is_empty()
    {
        return Err(anyhow!("Tenki is configured — authentication required"));
    }
    Ok(())
}

fn runtime_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KERNA_SPONSOR_RUNTIME_DIR") {
        return Some(PathBuf::from(path));
    }
    if cfg!(windows) {
        let installed = PathBuf::from(r"C:\KernaData\kerna-demo\runtime");
        if installed.join("sponsor-runtime.mjs").exists() {
            return Some(installed);
        }
    } else {
        let installed = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(std::env::temp_dir)
            .join("kerna-demo/runtime");
        if installed.join("sponsor-runtime.mjs").exists() {
            return Some(installed);
        }
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("runtime");
    source.exists().then_some(source)
}

fn bridge_path() -> Option<PathBuf> {
    runtime_dir()
        .map(|dir| dir.join("sponsor-runtime.mjs"))
        .filter(|path| path.exists())
}

fn cache_root() -> PathBuf {
    std::env::var_os("KERNA_WASMER_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(windows) {
                // The SDK's prewarmed registry lives beside the installed
                // bridge. Keep the older fallback for machines bootstrapped
                // before the runtime cache was colocated there.
                let runtime_cache = PathBuf::from(r"C:\KernaData\kerna-demo\runtime\.wasmer");
                if runtime_cache.exists() {
                    runtime_cache
                } else {
                    PathBuf::from(r"C:\KernaData\kerna-demo\wasmer-cache")
                }
            } else {
                std::env::temp_dir().join("kerna-wasmer-cache")
            }
        })
}

pub fn smoke_request() -> SandboxRequest {
    SandboxRequest {
        backend: ExecutionBackend::Wasmer,
        language: "python".to_string(),
        code: "print(sum(i * i for i in range(10)))".to_string(),
        timeout_ms: 10_000,
        auth_token: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsupported_language_and_oversized_code() {
        let mut request = smoke_request();
        request.language = "shell".to_string();
        assert!(validate(&request).is_err());
        request.language = "python".to_string();
        request.code = "x".repeat(MAX_CODE_BYTES + 1);
        assert!(validate(&request).is_err());
    }

    #[test]
    fn tenki_requires_authentication() {
        let mut request = smoke_request();
        request.backend = ExecutionBackend::Tenki;
        assert!(validate(&request).is_err());
    }

    #[test]
    fn kerna_denies_escape_intent_before_backend_contact() {
        let mut request = smoke_request();
        request.code = "print(open('/etc/passwd').read())".to_string();
        assert_eq!(admission_reason(&request), Some("host_filesystem_access"));
        assert!(policy_check(&request).is_err());

        request.code =
            "import urllib.request; urllib.request.urlopen('https://example.com')".to_string();
        assert_eq!(admission_reason(&request), Some("network_access"));
        assert!(policy_check(&request).is_err());
    }

    #[test]
    fn safe_code_is_admitted() {
        let request = smoke_request();
        assert_eq!(admission_reason(&request), None);
        assert!(policy_check(&request).is_ok());
    }
}

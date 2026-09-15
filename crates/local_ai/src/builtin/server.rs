//! Process supervision and readiness probing for `llama-server`.

use common::BoxError;
use common::config::AI_MAX_CONTEXT_TOKENS;
use reqwest::Client as ReqwestClient;
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::time::sleep;

/// Dedicated loopback port reserved for the built-in `llama-server` sidecar.
pub const BUILTIN_LLAMA_PORT: u16 = 11435;

/// Base URL for the built-in `llama-server` process.
pub const BUILTIN_BASE_URL: &str = "http://127.0.0.1:11435";

const HEALTH_POLL_ATTEMPTS: u32 = 20;
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Supervisor for managing the lifecycle of the bundled `llama-server` sidecar.
pub struct LlamaServerService {
    http_client: ReqwestClient,
    port: u16,
}

impl LlamaServerService {
    /// Creates a `LlamaServerService` on the default port (11435).
    pub fn new(http_client: ReqwestClient) -> Self {
        Self {
            http_client,
            port: BUILTIN_LLAMA_PORT,
        }
    }

    /// Returns the active base URL string.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Probes the `/health` endpoint to verify sidecar readiness.
    pub async fn is_running(&self) -> bool {
        self.http_client
            .get(format!("{}/health", self.base_url()))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Spawns the `llama-server` process with headless I/O and strict memory constraints.
    ///
    /// Configures execution arguments:
    /// - `--host 127.0.0.1` and `--port 11435`: Binds strictly to IPv4 loopback.
    /// - `-c 2048`: Caps KV cache allocation to prevent swap thrashing.
    /// - `-np 1`: Restricts concurrency to single-slot execution to protect host RAM.
    /// - `-ngl 99`: Offloads all layers to available GPU acceleration.
    /// - `-t <safe_threads>`: Restricts threads to half of available CPU cores (clamped between 1 and 4)
    ///   to keep the system responsive and avoid starving the desktop UI event loop.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if executable permissions cannot be applied or spawning fails.
    pub fn start_process(&self, binary_path: &Path, model_path: &Path) -> Result<Child, BoxError> {
        // Ensure the bundled binary has executable permissions on Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(binary_path) {
                let mut perms = metadata.permissions();
                if perms.mode() & 0o111 == 0 {
                    perms.set_mode(0o755);
                    let _ = std::fs::set_permissions(binary_path, perms);
                }
            }
        }

        let mut cmd = Command::new(binary_path);

        let total_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let safe_threads = (total_threads / 2).clamp(1, 4);

        cmd.arg("-m")
            .arg(model_path)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(self.port.to_string())
            .arg("-c")
            .arg(AI_MAX_CONTEXT_TOKENS.to_string())
            .arg("-np")
            .arg("1")
            .arg("-ngl")
            .arg("99")
            .arg("-t")
            .arg(safe_threads.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());

        // On Linux, ensure bundled shared libraries (e.g. libggml.so) beside the binary are discoverable
        #[cfg(target_os = "linux")]
        if let Some(parent) = binary_path.parent() {
            let current_ld = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
            let new_ld = if current_ld.is_empty() {
                parent.to_string_lossy().to_string()
            } else {
                format!("{}:{}", parent.to_string_lossy(), current_ld)
            };
            cmd.env("LD_LIBRARY_PATH", new_ld);
        }

        // On Windows, suppress flashing console windows
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn().map_err(|e| {
            format!(
                "failed to launch `llama-server` at {}: {e}",
                binary_path.display()
            )
        })?;

        Ok(child)
    }

    /// Polls the `/health` endpoint until the server reports ready or times out after 10 seconds.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if the server does not report ready within the timeout window.
    pub async fn wait_until_ready(&self) -> Result<(), BoxError> {
        for _ in 0..HEALTH_POLL_ATTEMPTS {
            if self.is_running().await {
                return Ok(());
            }
            sleep(HEALTH_POLL_INTERVAL).await;
        }

        Err(format!(
            "`llama-server` failed to become ready on 127.0.0.1:{}",
            self.port
        )
        .into())
    }
}
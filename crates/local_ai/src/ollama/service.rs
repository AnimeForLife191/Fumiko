//! Process management and model registry interactions for the external Ollama daemon.

use common::BoxError;
use futures_util::StreamExt;
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;

use super::DEFAULT_OLLAMA_URL;
use super::models::{list_available_models, names_match};

const HEALTH_POLL_ATTEMPTS: u32 = 10;
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Outcome returned when attempting to launch `ollama serve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServeOutcome {
    /// The Ollama daemon was already running and listening on the network port.
    AlreadyRunning,
    /// The process was spawned and successfully passed readiness polling.
    StartedNow,
}

#[derive(Debug, Serialize)]
struct PullRequest<'a> {
    name: &'a str,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct DeleteRequest<'a> {
    model: &'a str,
}

/// Progress event emitted during streaming model pulls from Ollama's registry.
#[derive(Debug, Deserialize, Clone, Serialize, PartialEq)]
pub struct PullProgress {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub completed: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

impl PullProgress {
    /// Returns the progress fraction completed between `0.0` and `1.0`.
    pub fn fraction(&self) -> Option<f32> {
        match (self.completed, self.total) {
            (Some(completed), Some(total)) if total > 0 => Some(completed as f32 / total as f32),
            _ => None,
        }
    }

    /// Returns `true` if this progress frame indicates successful completion.
    pub fn is_success(&self) -> bool {
        self.status.as_deref() == Some("success")
    }
}

/// Service controller for communicating with an Ollama daemon.
pub struct OllamaService {
    http_client: ReqwestClient,
    base_url: String,
}

impl OllamaService {
    /// Creates an `OllamaService` targeting the default loopback endpoint (`127.0.0.1:11434`).
    pub fn new(http_client: ReqwestClient) -> Self {
        Self::with_base_url(http_client, DEFAULT_OLLAMA_URL)
    }

    /// Creates an `OllamaService` targeting a custom daemon URL.
    pub fn with_base_url(http_client: ReqwestClient, base_url: impl Into<String>) -> Self {
        Self {
            http_client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Checks whether the Ollama daemon is actively responding on loopback.
    pub async fn is_running(&self) -> bool {
        self.http_client
            .get(format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false)
    }

    /// Spawns `ollama serve` in the background if not already running.
    ///
    /// Configures environment variables to limit concurrency to single-slot execution:
    /// - `OLLAMA_NUM_PARALLEL=1`: Prevents allocating multiple KV cache buffers simultaneously.
    /// - `OLLAMA_MAX_LOADED_MODELS=1`: Prevents pinning multiple heavy model weights in system RAM.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if spawning the binary fails or if the daemon fails to respond
    /// within 5 seconds.
    pub async fn serve(&self) -> Result<ServeOutcome, BoxError> {
        if self.is_running().await {
            return Ok(ServeOutcome::AlreadyRunning);
        }

        let mut cmd = Command::new("ollama");
        cmd.arg("serve")
            .env("OLLAMA_NUM_PARALLEL", "1")
            .env("OLLAMA_MAX_LOADED_MODELS", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());

        // On Windows, suppress flashing console windows
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.spawn().map_err(|e| {
            format!("failed to spawn `ollama serve`, is Ollama installed and on PATH? ({e})")
        })?;

        for _ in 0..HEALTH_POLL_ATTEMPTS {
            if self.is_running().await {
                return Ok(ServeOutcome::StartedNow);
            }
            sleep(HEALTH_POLL_INTERVAL).await;
        }

        Err(format!(
            "`ollama serve` was started but never responded on {}",
            self.base_url
        )
        .into())
    }

    /// Checks whether a model tag has already been pulled and is available locally.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if listing installed models fails.
    pub async fn is_model_pulled(&self, model: &str) -> Result<bool, BoxError> {
        let models = list_available_models(&self.http_client).await?;
        Ok(models.iter().any(|m| names_match(&m.name, model)))
    }

    /// Streams a model pull from the Ollama registry, emitting progress events to the callback.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if the download is interrupted, fails HTTP validation, or reports an error payload.
    pub async fn pull_model(
        &self,
        model: &str,
        on_progress: &mut dyn FnMut(PullProgress),
    ) -> Result<(), BoxError> {
        let response = self
            .http_client
            .post(format!("{}/api/pull", self.base_url))
            .json(&PullRequest {
                name: model,
                stream: true,
            })
            .send()
            .await?
            .error_for_status()?;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut saw_success = false;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_idx) = buffer.find('\n') {
                let line = buffer[..newline_idx].trim().to_string();
                buffer.drain(..=newline_idx);

                if line.is_empty() {
                    continue;
                }

                let progress: PullProgress = serde_json::from_str(&line).map_err(|e| {
                    format!("failed to parse pull progress line: {e}, raw line: {line}")
                })?;

                if let Some(err) = &progress.error {
                    return Err(format!("ollama pull error: {err}").into());
                }

                saw_success = saw_success || progress.is_success();
                on_progress(progress);
            }
        }

        if saw_success {
            Ok(())
        } else {
            Err(format!("pull for model '{model}' ended without a success status").into())
        }
    }

    /// Deletes a model from the local Ollama registry via `DELETE /api/delete`.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if the deletion request fails.
    pub async fn delete_model(&self, model: &str) -> Result<(), BoxError> {
        self.http_client
            .delete(format!("{}/api/delete", self.base_url))
            .json(&DeleteRequest { model })
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Ensures that the daemon is active and the specified model is installed, pulling if absent.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if starting the daemon or pulling the model fails.
    pub async fn ensure_ready(
        &self,
        model: &str,
        on_progress: &mut dyn FnMut(PullProgress),
    ) -> Result<(), BoxError> {
        self.serve().await?;

        if self.is_model_pulled(model).await? {
            return Ok(());
        }

        self.pull_model(model, on_progress).await
    }
}
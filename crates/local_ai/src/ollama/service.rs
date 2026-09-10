use common::BoxError;
use futures_util::StreamExt;
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;

use crate::ollama::models_list::names_match;

use super::OLLAMA_BASE_URL;
use super::models_list::{OllamaModel, list_available_models};

const HEALTH_POLL_ATTEMPTS: u32 = 10;
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Outcome returned when ensuring the Ollama background daemon is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServeOutcome {
    /// The daemon was already listening on `127.0.0.1:11434`.
    AlreadyRunning,
    /// The daemon was spawned as a background process and passed readiness polling.
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

/// Progress update chunk received while streaming a model download.
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
    /// Server error reported inline within the streaming NDJSON payload.
    #[serde(default)]
    pub error: Option<String>,
}

impl PullProgress {
    /// Returns the current download progress as a fractional value between `0.0` and `1.0`.
    pub fn fraction(&self) -> Option<f32> {
        match (self.completed, self.total) {
            (Some(completed), Some(total)) if total > 0 => Some(completed as f32 / total as f32),
            _ => None,
        }
    }

    /// Checks if this progress chunk indicates download completion.
    pub fn is_success(&self) -> bool {
        self.status.as_deref() == Some("success")
    }
}

/// Manages the operational lifecycle of the local Ollama background daemon and model downloads.
pub struct OllamaService {
    http_client: ReqwestClient,
}

impl OllamaService {
    /// Creates a new `OllamaService` instance.
    pub fn new(http_client: ReqwestClient) -> Self {
        Self { http_client }
    }

    /// Checks if the Ollama daemon is currently responsive on `127.0.0.1:11434`.
    pub async fn is_running(&self) -> bool {
        self.http_client
            .get(format!("{OLLAMA_BASE_URL}/api/tags"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|resp| resp.status().is_success())
            .unwrap_or(false)
    }

    /// Spawns `ollama serve` as a background process if inactive and polls for readiness.
    pub async fn serve(&self) -> Result<ServeOutcome, BoxError> {
        if self.is_running().await {
            return Ok(ServeOutcome::AlreadyRunning);
        }

        let mut cmd = Command::new("ollama");
        cmd.arg("serve")
            // Prevent Ollama from allocating multiple concurrent model slots
            .env("OLLAMA_NUM_PARALLEL", "1")
            .env("OLLAMA_MAX_LOADED_MODELS", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());

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

        Err("`ollama serve` was started but never came up on 127.0.0.1:11434".into())
    }

    /// Checks if a model tag is already pulled and available locally.
    pub async fn is_model_pulled(&self, model: &str) -> Result<bool, BoxError> {
        let models: Vec<OllamaModel> = list_available_models(&self.http_client).await?;
        Ok(models.iter().any(|m| names_match(&m.name, model)))
    }

    /// Streams a model pull from the Ollama registry, notifying the progress callback on updates.
    pub async fn pull_model(
        &self,
        model: &str,
        on_progress: &mut dyn FnMut(PullProgress),
    ) -> Result<(), BoxError> {
        let response = self
            .http_client
            .post(format!("{OLLAMA_BASE_URL}/api/pull"))
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

            // Buffer incomplete chunks across line boundaries.
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

    /// Deletes a model from local storage (`DELETE /api/delete`).
    pub async fn delete_model(&self, model: &str) -> Result<(), BoxError> {
        self.http_client
            .delete(format!("{OLLAMA_BASE_URL}/api/delete"))
            .json(&DeleteRequest { model })
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// Ensures the Ollama server is running and downloads the model if not already present.
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
//! Streaming GGUF model downloader with atomic file replacement.

use std::path::{Path, PathBuf};

use common::BoxError;
use futures_util::StreamExt;
use reqwest::Client as ReqwestClient;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;

/// Manages streaming downloads of quantized GGUF weights.
pub struct ModelDownloader {
    http_client: ReqwestClient,
}

impl ModelDownloader {
    /// Creates a new `ModelDownloader`.
    pub fn new(http_client: ReqwestClient) -> Self {
        Self { http_client }
    }

    /// Checks if a model filename exists inside the models directory.
    pub fn is_model_downloaded(models_dir: &Path, filename: &str) -> bool {
        models_dir.join(filename).exists()
    }

    /// Enumerates all `.gguf` files present in the models directory.
    pub fn list_downloaded_models(models_dir: &Path) -> Vec<PathBuf> {
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(models_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("gguf") {
                    results.push(path);
                }
            }
        }
        results
    }

    /// Streams a GGUF model file from a remote URL to the destination path.
    ///
    /// Writes to a temporary `.part` file first and renames atomically upon completion,
    /// preventing corrupted model files if the download is cancelled or loses connection.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if the network stream fails or filesystem writes fail.
    pub async fn download_model(
        &self,
        url: &str,
        dest_path: &Path,
        mut on_progress: impl FnMut(f32),
    ) -> Result<(), BoxError> {
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // 1. Write directly to temporary "{filename}.part" file
        let temp_filename = format!(
            "{}.part",
            dest_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("model.gguf")
        );
        let temp_path = dest_path.with_file_name(temp_filename);

        let response = self
            .http_client
            .get(url)
            .header(reqwest::header::USER_AGENT, "Fumiko/0.2.0")
            .send()
            .await?
            .error_for_status()?;

        let total_size = response.content_length().unwrap_or(0);

        let file = File::create(&temp_path).await?;
        let mut writer = tokio::io::BufWriter::with_capacity(1024 * 1024, file);
        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_reported_pct: u32 = 0;

        // 2. Stream chunks and report integer percentage progress
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            writer.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            if total_size > 0 {
                let current_pct = ((downloaded as f32 / total_size as f32) * 100.0) as u32;
                if current_pct > last_reported_pct {
                    last_reported_pct = current_pct;
                    on_progress(current_pct as f32 / 100.0);
                }
            }
        }

        writer.flush().await?;
        drop(writer);

        // 3. Atomically rename .part file to final .gguf destination
        if dest_path.exists() {
            let _ = fs::remove_file(dest_path).await;
        }

        fs::rename(&temp_path, dest_path).await?;
        on_progress(1.0);

        Ok(())
    }

    /// Deletes a local model file from disk.
    ///
    /// # Errors
    /// Returns an [`std::io::Error`] if the file cannot be removed.
    pub fn delete_model(models_dir: &Path, filename: &str) -> std::io::Result<()> {
        let path = models_dir.join(filename);
        if path.exists() {
            std::fs::remove_file(path)?;
        }

        Ok(())
    }
}
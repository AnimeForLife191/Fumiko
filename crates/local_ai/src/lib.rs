//! Local AI inference engine for Fumiko.
//!
//! Evaluates incoming emails directly on the user's hardware. Subject lines,
//! preview snippets, and message bodies remain strictly in memory on loopback
//! and are never transmitted to external cloud providers.
//!
//! # Multi-Backend Architecture
//! - **Built-in Sidecar (`builtin`)**: Zero-setup bundled engine powered by `llama-server`
//!   listening on `http://127.0.0.1:11435`. Directly streams verified GGUF weights from
//!   Hugging Face into OS application directories without terminal tools.
//! - **External Daemon (`ollama`)**: Optional integration for power users running an existing
//!   Ollama daemon on `http://127.0.0.1:11434`.
//!
//! # Memory & Resource Safeguards
//! - **Context Window Capping**: Context lengths are strictly capped to 2,048 tokens (`-c 2048`),
//!   reducing KV cache allocation to ~250 MB.
//! - **Generation Bounds**: Output generation is restricted to 128 tokens (`n_predict: 128`)
//!   for compact structured JSON triage.
//! - **Single-Slot Execution**: Concurrency is limited to single-slot execution (`-np 1` /
//!   `OLLAMA_NUM_PARALLEL=1`) to prevent duplicate model weights from hogging system RAM.

pub mod builtin;
pub mod ollama;
pub mod prompt;

pub use builtin::{
    BUILTIN_CATALOG, GgufModelInfo, LlamaServerClassifier, LlamaServerService, ModelDownloader,
    default_models_dir, find_llama_server_binary,
};
pub use ollama::{
    ModelEntry, ModelTier, OllamaClassifier, OllamaModel, OllamaService, PULLABLE_MODELS,
    PullProgress, PullableModel, list_available_models, list_models_view,
};

use common::BoxError;
use serde::{Deserialize, Serialize};

/// Classification rule consisting of a unique label and natural language description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Criterion {
    /// Short human-readable identifier (e.g. `"Careers & Interviews"`).
    pub label: String,
    /// Detailed prompt instructions defining what kinds of emails qualify.
    pub description: String,
}

/// Output result produced when an email matches a configured criterion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    /// Canonical label of the matched [`Criterion`].
    pub criterion_label: String,
    /// Model confidence score clamped between `0.0` and `1.0`.
    pub confidence: f32,
}

/// Unified classifier interface for email triage across local AI backends.
#[async_trait::async_trait]
pub trait EmailClassifier: Send + Sync {
    /// Evaluates an email using its subject, sender address, and preview snippet (Tier 1).
    ///
    /// # Errors
    /// Returns a [`BoxError`] if network communication with the inference server fails
    /// or if the model produces unparseable output.
    async fn classify(
        &self,
        subject: &str,
        sender: &str,
        snippet: Option<&str>,
        criteria: &[Criterion],
    ) -> Result<Option<Classification>, BoxError>;

    /// Evaluates an email using its full or truncated plain-text message body (Tier 2).
    ///
    /// # Errors
    /// Returns a [`BoxError`] if network communication with the inference server fails
    /// or if the model produces unparseable output.
    async fn classify_with_body(
        &self,
        subject: &str,
        sender: &str,
        body: &str,
        criteria: &[Criterion],
    ) -> Result<Option<Classification>, BoxError>;
}
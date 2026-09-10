//! Local AI inference engine and daemon management for Ollama.
//!
//! Evaluates incoming emails directly on the user's hardware via local Ollama models.
//! Subject, sender, snippet previews, and message bodies remain strictly in memory on
//! `127.0.0.1` and are never transmitted to cloud providers.

mod models_list;
mod ollama;
mod service;

const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

pub use models_list::{
    ModelEntry, list_available_models, list_models_view,
};
pub use ollama::{Criterion, OllamaClassifier};
pub use service::{OllamaService, PullProgress};
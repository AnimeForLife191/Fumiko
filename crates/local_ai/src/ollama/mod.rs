//! External Ollama daemon integration, model management, and inference client.

mod classifier;
mod models;
mod service;

/// Default loopback endpoint for an external Ollama daemon.
pub const DEFAULT_OLLAMA_URL: &str = "http://127.0.0.1:11434";

pub use classifier::OllamaClassifier;
pub use models::{
    ModelEntry, ModelTier, OllamaModel, PULLABLE_MODELS, PullableModel, list_available_models,
    list_models_view,
};
pub use service::{OllamaService, PullProgress, ServeOutcome};
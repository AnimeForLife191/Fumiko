mod ollama;

pub use ollama::{
    Criterion, ModelEntry, OllamaClassifier, OllamaService, PullProgress, list_available_models,
    list_models_view,
};

//! Catalog of quantized GGUF models available for built-in streaming download.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Metadata describing a quantized GGUF model file hosted on Hugging Face.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GgufModelInfo {
    /// Unique internal model identifier.
    pub id: Cow<'static, str>,
    /// User-facing display title with performance guidance.
    pub display_name: Cow<'static, str>,
    /// Destination filename on disk (e.g. `Llama-3.2-1B-Instruct-Q4_K_M.gguf`).
    pub filename: Cow<'static, str>,
    /// Direct HTTPS URL for streaming model weights.
    pub download_url: Cow<'static, str>,
    /// Approximate download size in megabytes.
    pub size_mb: u64,
    /// Detailed capability and resource description.
    pub description: Cow<'static, str>,
}

/// Curated catalog of GGUF weights.
pub const BUILTIN_CATALOG: &[GgufModelInfo] = &[
    GgufModelInfo {
        id: Cow::Borrowed("llama-3.2-1b-instruct"),
        display_name: Cow::Borrowed("Llama 3.2 1B (Recommended)"),
        filename: Cow::Borrowed("Llama-3.2-1B-Instruct-Q4_K_M.gguf"),
        download_url: Cow::Borrowed(
            "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        ),
        size_mb: 810,
        description: Cow::Borrowed(
            "Fastest option (~800 MB). Low RAM usage and strong classification.",
        ),
    },
    GgufModelInfo {
        id: Cow::Borrowed("qwen-2.5-1.5b-instruct"),
        display_name: Cow::Borrowed("Qwen 2.5 1.5B (Higher Accuracy)"),
        filename: Cow::Borrowed("qwen2.5-1.5b-instruct-q4_k_m.gguf"),
        download_url: Cow::Borrowed(
            "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        ),
        size_mb: 1120,
        description: Cow::Borrowed(
            "Slightly larger (~1 GB). Great instruction following for complex watch rules.",
        ),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_catalog_integrity() {
        let mut seen_ids = std::collections::HashSet::new();

        for model in BUILTIN_CATALOG {
            assert!(
                seen_ids.insert(&model.id),
                "duplicate model ID: {}",
                model.id
            );
            assert!(
                model.filename.ends_with(".gguf"),
                "filename must end in .gguf"
            );
            assert!(
                model.download_url.starts_with("https://"),
                "download url must be https"
            );
            assert!(model.size_mb > 0, "model size must be greater than 0");
            assert!(!model.display_name.is_empty());
        }
    }
}
//! Data structures and catalogs for Ollama model tracking.

use super::DEFAULT_OLLAMA_URL;
use common::BoxError;
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Model installed locally inside the user's Ollama daemon.
#[derive(Debug, Deserialize, Clone, Serialize, PartialEq, Eq)]
pub struct OllamaModel {
    /// Model tag identifier (e.g. `"llama3.2:1b"`).
    pub name: String,
    /// Total file size on disk in bytes.
    pub size: u64,
    /// ISO-8601 modification timestamp.
    pub modified_at: String,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

/// Retrieves all installed models from the running Ollama instance via `GET /api/tags`.
///
/// # Errors
/// Returns a [`BoxError`] if the daemon is offline or returns a non-200 HTTP status code.
pub async fn list_available_models(
    http_client: &ReqwestClient,
) -> Result<Vec<OllamaModel>, BoxError> {
    let response: OllamaTagsResponse = http_client
        .get(format!("{DEFAULT_OLLAMA_URL}/api/tags"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(response.models)
}

/// The evaluation scan tier in Fumiko's two-tier classification pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelTier {
    /// Tier 1 scan: Fast initial evaluation using only the sender, subject, and snippet preview.
    Tier1,
    /// Tier 2 scan: Deeper inspection evaluating the message body when Tier 1 is ambiguous.
    Tier2,
}

/// Curated model entry available for in-app download via Ollama.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullableModel {
    /// Registry tag string (e.g. `"llama3.2:1b"`).
    pub name: Cow<'static, str>,
    /// Approximate download size in gigabytes.
    pub approx_size_gb: f32,
    /// Triage evaluation tier.
    pub tier: ModelTier,
    /// Human-readable recommendation notes.
    pub description: Cow<'static, str>,
}

/// Catalog of models recommended for Ollama users.
pub const PULLABLE_MODELS: &[PullableModel] = &[
    PullableModel {
        name: Cow::Borrowed("llama3.2:1b"),
        approx_size_gb: 1.3,
        tier: ModelTier::Tier1,
        description: Cow::Borrowed("Smallest, fastest option though may be less accurate"),
    },
    PullableModel {
        name: Cow::Borrowed("llama3.2:3b"),
        approx_size_gb: 2.0,
        tier: ModelTier::Tier1,
        description: Cow::Borrowed("A bit more accurate than the 1b if you can spare the RAM."),
    },
    PullableModel {
        name: Cow::Borrowed("gemma2:2b"),
        approx_size_gb: 1.6,
        tier: ModelTier::Tier1,
        description: Cow::Borrowed("Fast lightweight model with strong instruction following."),
    },
    PullableModel {
        name: Cow::Borrowed("phi4-mini"),
        approx_size_gb: 3.8,
        tier: ModelTier::Tier2,
        description: Cow::Borrowed(
            "Stronger reasoning, better for full-body classification on ambiguous emails.",
        ),
    },
];

/// Unified representation of a model for the UI, combining installation state with catalog details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub installed: bool,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
    pub catalog: Option<PullableModel>,
}

/// Evaluates whether an installed model name matches a catalog tag, accounting for `:latest` aliases.
pub(crate) fn names_match(installed_name: &str, catalog_name: &str) -> bool {
    installed_name == catalog_name
        || installed_name == format!("{catalog_name}:latest")
        || catalog_name == format!("{installed_name}:latest")
}

/// Builds a consolidated model list combining local tags with available catalog entries.
///
/// # Errors
/// Returns a [`BoxError`] if querying Ollama's `/api/tags` endpoint fails.
pub async fn list_models_view(http_client: &reqwest::Client) -> Result<Vec<ModelEntry>, BoxError> {
    let installed = list_available_models(http_client).await?;

    let mut entries: Vec<ModelEntry> = installed
        .iter()
        .map(|model| ModelEntry {
            name: model.name.clone(),
            installed: true,
            size: Some(model.size),
            modified_at: Some(model.modified_at.clone()),
            catalog: PULLABLE_MODELS
                .iter()
                .find(|pullable| names_match(&model.name, &pullable.name))
                .cloned(),
        })
        .collect();

    for pullable in PULLABLE_MODELS {
        let already_listed = installed
            .iter()
            .any(|model| names_match(&model.name, &pullable.name));

        if !already_listed {
            entries.push(ModelEntry {
                name: pullable.name.to_string(),
                installed: false,
                size: None,
                modified_at: None,
                catalog: Some(pullable.clone()),
            });
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_names_match_exact_and_latest_suffix() {
        assert!(names_match("llama3.2:1b", "llama3.2:1b"));
        assert!(names_match("llama3.2:1b", "llama3.2:1b:latest"));
        assert!(names_match("llama3.2:1b:latest", "llama3.2:1b"));
        assert!(!names_match("llama3.2:1b", "llama3.2:3b"));
        assert!(!names_match("llama3.2", "phi4-mini"));
    }

    #[test]
    fn test_pullable_catalog_entries_valid() {
        for model in PULLABLE_MODELS {
            assert!(!model.name.is_empty());
            assert!(model.approx_size_gb > 0.0);
            assert!(!model.description.is_empty());
        }
    }
}
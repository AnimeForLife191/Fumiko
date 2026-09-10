use super::OLLAMA_BASE_URL;
use common::BoxError;
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Summary metadata for a model currently installed in the local Ollama instance.
#[derive(Debug, Deserialize, Clone, Serialize, PartialEq, Eq)]
pub struct OllamaModel {
    pub name: String,
    pub size: u64,
    pub modified_at: String,
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

/// Queries the local Ollama daemon for all currently downloaded models (`GET /api/tags`).
pub async fn list_available_models(
    http_client: &ReqwestClient,
) -> Result<Vec<OllamaModel>, BoxError> {
    let response: OllamaTagsResponse = http_client
        .get(format!("{OLLAMA_BASE_URL}/api/tags"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(response.models)
}

/// Hardware resource category for a recommended language model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelTier {
    /// Lightweight models (1B–3B parameters) optimized for speed and low RAM footprint.
    Tier1,
    /// Higher-capacity reasoning models (3B+ parameters) optimized for deeper inspection.
    Tier2,
}

/// Entry in the curated catalog of verified models recommended for Fumiko.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullableModel {
    pub name: Cow<'static, str>,
    pub approx_size_gb: f32,
    pub tier: ModelTier,
    pub description: Cow<'static, str>,
}

/// Curated catalog of recommended models categorized by resource tier.
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

/// Unified view of a model combining local installation state with catalog metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub name: String,
    pub installed: bool,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
    pub catalog: Option<PullableModel>,
}

pub(crate) fn names_match(installed_name: &str, catalog_name: &str) -> bool {
    installed_name == catalog_name
        || installed_name == format!("{catalog_name}:latest")
        || catalog_name == format!("{installed_name}:latest")
}

/// Returns a unified list of all installed models merged with uninstalled catalog recommendations.
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
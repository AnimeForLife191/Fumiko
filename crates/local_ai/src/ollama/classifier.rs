//! Email classification client targeting an external Ollama daemon.

use common::BoxError;
use common::config::{AI_MAX_CONTEXT_TOKENS, AI_MAX_OUTPUT_TOKENS};
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};

use super::DEFAULT_OLLAMA_URL;
use crate::prompt::{build_prompt, classification_json_schema, clean_json_response};
use crate::{Classification, Criterion, EmailClassifier};

#[derive(Debug, Serialize)]
struct OllamaOptions {
    num_ctx: u32,
    num_predict: u32,
    temperature: f32,
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    format: serde_json::Value,
    stream: bool,
    keep_alive: &'a str,
    options: OllamaOptions,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

#[derive(Debug, Deserialize, Default)]
struct ModelClassificationOutput {
    #[serde(default)]
    matched: bool,
    #[serde(default)]
    criterion_label: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
}

/// Evaluates emails using an external Ollama daemon via `/api/generate`.
pub struct OllamaClassifier {
    http_client: ReqwestClient,
    base_url: String,
    model: String,
}

impl OllamaClassifier {
    /// Creates an `OllamaClassifier` for the specified model tag on `127.0.0.1:11434`.
    pub fn new(http_client: ReqwestClient, model: impl Into<String>) -> Self {
        Self::with_base_url(http_client, DEFAULT_OLLAMA_URL, model)
    }

    /// Creates an `OllamaClassifier` targeting a custom base URL.
    pub fn with_base_url(
        http_client: ReqwestClient,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            http_client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
        }
    }

    /// Sends a prompt and GBNF JSON schema to Ollama, validating the output against known criteria.
    async fn run_classification(
        &self,
        prompt: &str,
        schema: serde_json::Value,
        criteria: &[Criterion],
    ) -> Result<Option<Classification>, BoxError> {
        let request = OllamaGenerateRequest {
            model: &self.model,
            prompt,
            format: schema,
            stream: false,
            keep_alive: "1m",
            options: OllamaOptions {
                num_ctx: AI_MAX_CONTEXT_TOKENS,
                num_predict: AI_MAX_OUTPUT_TOKENS,
                temperature: 0.1,
            },
        };

        let response: OllamaGenerateResponse = self
            .http_client
            .post(format!("{}/api/generate", self.base_url))
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let cleaned = clean_json_response(&response.response);
        let parsed: ModelClassificationOutput = serde_json::from_str(cleaned).map_err(|e| {
            format!(
                "model returned invalid JSON: {e}, raw output: {}",
                response.response
            )
        })?;

        let raw_label = parsed.criterion_label.unwrap_or_default();
        if !parsed.matched || raw_label.trim().is_empty() {
            return Ok(None);
        }

        // Validate against known criteria to reject hallucinations and normalize casing
        let matched_criterion = criteria
            .iter()
            .find(|c| c.label.eq_ignore_ascii_case(raw_label.trim()));

        let Some(canonical) = matched_criterion else {
            return Ok(None);
        };

        let raw_conf = parsed.confidence.unwrap_or(1.0);
        let confidence = if raw_conf.is_nan() {
            0.0
        } else {
            raw_conf.clamp(0.0, 1.0)
        };

        Ok(Some(Classification {
            criterion_label: canonical.label.clone(),
            confidence,
        }))
    }
}

#[async_trait::async_trait]
impl EmailClassifier for OllamaClassifier {
    /// Classifies an incoming email using Tier 1 metadata and snippet previews.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if network communication with Ollama fails or the response cannot be parsed.
    async fn classify(
        &self,
        subject: &str,
        sender: &str,
        snippet: Option<&str>,
        criteria: &[Criterion],
    ) -> Result<Option<Classification>, BoxError> {
        if criteria.is_empty() {
            return Ok(None);
        }

        let prompt = build_prompt(subject, sender, snippet, None, criteria);
        let schema = classification_json_schema(criteria);
        self.run_classification(&prompt, schema, criteria).await
    }

    /// Classifies an incoming email using Tier 2 deep body inspection.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if network communication with Ollama fails or the response cannot be parsed.
    async fn classify_with_body(
        &self,
        subject: &str,
        sender: &str,
        body: &str,
        criteria: &[Criterion],
    ) -> Result<Option<Classification>, BoxError> {
        if criteria.is_empty() {
            return Ok(None);
        }

        let prompt = build_prompt(subject, sender, None, Some(body), criteria);
        let schema = classification_json_schema(criteria);
        self.run_classification(&prompt, schema, criteria).await
    }
}
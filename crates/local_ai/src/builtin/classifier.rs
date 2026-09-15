//! Email classification client targeting the bundled `llama-server` sidecar.

use common::BoxError;
use common::config::AI_MAX_OUTPUT_TOKENS;
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};

use crate::builtin::server::BUILTIN_BASE_URL;
use crate::prompt::{build_prompt, classification_json_schema, clean_json_response};
use crate::{Classification, Criterion, EmailClassifier};

#[derive(Debug, Serialize)]
struct LlamaServerCompletionRequest<'a> {
    prompt: &'a str,
    json_schema: serde_json::Value,
    n_predict: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct LlamaServerCompletionResponse {
    content: String,
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

/// Evaluates emails using the local `llama-server` sidecar via `/completion`.
pub struct LlamaServerClassifier {
    http_client: ReqwestClient,
    base_url: String,
}

impl LlamaServerClassifier {
    /// Creates a `LlamaServerClassifier` targeting the default sidecar URL (`127.0.0.1:11435`).
    pub fn new(http_client: ReqwestClient) -> Self {
        Self {
            http_client,
            base_url: BUILTIN_BASE_URL.to_string(),
        }
    }

    /// Creates a `LlamaServerClassifier` targeting a custom base URL.
    pub fn with_base_url(http_client: ReqwestClient, base_url: impl Into<String>) -> Self {
        Self {
            http_client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Dispatches a prompt with GBNF grammar constraints to `/completion` and validates the output.
    async fn run_classification(
        &self,
        prompt: &str,
        schema: serde_json::Value,
        criteria: &[Criterion],
    ) -> Result<Option<Classification>, BoxError> {
        let request = LlamaServerCompletionRequest {
            prompt,
            json_schema: schema,
            n_predict: AI_MAX_OUTPUT_TOKENS,
            temperature: 0.1,
            stream: false,
        };

        let response: LlamaServerCompletionResponse = self
            .http_client
            .post(format!("{}/completion", self.base_url))
            .json(&request)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let cleaned = clean_json_response(&response.content);
        let parsed: ModelClassificationOutput = serde_json::from_str(cleaned).map_err(|e| {
            format!(
                "llama-server returned invalid JSON: {e}, raw output: {}",
                response.content
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
impl EmailClassifier for LlamaServerClassifier {
    /// Classifies an incoming email using Tier 1 metadata and preview snippets.
    ///
    /// # Errors
    /// Returns a [`BoxError`] if network communication with `llama-server` fails or the response cannot be parsed.
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
    /// Returns a [`BoxError`] if network communication with `llama-server` fails or the response cannot be parsed.
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
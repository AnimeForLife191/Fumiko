use common::BoxError;
use reqwest::Client as ReqwestClient;
use serde::{Deserialize, Serialize};
use super::OLLAMA_BASE_URL;

/// Classification rule consisting of a unique label and natural language description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Criterion {
    pub label: String,
    pub description: String,
}

/// Output result produced when an email matches a configured criterion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub criterion_label: String,
    pub confidence: f32,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    /// Caps KV cache memory. 2048 is plenty for subject + sender + 4000 char body.
    num_ctx: u32,
    /// JSON output is only ~30-50 tokens. Prevents runaway generation.
    num_predict: u32,
    temperature: f32,
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    format: serde_json::Value,
    stream: bool,
    /// Keep loaded for 1 minute of inactivity instead of the default 5 minutes
    keep_alive: &'a str,
    options: OllamaOptions,
}
#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

// Optional fields handle small models omitting properties when matched is false.
#[derive(Debug, Deserialize, Default)]
struct ModelClassificationOutput {
    #[serde(default)]
    matched: bool,
    #[serde(default)]
    criterion_label: Option<String>,
    #[serde(default)]
    confidence: Option<f32>,
}

/// Evaluates email text against active criteria using a local Ollama model.
pub struct OllamaClassifier {
    http_client: ReqwestClient,
    model: String,
}

impl OllamaClassifier {
    /// Creates a new `OllamaClassifier` targeting the specified local model tag.
    pub fn new(http_client: ReqwestClient, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or(http_client);

        Self { 
            http_client: client, 
            model 
        }
    }

    /// Evaluates an email using Tier 1 lightweight metadata (subject, sender, and preview snippet).
    ///
    /// Incurs zero extra network payload since snippets are already fetched during mailbox sync.
    pub async fn classify(
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
        self.run_classification(&prompt).await
    }

    /// Evaluates an email using Tier 2 deep body inspection for ambiguous Tier 1 matches.
    ///
    /// The body text is safely truncated to 4,000 characters to prevent prompt context bloat.
    pub async fn classify_with_body(
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
        self.run_classification(&prompt).await
    }

    async fn run_classification(&self, prompt: &str) -> Result<Option<Classification>, BoxError> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "matched": { "type": "boolean" },
                "criterion_label": { "type": "string" },
                "confidence": { "type": "number" }
            },
            "required": ["matched", "criterion_label", "confidence"]
        });

        let request = OllamaGenerateRequest {
            model: &self.model,
            prompt,
            format: schema,
            stream: false,
            keep_alive: "1m", // Unloads model after 1 minute of inactivity
            options: OllamaOptions {
                num_ctx: 2048,     // Drastically reduces KV cache RAM
                num_predict: 128,  // Minimal token buffer
                temperature: 0.1,  // Deterministic JSON outputs
            },
        };

        let response: OllamaGenerateResponse = self
            .http_client
            .post(format!("{OLLAMA_BASE_URL}/api/generate"))
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

        let label = parsed.criterion_label.unwrap_or_default();
        if !parsed.matched || label.trim().is_empty() {
            return Ok(None);
        }

        let confidence = parsed.confidence.unwrap_or(1.0).clamp(0.0, 1.0);

        Ok(Some(Classification {
            criterion_label: label,
            confidence,
        }))
    }
}

// Strips markdown fences if the model wraps JSON despite format=json mode.
fn clean_json_response(raw: &str) -> &str {
    let trimmed = raw.trim();
    
    let without_fences = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else {
        trimmed
    };

    if let (Some(start), Some(end)) = (without_fences.find('{'), without_fences.rfind('}')) {
        if start <= end {
            return &without_fences[start..=end];
        }
    }

    without_fences
}

fn build_prompt(
    subject: &str,
    sender: &str,
    snippet: Option<&str>,
    body: Option<&str>,
    criteria: &[Criterion],
) -> String {
    let criteria_list = criteria
        .iter()
        .map(|c| format!("- \"{}\": {}", c.label, c.description))
        .collect::<Vec<_>>()
        .join("\n");

    let content_section = if let Some(b) = body {
        format!("Body:\n{}\n", truncate(b, 4000))
    } else if let Some(s) = snippet {
        if s.trim().is_empty() {
            String::new()
        } else {
            format!("Snippet Preview:\n{}\n", s.trim())
        }
    } else {
        String::new()
    };

    // Text is aligned to the left margin without leading indentation so small models are not distracted by indentation artifacts.
    format!(
"You are an email classification assistant. Given an email and a list of criteria, determine if the email matches ANY of the criteria.

Criteria:
{criteria_list}

Email:
Subject: {subject}
From: {sender}
{content_section}
Respond with ONLY a JSON object in this exact shape, no other text:
{{\"matched\": true or false, \"criterion_label\": \"the exact label of the matched criterion, or empty string if no match\", \"confidence\": a number between 0 and 1}}"
    )
}

fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}
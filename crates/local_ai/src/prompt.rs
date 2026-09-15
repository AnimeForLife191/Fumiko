//! Prompt engineering, character-safe truncation, and JSON grammar schemas.

use crate::Criterion;

/// Builds a structured JSON schema enforced during token generation.
///
/// Instructs the inference sampler to apply token-level GBNF grammar constraints.
/// By restricting the `criterion_label` property to an exact enum containing only the
/// active criteria labels, the model's sampler masks out disallowed tokens at the logits level,
/// making it physically impossible for the model to hallucinate unconfigured categories.
pub fn classification_json_schema(criteria: &[Criterion]) -> serde_json::Value {
    let mut allowed_labels = vec!["".to_string()];
    for c in criteria {
        allowed_labels.push(c.label.clone());
    }

    serde_json::json!({
        "type": "object",
        "properties": {
            "matched": { "type": "boolean" },
            "criterion_label": {
                "type": "string",
                "enum": allowed_labels
            },
            "confidence": { "type": "number" }
        },
        "required": ["matched", "criterion_label", "confidence"]
    })
}

/// Builds the classification prompt formatted without leading indentation.
///
/// Uses zero-indentation strings to prevent small models (1B to 3B parameters) from mirroring
/// leading whitespace or escaping structured output into malformed markdown blocks.
pub fn build_prompt(
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
        format!("Body:\n{}\n", truncate_chars(b, 4000))
    } else if let Some(s) = snippet {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            String::new()
        } else {
            format!("Snippet Preview:\n{trimmed}\n")
        }
    } else {
        String::new()
    };

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

/// Extracts the raw JSON substring by stripping markdown code fences and conversational preamble.
///
/// Slices text from the first opening brace `{` to the last closing brace `}`.
pub fn clean_json_response(raw: &str) -> &str {
    let trimmed = raw.trim();

    // 1. Strip markdown code fences if emitted by the model
    let without_fences = if let Some(rest) = trimmed.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else if let Some(rest) = trimmed.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else {
        trimmed
    };

    // 2. Extract substring between outer braces to discard conversational preamble
    if let (Some(start), Some(end)) = (without_fences.find('{'), without_fences.rfind('}')) {
        if start <= end {
            return &without_fences[start..=end];
        }
    }

    without_fences
}

/// Truncates a string on Unicode scalar boundaries to avoid splitting multi-byte characters.
pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_json_strips_markdown_and_preamble() {
        let raw = "Here is your classification:\n```json\n{\"matched\": true, \"criterion_label\": \"urgent\", \"confidence\": 0.95}\n```\nHope that helps!";
        assert_eq!(
            clean_json_response(raw),
            "{\"matched\": true, \"criterion_label\": \"urgent\", \"confidence\": 0.95}"
        );
    }

    #[test]
    fn test_truncate_chars_handles_multibyte() {
        let text = "文子Fumiko";
        assert_eq!(truncate_chars(text, 2), "文子");
        assert_eq!(truncate_chars(text, 10), "文子Fumiko");
    }

    #[test]
    fn test_classification_schema_contains_labels() {
        let criteria = vec![
            Criterion {
                label: "Urgent".into(),
                description: "Immediate action".into(),
            },
            Criterion {
                label: "Receipts".into(),
                description: "Invoices".into(),
            },
        ];
        let schema = classification_json_schema(&criteria);
        let enum_arr = schema["properties"]["criterion_label"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(enum_arr.len(), 3);
        assert_eq!(enum_arr[0], "");
        assert_eq!(enum_arr[1], "Urgent");
        assert_eq!(enum_arr[2], "Receipts");
    }

    #[test]
    fn test_build_prompt_formatting() {
        let criteria = vec![Criterion {
            label: "Jobs".into(),
            description: "Interviews".into(),
        }];
        let prompt = build_prompt("Subject", "me@example.com", Some("Hello"), None, &criteria);
        assert!(prompt.contains("Subject: Subject"));
        assert!(prompt.contains("From: me@example.com"));
        assert!(prompt.contains("Snippet Preview:\nHello"));
    }

    #[test]
    fn test_clean_json_nested_braces_and_dirty_preamble() {
        let raw = "I analyzed your email! Here is the output:\n{\n  \"matched\": false,\n  \"criterion_label\": \"\",\n  \"confidence\": 0.1\n}\nLet me know if you need anything else!";
        let cleaned = clean_json_response(raw);
        assert_eq!(
            cleaned,
            "{\n  \"matched\": false,\n  \"criterion_label\": \"\",\n  \"confidence\": 0.1\n}"
        );
    }

    #[test]
    fn test_clean_json_empty_and_no_braces() {
        assert_eq!(clean_json_response(""), "");
        assert_eq!(clean_json_response("no braces here"), "no braces here");
    }

    #[test]
    fn test_empty_criteria_schema_enum() {
        let schema = classification_json_schema(&[]);
        let enum_arr = schema["properties"]["criterion_label"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(enum_arr.len(), 1);
        assert_eq!(enum_arr[0], "");
    }
}
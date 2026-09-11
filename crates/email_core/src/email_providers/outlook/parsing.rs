use html2text::from_read;
use reqwest::Client as ReqwestClient;

use super::models::{AttachmentListResponse, OutlookAttachment, OutlookBody};
use crate::{AttachmentMeta, ProviderError};

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

pub fn extract_body(body: Option<OutlookBody>) -> (Option<String>, Option<String>) {
    match body {
        Some(b) if b.content_type.eq_ignore_ascii_case("html") => {
            // Derive plain text locally with html2text so both bodies are available without an extra API call.
            let derived_text = from_read(b.content.as_bytes(), 80).ok();
            (derived_text, Some(b.content))
        }
        Some(b) if b.content_type.eq_ignore_ascii_case("text") => (Some(b.content), None),
        _ => (None, None),
    }
}

pub async fn extract_attachments(
    http_client: &ReqwestClient,
    access_token: &str,
    message_id: &str,
) -> Result<Vec<AttachmentMeta>, ProviderError> {
    let url = format!(
        "{GRAPH_BASE}/me/messages/{message_id}/attachments?$select=id,name,contentType,size,isInline"
    );

    let response: AttachmentListResponse = http_client
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let attachments = response
        .value
        .into_iter()
        .filter(|a| !a.is_inline.unwrap_or(false))
        .map(|a| AttachmentMeta {
            id: a.id,
            filename: a.name,
            mime_type: a
                .content_type
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            size: a.size.map(|s| s.max(0) as u64).unwrap_or(0),
        })
        .collect();

    Ok(attachments)
}

pub async fn fetch_inline_attachments(
    http_client: &ReqwestClient,
    access_token: &str,
    message_id: &str,
) -> Result<Vec<OutlookAttachment>, ProviderError> {
    // Filter on the server side to only download bytes for inline images embedded within HTML bodies.
    let url = format!(
        "{GRAPH_BASE}/me/messages/{message_id}/attachments?$filter=isInline eq true&$select=contentType,isInline,microsoft.graph.fileAttachment/contentId,microsoft.graph.fileAttachment/contentBytes"
    );

    let response: AttachmentListResponse = http_client
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(response.value)
}

pub fn apply_inline_images(
    html: Option<String>,
    inline_attachments: &[OutlookAttachment],
) -> Option<String> {
    let mut html = html?;

    for attachment in inline_attachments {
        let (Some(content_id), Some(content_bytes)) =
            (&attachment.content_id, &attachment.content_bytes)
        else {
            continue;
        };

        let content_type = attachment
            .content_type
            .as_deref()
            .unwrap_or("application/octet-stream");

        let clean_cid = content_id.trim_start_matches('<').trim_end_matches('>');
        let data_uri = format!("data:{content_type};base64,{content_bytes}");
        
        if let Ok(re) = regex::Regex::new(&format!(r"(?i)cid:{}", regex::escape(clean_cid))) {
            html = re.replace_all(&html, &data_uri).into_owned();
        }
    }

    Some(html)
}

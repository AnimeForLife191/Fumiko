use base64::engine::general_purpose::{STANDARD, URL_SAFE};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use common::ProviderError;
use encoding_rs::Encoding;
use reqwest::Client as ReqwestClient;

use super::AttachmentMeta;
use super::models::{AttachmentDataResponse, MessageHeader, MessagePartFull};

pub fn header_value<'a>(headers: &'a [MessageHeader], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

pub fn charset_for_part(part: &MessagePartFull) -> &'static Encoding {
    part.headers
        .as_ref()
        .and_then(|headers| header_value(headers, "Content-Type"))
        .and_then(|content_type| {
            content_type
                .split(';')
                .find_map(|segment| segment.trim().strip_prefix("charset="))
                .map(|c| c.trim_matches('"'))
        })
        .and_then(|label| Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8)
}

fn decode_body(data: &str, encoding: &'static Encoding) -> Result<String, ProviderError> {
    let clean_data = data.trim();
    // Protocol quirk: Gmail documents URL-safe unpadded Base64, but occasionally sends padded data.
    let bytes = match URL_SAFE_NO_PAD.decode(clean_data) {
        Ok(b) => b,
        Err(_) => match URL_SAFE.decode(clean_data) {
            Ok(b) => b,
            Err(e) => {
                return Err(ProviderError::InvalidData(format!(
                    "invalid Base64 body: {e}"
                )));
            }
        },
    };

    let (text, _, _had_errors) = encoding.decode(&bytes);
    Ok(text.into_owned())
}

pub fn find_part<'a>(part: &'a MessagePartFull, target_mime: &str) -> Option<&'a MessagePartFull> {
    if part.mime_type == target_mime {
        let has_content = part
            .body
            .as_ref()
            .map(|b| b.data.is_some() || b.attachment_id.is_some())
            .unwrap_or(false);
        if has_content {
            return Some(part);
        }
    }
    part.parts
        .as_ref()?
        .iter()
        .find_map(|p| find_part(p, target_mime))
}

fn content_id(part: &MessagePartFull) -> Option<&str> {
    part.headers
        .as_ref()
        .and_then(|headers| header_value(headers, "Content-ID"))
        .map(|v| v.trim_start_matches('<').trim_end_matches('>'))
}

pub fn collect_attachments(
    part: &MessagePartFull,
    out: &mut Vec<AttachmentMeta>,
    inline_out: &mut Vec<(String, String, String)>,
) {
    if let (Some(filename), Some(body)) = (&part.filename, &part.body) {
        if !filename.is_empty() {
            if let Some(attachment_id) = &body.attachment_id {
                // If a Content-ID header exists, this is an inline image referenced by cid: in the HTML.
                match content_id(part) {
                    Some(cid) => inline_out.push((
                        cid.to_string(),
                        part.mime_type.clone(),
                        attachment_id.clone(),
                    )),
                    None => out.push(AttachmentMeta {
                        id: attachment_id.clone(),
                        filename: filename.clone(),
                        mime_type: part.mime_type.clone(),
                        size: body.size.unwrap_or(0) as u64,
                    }),
                }
            }
        }
    }
    if let Some(children) = &part.parts {
        for child in children {
            collect_attachments(child, out, inline_out);
        }
    }
}

pub async fn apply_inline_images(
    http_client: &ReqwestClient,
    access_token: &str,
    message_id: &str,
    html: Option<String>,
    inline_parts: &[(String, String, String)],
) -> Result<Option<String>, ProviderError> {
    let Some(mut html) = html else {
        return Ok(None);
    };

    for (content_id, mime_type, attachment_id) in inline_parts {
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}/attachments/{attachment_id}"
        );
        let attachment = http_client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?
            .json::<AttachmentDataResponse>()
            .await?;

        let clean_data = attachment.data.trim();
        let bytes = match URL_SAFE_NO_PAD.decode(clean_data) {
            Ok(b) => b,
            Err(_) => URL_SAFE.decode(clean_data).map_err(|e| {
                ProviderError::InvalidData(format!("invalid Base64 attachment: {e}"))
            })?,
        };
        let standard_b64 = STANDARD.encode(&bytes);

        let data_url = format!("data:{mime_type};base64,{standard_b64}");
        html = html.replace(&format!("cid:{content_id}"), &data_url);
    }

    Ok(Some(html))
}

pub async fn extract_part_content(
    http_client: &reqwest::Client,
    access_token: &str,
    message_id: &str,
    part: &MessagePartFull,
) -> Result<Option<String>, ProviderError> {
    let Some(body) = part.body.as_ref() else {
        return Ok(None);
    };
    let encoding = charset_for_part(part);

    if let Some(data) = &body.data {
        return decode_body(data, encoding).map(Some);
    }

    if let Some(attachment_id) = &body.attachment_id {
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}/attachments/{attachment_id}"
        );

        let attachment = http_client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?
            .json::<AttachmentDataResponse>()
            .await?;

        return decode_body(&attachment.data, encoding).map(Some);
    }
    Ok(None)
}

mod models;
mod parsing;

use std::time::Duration;

use futures::stream::{self, StreamExt};
use models::{DeltaResponse, OutlookMessageResponse, OutlookProfileResponse};
use parsing::{apply_inline_images, extract_attachments, extract_body, fetch_inline_attachments};
use reqwest::Client as ReqwestClient;

use crate::{
    EmailProvider, FullMessage, MessageMetadata, Profile, ProviderError, SyncCursor, SyncOptions,
    SyncPage,
};

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const MAX_INITIAL_SYNC_PAGES: usize = 200;
const MAX_429_RETRIES: usize = 3;

/// Microsoft Graph API client implementing the [`EmailProvider`] trait.
pub struct OutlookProvider {
    http_client: ReqwestClient,
}

impl OutlookProvider {
    /// Creates a new `OutlookProvider` using the provided HTTP client.
    pub fn new(http_client: ReqwestClient) -> Self {
        Self { http_client }
    }

    async fn send_request_with_retry(
        &self,
        url: &str,
        access_token: &str
    ) -> Result<reqwest::Response, ProviderError> {
        let mut attempts = 0;

        loop {
            let response = self
                .http_client
                .get(url)
                .bearer_auth(access_token)
                .send()
                .await?;

            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempts < MAX_429_RETRIES {
                attempts += 1;

                let retry_after_secs = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|hv| hv.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(2_u64.pow(attempts as u32));

                let delay = Duration::from_secs(retry_after_secs.min(30));
                tracing::warn!(
                    "Graph API returned 429 Too Many Requests. Retrying in {delay:?} (attempt {attempts}/{MAX_429_RETRIES})"
                );

                tokio::time::sleep(delay).await;
                continue;
            }

            return Ok(response);
        }
    }

    async fn fetch_profile(
        &self,
        access_token: &str,
    ) -> Result<OutlookProfileResponse, ProviderError> {
        let profile: OutlookProfileResponse = self
            .http_client
            .get(format!("{GRAPH_BASE}/me"))
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(profile)
    }

    async fn fetch_one_summary(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<MessageMetadata, ProviderError> {
        let url = format!(
            "{GRAPH_BASE}/me/messages/{message_id}?$select=subject,from,receivedDateTime,isRead,bodyPreview"
        );

        let message: OutlookMessageResponse = self
            .http_client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let subject = message
            .subject
            .unwrap_or_else(|| "(no subject)".to_string());

        let from = message
            .from
            .and_then(|f| f.email_address)
            .map(|addr| addr.address)
            .unwrap_or_else(|| "(no sender)".to_string());

        let received_at = message
            .received_date_time
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
            .map(|dt| dt.timestamp())
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        Ok(MessageMetadata {
            id: message_id.to_string(),
            subject,
            from,
            received_at,
            is_read: message.is_read.unwrap_or(false),
            snippet: message.body_preview,
        })
    }

    async fn fetch_full_message_inner(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<FullMessage, ProviderError> {
        let url = format!("{GRAPH_BASE}/me/messages/{message_id}?$select=body,uniqueBody");

        let message: OutlookMessageResponse = self
            .http_client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let (body_text, body_html) = extract_body(message.body);
        let (_, unique_body_html) = extract_body(message.unique_body);

        let inline_attachments =
            fetch_inline_attachments(&self.http_client, access_token, message_id).await?;

        let body_html = apply_inline_images(body_html, &inline_attachments);
        let unique_body_html = apply_inline_images(unique_body_html, &inline_attachments);

        let attachments = extract_attachments(&self.http_client, access_token, message_id).await?;

        Ok(FullMessage {
            id: message_id.to_string(),
            body_text,
            body_html,
            unique_body_html,
            attachments,
        })
    }
}

#[async_trait::async_trait]
impl EmailProvider for OutlookProvider {
    async fn get_profile(&self, access_token: &str) -> Result<Profile, ProviderError> {
        let profile = self.fetch_profile(access_token).await?;
        Ok(Profile {
            email_address: profile
                .mail
                .or(profile.user_principal_name)
                .unwrap_or_default(),
            display_name: profile.display_name,
        })
    }

    async fn initial_sync(
        &self,
        access_token: &str,
        options: SyncOptions,
    ) -> Result<SyncPage, ProviderError> {
        let mut next_url = Some(format!(
            "{GRAPH_BASE}/me/mailFolders/inbox/messages/delta?$top={}",
            options.max_results
        ));
        let mut all_message_ids = Vec::new();
        let mut final_delta_link = None;
        let mut page_count = 0;

        // Protocol requirement: Microsoft Graph delta queries do NOT return @odata.deltaLink
        // on page 1 if more results exist. We must follow @odata.nextLink until the final page.
        while let Some(url) = next_url {
            page_count += 1;
            if page_count > MAX_INITIAL_SYNC_PAGES {
                return Err(ProviderError::InvalidData(format!(
                    "Outlook initial sync exceeded maximum page limit of {MAX_INITIAL_SYNC_PAGES}"
                )));
            }

            let response: DeltaResponse = self
                .send_request_with_retry(&url, access_token)
                .await?
                .error_for_status()?
                .json()
                .await?;

            for item in response.value {
                if all_message_ids.len() < options.max_results as usize {
                    all_message_ids.push(item.id);
                }
            }

            if let Some(delta) = response.delta_link {
                final_delta_link = Some(delta);
                break;
            }

            next_url = response.next_link;
        }

        let next_cursor = final_delta_link.ok_or_else(|| {
            ProviderError::InvalidData(
                "Outlook initial sync ended without returning a delta cursor".to_string(),
            )
        })?;

        Ok(SyncPage {
            new_message_ids: all_message_ids,
            trashed_message_ids: Vec::new(),
            deleted_message_ids: Vec::new(),
            restored_message_ids: Vec::new(),
            next_cursor: SyncCursor(next_cursor),
            has_more: false,
        })
    }

    async fn incremental_sync(
        &self,
        access_token: &str,
        cursor: &SyncCursor,
    ) -> Result<SyncPage, ProviderError> {
        let raw_response = self
            .http_client
            .get(&cursor.0)
            .bearer_auth(access_token)
            .send()
            .await?;

        // Microsoft Graph returns HTTP 410 Gone when the delta token is no longer retained.
        if raw_response.status() == reqwest::StatusCode::GONE {
            return Err(ProviderError::CursorExpired);
        }

        let response: DeltaResponse = raw_response.error_for_status()?.json().await?;

        let mut new_message_ids = Vec::new();
        let mut trashed_message_ids = Vec::new();

        for item in response.value {
            if item.removed.is_some() {
                trashed_message_ids.push(item.id);
            } else {
                new_message_ids.push(item.id);
            }
        }

        let next_cursor = response.delta_link.or(response.next_link).ok_or_else(|| {
            ProviderError::InvalidData(
                "Outlook incremental sync response contained neither a nextLink nor a deltaLink"
                    .to_string(),
            )
        })?;

        Ok(SyncPage {
            new_message_ids,
            trashed_message_ids,
            deleted_message_ids: Vec::new(),
            restored_message_ids: Vec::new(),
            next_cursor: SyncCursor(next_cursor),
            has_more: false,
        })
    }

    async fn fetch_message_metadata(
        &self,
        access_token: &str,
        message_ids: &[String],
    ) -> Result<Vec<Result<MessageMetadata, ProviderError>>, ProviderError> {
        let results = stream::iter(message_ids.iter().cloned())
            .map(|id| async move { self.fetch_one_summary(access_token, &id).await })
            .buffer_unordered(10)
            .collect()
            .await;

        Ok(results)
    }

    async fn fetch_full_message(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<FullMessage, ProviderError> {
        self.fetch_full_message_inner(access_token, message_id)
            .await
    }
}
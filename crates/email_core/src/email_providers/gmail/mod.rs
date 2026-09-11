pub mod models;
pub mod parsing;

use std::collections::HashSet;

use futures::stream::{self, StreamExt};
use reqwest::Client as ReqwestClient;

use common::ProviderError;
use models::{
    GmailFullMessageResponse, GmailMessageSummaryResponse, GmailProfileResponse,
    HistoryListResponse, ListMessagesResponse,
};
use parsing::{collect_attachments, extract_part_content, find_part, header_value};

use crate::{
    AttachmentMeta, EmailProvider, FullMessage, MessageMetadata, Profile, SyncCursor, SyncOptions,
    SyncPage, email_providers::gmail::parsing::apply_inline_images,
};

/// Google Gmail API client implementing the [`EmailProvider`] trait.
pub struct GmailProvider {
    http_client: ReqwestClient,
}

impl GmailProvider {
    /// Creates a new `GmailProvider` using the provided HTTP client.
    pub fn new(http_client: ReqwestClient) -> Self {
        Self { http_client }
    }

    async fn fetch_gmail_profile(
        &self,
        access_token: &str,
    ) -> Result<GmailProfileResponse, ProviderError> {
        let profile: GmailProfileResponse = self
            .http_client
            .get("https://gmail.googleapis.com/gmail/v1/users/me/profile")
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
        // Querying format=metadata avoids downloading the entire MIME payload just to render inbox lists.
        let detail_url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}?format=metadata&metadataHeaders=Subject&metadataHeaders=From"
        );

        let metadata: GmailMessageSummaryResponse = self
            .http_client
            .get(&detail_url)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let headers = metadata.payload.map(|p| p.headers).unwrap_or_default();
        let subject = header_value(&headers, "Subject")
            .unwrap_or("(no subject)")
            .to_string();
        let from = header_value(&headers, "From")
            .unwrap_or("(no sender)")
            .to_string();
        let received_at = metadata
            .internal_date
            .parse::<i64>()
            .map(|ms| ms / 1000)
            .unwrap_or_else(|_| chrono::Utc::now().timestamp());
        let is_read = !metadata
            .label_ids
            .unwrap_or_default()
            .iter()
            .any(|label| label == "UNREAD");

        Ok(MessageMetadata {
            id: message_id.to_string(),
            subject,
            from,
            received_at,
            is_read,
            snippet: metadata.snippet,
        })
    }

    async fn fetch_full_message_inner(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<FullMessage, ProviderError> {
        let detail_url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{message_id}?format=full"
        );

        let message: GmailFullMessageResponse = self
            .http_client
            .get(&detail_url)
            .bearer_auth(access_token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let Some(payload) = message.payload else {
            return Ok(FullMessage {
                id: message_id.to_string(),
                body_text: None,
                body_html: None,
                attachments: vec![],
                unique_body_html: None,
            });
        };

        let body_html = match find_part(&payload, "text/html") {
            Some(part) => {
                extract_part_content(&self.http_client, access_token, message_id, part).await?
            }
            None => None,
        };

        // MIME tree extraction: text/plain and text/html nodes can reside at any depth in multipart trees.
        let body_text = match find_part(&payload, "text/plain") {
            Some(part) => {
                extract_part_content(&self.http_client, access_token, message_id, part).await?
            }
            None => body_html
                .as_ref()
                .and_then(|html| html2text::from_read(html.as_bytes(), 80)
                .ok())
        };

        let mut attachments = Vec::new();
        let mut inline_parts = Vec::new();
        collect_attachments(&payload, &mut attachments, &mut inline_parts);

        let body_html = apply_inline_images(
            &self.http_client,
            access_token,
            message_id,
            body_html,
            &inline_parts,
        )
        .await?;

        Ok(FullMessage {
            id: message_id.to_string(),
            body_text,
            body_html,
            attachments,
            unique_body_html: None,
        })
    }
}

#[async_trait::async_trait]
impl EmailProvider for GmailProvider {
    async fn get_profile(&self, access_token: &str) -> Result<Profile, ProviderError> {
        let gmail_profile = self.fetch_gmail_profile(access_token).await?;
        Ok(Profile {
            email_address: gmail_profile.email_address,
            display_name: None,
        })
    }

    async fn initial_sync(
        &self,
        access_token: &str,
        options: SyncOptions,
    ) -> Result<SyncPage, ProviderError> {
        let mut all_message_ids: Vec<String> = Vec::new();
        let mut next_page_token = None;
        let max_results = options.max_results;

        let page_size = max_results.min(500);
        let mut total_fetched = 0;

        loop {
            let mut url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages?maxResults={}&labelIds=INBOX",
                page_size
            );

            if let Some(token) = &next_page_token {
                url.push_str(&format!("&pageToken={}", token));
            }

            let response: ListMessagesResponse = self
                .http_client
                .get(&url)
                .bearer_auth(access_token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            if let Some(messages) = response.messages {
                let count = messages.len();
                all_message_ids.extend(messages.into_iter().map(|m| m.id));
                total_fetched += count;
            }

            match response.next_page_token {
                Some(token) if total_fetched < max_results as usize => {
                    next_page_token = Some(token);
                }
                _ => break,
            }
        }

        if all_message_ids.len() > max_results as usize {
            all_message_ids.truncate(max_results as usize);
        }

        // Establish the initial sync baseline cursor via the user profile's current historyId.
        let fresh_history_id = self.fetch_gmail_profile(access_token).await?.history_id;

        tracing::info!(
            "Initial sync fetched {} messages (limit: {})",
            all_message_ids.len(),
            max_results
        );

        Ok(SyncPage {
            new_message_ids: all_message_ids,
            trashed_message_ids: Vec::new(),
            deleted_message_ids: Vec::new(),
            restored_message_ids: Vec::new(),
            next_cursor: SyncCursor(fresh_history_id),
            has_more: false,
        })
    }

    async fn incremental_sync(
        &self,
        access_token: &str,
        cursor: &SyncCursor,
    ) -> Result<SyncPage, ProviderError> {
        let mut next_page_token: Option<String> = None;
        let mut final_history_id = cursor.0.clone();

        let mut new_message_ids: Vec<String> = Vec::new();
        let mut deleted_message_ids: HashSet<String> = HashSet::new();
        let mut trashed_message_ids: Vec<String> = Vec::new();
        let mut restored_message_ids: Vec<String> = Vec::new();

        loop {
            let mut url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/history?startHistoryId={}&historyTypes=messageAdded&historyTypes=messageDeleted&historyTypes=labelAdded&historyTypes=labelRemoved",
                cursor.0
            );

            if let Some(token) = &next_page_token {
                url.push_str(&format!("&pageToken={}", token));
            }

            let raw_response = self
                .http_client
                .get(&url)
                .bearer_auth(access_token)
                .send()
                .await?;

            // Gmail returns HTTP 404 when the startHistoryId has expired from their history logs.
            if raw_response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(ProviderError::CursorExpired);
            }

            let response: HistoryListResponse = raw_response.error_for_status()?.json().await?;

            if !response.history_id.is_empty() {
                final_history_id = response.history_id.clone();
            }

            if let Some(history_records) = response.history {
                for record in history_records {
                    if let Some(messages_added) = record.messages_added {
                        for added in messages_added {
                            new_message_ids.push(added.message.id);
                        }
                    }

                    if let Some(messages_deleted) = record.messages_deleted {
                        for deleted in messages_deleted {
                            deleted_message_ids.insert(deleted.message.id);
                        }
                    }

                    // Gmail signals move-to-trash/spam via label additions rather than hard deletions.
                    if let Some(labels_added) = record.labels_added {
                        for label_added in labels_added {
                            let is_trash = label_added.label_ids.iter().any(|l| l == "TRASH");
                            let is_spam = label_added.label_ids.iter().any(|l| l == "SPAM");

                            if is_trash || is_spam {
                                trashed_message_ids.push(label_added.message.id);
                            }
                        }
                    }

                    if let Some(labels_removed) = record.labels_removed {
                        for label_removed in labels_removed {
                            if label_removed.label_ids.iter().any(|l| l == "TRASH" || l == "SPAM") {
                                restored_message_ids.push(label_removed.message.id);
                            }
                        }
                    }
                }
            }

            match response.next_page_token {
                Some(token) if !token.is_empty() => next_page_token = Some(token),
                _ => break,
            }
        }

        // Deduplicate IDs: multiple history records within the same range can reference the same message.
        new_message_ids.sort_unstable();
        new_message_ids.dedup();

        trashed_message_ids.sort_unstable();
        trashed_message_ids.dedup();

        restored_message_ids.sort_unstable();
        restored_message_ids.dedup();

        let final_new_ids: Vec<String> = new_message_ids
            .into_iter()
            .filter(|id| !deleted_message_ids.contains(id))
            .collect();

        let final_deleted_ids: Vec<String> = deleted_message_ids.into_iter().collect();

        Ok(SyncPage {
            new_message_ids: final_new_ids,
            trashed_message_ids,
            deleted_message_ids: final_deleted_ids,
            restored_message_ids,
            next_cursor: SyncCursor(final_history_id),
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
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, atomic::Ordering};
use std::time::{Duration, Instant};

use common::{Provider, ProviderError, SyncError, TokenError, setting_keys};
use futures::stream::{self, StreamExt};
use local_ai::{Criterion, OllamaClassifier};
use oauth::{OAuthProvider, get_access_token_for_account};
use reqwest::Client as ReqwestClient;
use storage::{Storage, models::LinkedAccount};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::EmailProvider;
use super::email_providers::{GmailProvider, OutlookProvider};

const MIN_CONFIDENCE_TO_ACCEPT: f32 = 0.5;
// Ambiguity window: Tier 1 confidence must be in [0.25, 0.49] to qualify for a Tier 2 body inspection.
const TIER1_AMBIGUITY_THRESHOLD: f32 = 0.25;
// Maximum number of full-body downloads permitted in a single sync pass to protect API quotas.
const MAX_TIER2_FETCHES_PER_SYNC: usize = 3;

pub const DEFAULT_TRASH_RETENTION_DAYS: i64 = 30;
// OAuth access tokens typically expire in 60 minutes; cache for 50 minutes to refresh proactively.
const ACCESS_TOKEN_TTL: Duration = Duration::from_secs(50 * 60);

/// Opaque synchronization cursor encapsulating a Gmail history ID or Graph delta link URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCursor(pub String);

/// Set of message ID changes discovered during a synchronization cycle.
#[derive(Debug, Clone)]
pub struct SyncPage {
    pub new_message_ids: Vec<String>,
    pub trashed_message_ids: Vec<String>,
    pub deleted_message_ids: Vec<String>,
    pub restored_message_ids: Vec<String>,
    pub next_cursor: SyncCursor,
    pub has_more: bool,
}

/// Configuration options for initial inbox synchronization passes.
#[derive(Debug, Clone, Copy)]
pub struct SyncOptions {
    pub max_results: u32,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self { max_results: 100 }
    }
}

/// Coordinates mailbox synchronization, token lifecycle caching, and background AI classification.
pub struct SyncService {
    pub storage: Storage,
    pub http_client: ReqwestClient,
    token_cache: Mutex<HashMap<Uuid, (String, Instant)>>,
}

impl SyncService {
    /// Creates a new `SyncService` instance with an empty access token cache.
    pub fn new(storage: Storage, http_client: ReqwestClient) -> Self {
        Self {
            storage,
            http_client,
            token_cache: Mutex::new(HashMap::new()),
        }
    }

    fn invalidate_token(&self, account_id: Uuid) {
        self.token_cache.lock().unwrap().remove(&account_id);
    }

    async fn access_token_for(&self, account: &LinkedAccount) -> Result<String, SyncError> {
        // Fast path: reuse valid cached access token without acquiring storage locks or network roundtrips.
        {
            let cache = self.token_cache.lock().unwrap();
            if let Some((token, expires_at)) = cache.get(&account.id) {
                if Instant::now() < *expires_at {
                    return Ok(token.clone());
                }
            }
        }

        // Slow path: refresh token against identity provider.
        let tokens =
            get_access_token_for_account(&self.storage, account, &self.http_client).await?;

        self.token_cache.lock().unwrap().insert(
            account.id,
            (
                tokens.access_token.clone(),
                Instant::now() + ACCESS_TOKEN_TTL,
            ),
        );

        Ok(tokens.access_token)
    }

    fn provider_for(&self, account: &LinkedAccount) -> Result<Box<dyn EmailProvider>, SyncError> {
        let provider = Provider::from_str(account.provider.as_str())
            .map_err(TokenError::UnsupportedProvider)?;

        let provider_kind = match provider {
            Provider::Gmail => OAuthProvider::Google,
            Provider::Outlook => OAuthProvider::Microsoft,
        };

        Ok(match provider_kind {
            OAuthProvider::Google => Box::new(GmailProvider::new(self.http_client.clone())),
            OAuthProvider::Microsoft => Box::new(OutlookProvider::new(self.http_client.clone())),
        })
    }

    async fn fetch_sync_page(
        &self,
        provider: &dyn EmailProvider,
        account: &LinkedAccount,
        access_token: &str,
    ) -> Result<SyncPage, ProviderError> {
        match &account.sync_cursor {
            None => {
                provider
                    .initial_sync(access_token, SyncOptions::default())
                    .await
            }
            Some(cursor) => {
                match provider
                    .incremental_sync(access_token, &SyncCursor(cursor.clone()))
                    .await
                {
                    Ok(page) => Ok(page),
                    // CursorExpired occurs when mailbox histories are pruned on the server.
                    // Fall back to a full initial sync to establish a fresh baseline.
                    Err(ProviderError::CursorExpired) => {
                        provider
                            .initial_sync(access_token, SyncOptions::default())
                            .await
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    async fn sync_one_account(&self, account: &LinkedAccount) -> Result<(), SyncError> {
        let result = self.sync_one_account_inner(account).await;

        if let Err(e) = &result {
            if let Err(record_err) = self
                .storage
                .set_sync_error(account.id, &e.to_string())
                .await
            {
                tracing::error!(
                    "Failed to record sync error for account {}: {record_err:?}",
                    account.id
                );
            }
        }

        result
    }

    async fn sync_one_account_inner(&self, account: &LinkedAccount) -> Result<(), SyncError> {
        let provider = self.provider_for(account)?;
        let mut access_token = self.access_token_for(account).await?;
        let is_initial_sync = account.sync_cursor.is_none();

        // Automatic retry mechanism: on HTTP 401 Unauthorized, evict the cached token,
        // request a fresh token via refresh flow, and retry once.
        let page = match self
            .fetch_sync_page(&*provider, account, &access_token)
            .await
        {
            Ok(page) => page,
            Err(ProviderError::Unauthorized) => {
                self.invalidate_token(account.id);
                access_token = self.access_token_for(account).await?;
                self.fetch_sync_page(&*provider, account, &access_token)
                    .await?
            }
            Err(e) => return Err(e.into()),
        };

        // Stores: (email_id, provider_message_id, subject, sender, snippet)
        let mut newly_saved: Vec<(Uuid, String, String, String, Option<String>)> = Vec::new();
        let mut fetch_errors_occurred = false;

        if !page.new_message_ids.is_empty() {
            let metadata = provider
                .fetch_message_metadata(&access_token, &page.new_message_ids)
                .await?;

            for result in metadata {
                match result {
                    Ok(m) => {
                        let email_id = self
                            .storage
                            .save_email(
                                account.id,
                                &m.id,
                                Some(&m.subject),
                                Some(&m.from),
                                m.received_at,
                                m.is_read,
                                m.snippet.as_deref(),
                            )
                            .await?;
                        newly_saved.push((email_id, m.id, m.subject, m.from, m.snippet));
                    }
                    Err(e) => {
                        warn!(
                            "Failed to fetch metadata for a message during sync for account {}: {e}",
                            account.id
                        );
                        // Flag failure to prevent advancing the cursor over missing messages.
                        fetch_errors_occurred = true;
                    }
                }
            }
        }

        for message_id in &page.trashed_message_ids {
            self.storage.mark_trashed(account.id, message_id).await?;
        }

        for message_id in &page.restored_message_ids {
            self.storage.mark_untrashed(account.id, message_id).await?;
        }

        for message_id in &page.deleted_message_ids {
            self.storage
                .delete_by_provider_message_id(account.id, message_id)
                .await?;
        }

        // Invariant: The sync cursor must only advance if all discovered items were cleanly hydrated.
        // Advancing on partial failure causes missed messages to become permanently invisible.
        if !fetch_errors_occurred {
            self.storage
                .update_sync_cursor(account.id, &page.next_cursor.0)
                .await?;
        } else {
            warn!(
                "Sync cursor NOT updated for account {} due to message retrieval errors",
                account.id
            );
        }

        self.classify_new_emails(
            &*provider,
            &access_token,
            is_initial_sync,
            newly_saved,
        )
        .await;

        Ok(())
    }

    /// Synchronizes a specific linked account by ID.
    pub async fn sync_account(&self, account_id: Uuid) -> Result<(), SyncError> {
        let account = self
            .storage
            .get_account(account_id)
            .await?
            .ok_or(SyncError::AccountNotFound(account_id))?;

        self.sync_one_account(&account).await
    }

    /// Synchronizes all linked accounts sequentially and purges expired trash.
    pub async fn sync_all(&self) -> Result<(), SyncError> {
        let accounts = self.storage.list_accounts().await?;
        for account in accounts {
            if let Err(e) = self.sync_one_account(&account).await {
                warn!(
                    "sync_all: account {} failed: {e} — continuing with remaining accounts",
                    account.id
                );
            }
        }

        let retention_days = self
            .storage
            .get_setting(setting_keys::TRASH_RETENTION_DAYS)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_TRASH_RETENTION_DAYS);

        match self.storage.purge_expired_trash(retention_days).await {
            Ok(count) if count > 0 => info!("purged {count} expired trashed email(s)"),
            Ok(_) => {}
            Err(e) => warn!("failed to purge expired trash: {e}"),
        }

        Ok(())
    }

    async fn classify_new_emails(
        &self,
        provider: &dyn EmailProvider,
        access_token: &str,
        is_initial_sync: bool,
        emails: Vec<(Uuid, String, String, String, Option<String>)>
    ) {
        if emails.is_empty() {
            return;
        }

        let active_criteria = match self.storage.list_active_criteria().await {
            Ok(rows) => rows,
            Err(e) => {
                warn!("classification skipped: failed to load active criteria: {e}");
                return;
            }
        };

        if active_criteria.is_empty() {
            return;
        }

        let criteria: Vec<Criterion> = active_criteria
            .iter()
            .map(|c| Criterion {
                label: c.label.clone(),
                description: c.description.clone(),
            })
            .collect();

        let model = match self.storage.get_setting(setting_keys::ACTIVE_AI_MODEL_ID).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                debug!("classification skipped: no AI model selected");
                return;
            }
            Err(e) => {
                warn!("classification skipped: failed to read active model setting: {e}");
                return;
            }
        };

        let classifier = OllamaClassifier::new(self.http_client.clone(), model);
        let tier2_budget = Arc::new(AtomicUsize::new(MAX_TIER2_FETCHES_PER_SYNC));

        // Bounded concurrency (up to 4 inferences in parallel) prevents overwhelming the local Ollama daemon.
        stream::iter(emails)
            .for_each_concurrent(4, |(email_id, provider_message_id, subject, sender, snippet)| {
                let classifier = &classifier;
                let criteria = &criteria;
                let active_criteria = &active_criteria;
                let tier2_budget = tier2_budget.clone();

                async move {
                    // Step 1: Run fast Tier 1 classification (Subject + Sender + Snippet).
                    let tier1_result = classifier
                        .classify(&subject, &sender, snippet.as_deref(), criteria)
                        .await;

                    let classification = match tier1_result {
                        Ok(Some(c)) if c.confidence >= MIN_CONFIDENCE_TO_ACCEPT => {
                            // Clean match from Tier 1.
                            Some((c, "tier1"))
                        }
                        Ok(Some(c)) if c.confidence >= TIER1_AMBIGUITY_THRESHOLD && !is_initial_sync => {
                            // Ambiguous result: qualify for Tier 2 if within budget and not initial sync.
                            let has_budget = tier2_budget
                                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                                    if count > 0 { Some(count - 1) } else { None }
                                })
                                .is_ok();

                            if has_budget {
                                debug!(
                                    "email {email_id} ambiguous in tier 1 ({:.2}), promoting to tier 2 body inspection",
                                    c.confidence
                                );

                                match provider.fetch_full_message(access_token, &provider_message_id).await {
                                    Ok(full_msg) => {
                                        let body_text = full_msg
                                            .body_text
                                            .as_deref()
                                            .or(full_msg.body_html.as_deref());

                                        if let Some(body) = body_text {
                                            match classifier.classify_with_body(&subject, &sender, body, criteria).await {
                                                Ok(Some(t2_c)) if t2_c.confidence >= MIN_CONFIDENCE_TO_ACCEPT => {
                                                    Some((t2_c, "tier2"))
                                                }
                                                Ok(Some(t2_c)) => {
                                                    debug!(
                                                        "email {email_id} tier 2 matched \"{}\" but confidence {:.2} below threshold, discarding",
                                                        t2_c.criterion_label, t2_c.confidence
                                                    );
                                                    None
                                                }
                                                Ok(None) => None,
                                                Err(e) => {
                                                    warn!("tier 2 classification error for email {email_id}: {e}");
                                                    None
                                                }
                                            }
                                        } else {
                                            None
                                        }
                                    }
                                    Err(e) => {
                                        warn!("failed to fetch full message for tier 2 inspection on {email_id}: {e}");
                                        None
                                    }
                                }
                            } else {
                                debug!(
                                    "email {email_id} ambiguous ({:.2}) but tier 2 budget exceeded, discarding",
                                    c.confidence
                                );
                                None
                            }
                        }

                        Ok(Some(c)) => {
                            debug!(
                                "email {email_id} matched \"{}\" but confidence {:.2} below threshold, discarding",
                                c.criterion_label, c.confidence
                            );
                            None
                        }
                        Ok(None) => None,
                        Err(e) => {
                            warn!("classification failed for email {email_id}: {e}, leaving unclassified");
                            None
                        }
                    };

                    // Step 2: Validate and persist if matched.
                    if let Some((match_data, tier_label)) = classification {
                        let Some(criterion) = active_criteria
                            .iter()
                            .find(|c| c.label == match_data.criterion_label)
                        else {
                            warn!(
                                "model returned unknown criterion_label \"{}\" for email {email_id}, discarding",
                                match_data.criterion_label
                            );
                            return;
                        };

                        if let Err(e) = self
                            .storage
                            .save_classification(
                                email_id,
                                criterion.id,
                                Some(match_data.confidence),
                            )
                            .await
                        {
                            warn!("failed to save classification for email {email_id}: {e}");
                        } else {
                            info!(
                                "matched [{}] via {tier_label}: email {email_id} (confidence {:.2})",
                                match_data.criterion_label, match_data.confidence
                            );

                            let _ = notify_rust::Notification::new()
                                .appname("Fumiko")
                                .summary(&format!("New Findings: {}", match_data.criterion_label))
                                .body(&format!("{}\nFrom: {}", subject, sender))
                                .timeout(notify_rust::Timeout::Milliseconds(6000))
                                .show();
                        }
                    }
                }
            })
            .await;
    }
}
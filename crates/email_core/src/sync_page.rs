//! Mailbox synchronization lifecycle, credential caching, and two-tier AI gating.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Provider, ProviderError, SyncError, ai_backends, config, setting_keys};
use futures::stream::{self, StreamExt};
use local_ai::{Criterion, EmailClassifier, LlamaServerClassifier, OllamaClassifier};
use oauth::get_access_token_for_account;
use reqwest::Client as ReqwestClient;
use storage::{Storage, models::LinkedAccount};
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::EmailProvider;
use crate::email_providers::{GmailProvider, ImapProvider, OutlookProvider};

/// Minimum confidence required to accept an AI classification match onto the findings board.
const MIN_CONFIDENCE_TO_ACCEPT: f32 = 0.5;

/// Threshold below which an email is considered a definite non-match in Tier 1.
/// Emails scoring between 0.25 and 0.49 qualify as ambiguous and are promoted to Tier 2.
const TIER1_AMBIGUITY_THRESHOLD: f32 = 0.25;

/// Proactive TTL margin for in-memory access token caching.
const ACCESS_TOKEN_TTL: Duration = Duration::from_secs(50 * 60);

/// Opaque synchronization cursor encapsulating provider mailbox state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCursor(pub String);

/// Discovered message changes in a single synchronization pass.
#[derive(Debug, Clone)]
pub struct SyncPage {
    /// Newly discovered message IDs requiring metadata hydration.
    pub new_message_ids: Vec<String>,
    /// Messages moved to Trash on the server.
    pub trashed_message_ids: Vec<String>,
    /// Messages permanently expunged or deleted on the server.
    pub deleted_message_ids: Vec<String>,
    /// Messages un-trashed or un-spammed on the server.
    pub restored_message_ids: Vec<String>,
    /// Resume cursor for the subsequent incremental sync pass.
    pub next_cursor: SyncCursor,
    /// Flag indicating whether additional change pages remain.
    pub has_more: bool,
}

/// Options governing sync limits and batch sizes.
#[derive(Debug, Clone, Copy)]
pub struct SyncOptions {
    /// Maximum number of messages to fetch during initial discovery.
    pub max_results: u32,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self { max_results: 100 }
    }
}

/// Synchronization phase driving progress bars in the desktop UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncPhase {
    /// Hydrating lightweight headers and preview snippets.
    Hydrating,
    /// Running local AI inference against active watch criteria.
    Classifying,
}

/// Real-time progress update event emitted during synchronization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncProgress {
    /// Current item index being processed.
    pub current: usize,
    /// Total items in the current phase.
    pub total: usize,
    /// Active phase indicator.
    pub phase: SyncPhase,
}

/// Central coordinator for mailbox synchronization, token lifecycle, and AI evaluation.
pub struct SyncService {
    pub storage: Storage,
    pub http_client: ReqwestClient,
    token_cache: Mutex<HashMap<Uuid, (String, Instant)>>,
    pub progress_tx: Option<UnboundedSender<SyncProgress>>,
}

impl SyncService {
    /// Creates a new `SyncService`.
    pub fn new(storage: Storage, http_client: ReqwestClient) -> Self {
        Self {
            storage,
            http_client,
            token_cache: Mutex::new(HashMap::new()),
            progress_tx: None,
        }
    }

    /// Attaches an unbounded channel sender for streaming progress updates to the UI.
    pub fn with_progress_sender(mut self, tx: UnboundedSender<SyncProgress>) -> Self {
        self.progress_tx = Some(tx);
        self
    }

    /// Emits a progress update to the desktop UI if a progress channel is attached.
    fn report_progress(&self, current: usize, total: usize, phase: SyncPhase) {
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(SyncProgress {
                current,
                total,
                phase,
            });
        }
    }

    /// Evicts an account's access token from memory cache on HTTP 401 Unauthorized.
    fn invalidate_token(&self, account_id: Uuid) {
        let mut cache = self.token_cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.remove(&account_id);
    }

    /// Resolves an active access token or app password, checking memory cache before refreshing.
    async fn access_token_for(&self, account: &LinkedAccount) -> Result<String, SyncError> {
        {
            let cache = self.token_cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((token, expires_at)) = cache.get(&account.id) {
                if Instant::now() < *expires_at {
                    return Ok(token.clone());
                }
            }
        }

        let tokens =
            get_access_token_for_account(&self.storage, account, &self.http_client).await?;

        let mut cache = self.token_cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.insert(
            account.id,
            (
                tokens.access_token.clone(),
                Instant::now() + ACCESS_TOKEN_TTL,
            ),
        );

        Ok(tokens.access_token)
    }

    /// Instantiates the appropriate [`EmailProvider`] client for an account.
    fn provider_for(&self, account: &LinkedAccount) -> Result<Box<dyn EmailProvider>, SyncError> {
        if account.provider == Provider::Imap || account.imap_host.is_some() {
            let host = account
                .imap_host
                .clone()
                .unwrap_or_else(|| "imap.gmail.com".to_string());
            let port = account.imap_port_u16().unwrap_or(993);

            return Ok(Box::new(ImapProvider::new(
                host,
                port,
                account.email_address.clone(),
            )));
        }

        match account.provider {
            Provider::Gmail => Ok(Box::new(GmailProvider::new(self.http_client.clone()))),
            Provider::Outlook => Ok(Box::new(OutlookProvider::new(self.http_client.clone()))),
            Provider::Imap => unreachable!("Handled above"),
        }
    }

    /// Reads initial sync limits from SQLite settings, defaulting to configured limits.
    async fn initial_sync_options(&self) -> SyncOptions {
        let max_results = self
            .storage
            .get_setting(setting_keys::INITIAL_SYNC_LIMIT)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(config::DEFAULT_INITIAL_SYNC_LIMIT)
            .clamp(10, config::MAX_INITIAL_SYNC_LIMIT);

        SyncOptions { max_results }
    }

    /// Discovers mailbox changes, returning `(SyncPage, is_initial_sync)`.
    async fn fetch_sync_page(
        &self,
        provider: &dyn EmailProvider,
        account: &LinkedAccount,
        access_token: &str,
    ) -> Result<(SyncPage, bool), ProviderError> {
        match &account.sync_cursor {
            None => {
                let options = self.initial_sync_options().await;
                let page = provider.initial_sync(access_token, options).await?;
                Ok((page, true))
            }
            Some(cursor) => {
                match provider
                    .incremental_sync(access_token, &SyncCursor(cursor.clone()))
                    .await
                {
                    Ok(page) => Ok((page, false)),
                    Err(ProviderError::CursorExpired) => {
                        warn!(
                            "Cursor expired for account {}; resetting cursor and performing fresh initial sync",
                            account.id
                        );
                        let _ = self.storage.clear_sync_cursor(account.id).await;
                        let options = self.initial_sync_options().await;
                        let page = provider.initial_sync(access_token, options).await?;
                        Ok((page, true))
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    /// Synchronizes an account and records diagnostic failure messages on error.
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

    /// Internal execution loop for single account synchronization.
    async fn sync_one_account_inner(&self, account: &LinkedAccount) -> Result<(), SyncError> {
        let provider = self.provider_for(account)?;
        let mut access_token = self.access_token_for(account).await?;

        // 1. Discovery Pass with 401 Retry:
        // If the provider rejects the cached token with 401 Unauthorized, invalidate the cache,
        // re-hydrate a fresh token, and retry the discovery call once before failing.
        let (page, is_initial_sync) = match self
            .fetch_sync_page(&*provider, account, &access_token)
            .await
        {
            Ok(res) => res,
            Err(ProviderError::Unauthorized) => {
                self.invalidate_token(account.id);
                access_token = self.access_token_for(account).await?;
                self.fetch_sync_page(&*provider, account, &access_token)
                    .await?
            }
            Err(e) => return Err(e.into()),
        };

        let mut newly_saved: Vec<(Uuid, String, String, String, Option<String>)> = Vec::new();
        let mut fetch_errors_occurred = false;

        // 2. Hydration Pass: Fetch summary metadata (headers & snippets) for new message IDs
        if !page.new_message_ids.is_empty() {
            let total = page.new_message_ids.len();
            self.report_progress(0, total, SyncPhase::Hydrating);

            let metadata = provider
                .fetch_message_metadata(&access_token, &page.new_message_ids)
                .await?;

            for (idx, result) in metadata.into_iter().enumerate() {
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
                    // If a message was deleted on the server between discovery and hydration,
                    // skip it safely without setting fetch_errors_occurred so the cursor can advance.
                    Err(e) if is_not_found_error(&e) => {
                        debug!(
                            "Message missing or deleted on server, skipping without holding cursor: {e}"
                        );
                    }
                    Err(e) => {
                        warn!("Failed to fetch message metadata during sync: {e}");
                        fetch_errors_occurred = true;
                    }
                }

                self.report_progress(idx + 1, total, SyncPhase::Hydrating);
            }
        }

        // 3. Process Remote Server Deletions, Trash, and Restorations
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

        // Safe Cursor Advancement Invariant:
        // Do not update the sync cursor in SQLite if any message failed to hydrate due to network
        // or server errors. Leaving the cursor at its previous position ensures the next sync cycle
        // re-discovers the unhydrated message IDs rather than permanently skipping them.
        if !fetch_errors_occurred {
            self.storage
                .update_sync_cursor(account.id, &page.next_cursor.0)
                .await?;
        } else {
            warn!(
                "Sync cursor NOT updated for account {} due to hydration errors",
                account.id
            );
        }

        // 4. Background AI Classification Pass
        self.classify_new_emails(&*provider, &access_token, is_initial_sync, newly_saved)
            .await;

        // 5. Purging old emails from SQLite
        let max_inbox = self
            .storage
            .get_setting(setting_keys::INBOX_CAPACITY)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(config::DEFAULT_INBOX_CAPACITY as i64);

        let max_findings = self
            .storage
            .get_setting(setting_keys::FINDINGS_CAPACITY)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(config::DEFAULT_FINDINGS_CAPACITY as i64);

        let _ = self
            .storage
            .purge_old_emails(account.id, max_inbox, max_findings)
            .await;

        Ok(())
    }

    /// Synchronizes a specific account by its account UUID.
    ///
    /// # Errors
    /// Returns [`SyncError::AccountNotFound`] if the account does not exist,
    /// or [`SyncError`] if network or database operations fail.
    pub async fn sync_account(&self, account_id: Uuid) -> Result<(), SyncError> {
        let account = self
            .storage
            .get_account(account_id)
            .await?
            .ok_or(SyncError::AccountNotFound(account_id))?;

        self.sync_one_account(&account).await
    }

    /// Sequentially synchronizes all linked accounts and purges expired trash.
    ///
    /// # Errors
    /// Returns [`SyncError::Database`] if querying accounts fails.
    pub async fn sync_all(&self) -> Result<(), SyncError> {
        let accounts = self.storage.list_accounts().await?;
        for account in accounts {
            if let Err(e) = self.sync_one_account(&account).await {
                warn!("sync_all: account {} failed: {e}", account.id);
            }
        }

        let retention_days = self
            .storage
            .get_setting(setting_keys::TRASH_RETENTION_DAYS)
            .await
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(config::DEFAULT_TRASH_RETENTION_DAYS);

        match self.storage.purge_expired_trash(retention_days).await {
            Ok(count) if count > 0 => info!("purged {count} expired trashed email(s)"),
            Ok(_) => {}
            Err(e) => warn!("failed to purge expired trash: {e}"),
        }

        Ok(())
    }

    /// Evaluates newly hydrated emails using two-tier local AI classification.
    async fn classify_new_emails(
        &self,
        provider: &dyn EmailProvider,
        access_token: &str,
        is_initial_sync: bool,
        emails: Vec<(Uuid, String, String, String, Option<String>)>,
    ) {
        if emails.is_empty() {
            return;
        }

        let active_criteria = match self.storage.list_active_criteria().await {
            Ok(rows) => Arc::new(rows),
            Err(e) => {
                warn!("classification skipped: failed to load active criteria: {e}");
                return;
            }
        };

        if active_criteria.is_empty() {
            return;
        }

        let criteria: Arc<[Criterion]> = active_criteria
            .iter()
            .map(|c| Criterion {
                label: c.label.clone(),
                description: c.description.clone(),
            })
            .collect::<Vec<_>>()
            .into();

        let active_model = match self
            .storage
            .get_setting(setting_keys::ACTIVE_AI_MODEL_ID)
            .await
        {
            Ok(Some(m)) if !m.trim().is_empty() => m,
            Ok(_) => {
                debug!("classification skipped: no AI model selected");
                return;
            }
            Err(e) => {
                warn!("classification skipped: failed to read active model setting: {e}");
                return;
            }
        };

        let backend_type = self
            .storage
            .get_setting(setting_keys::AI_BACKEND)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| ai_backends::BUILTIN.to_string());

        let classifier: Arc<dyn EmailClassifier> = if backend_type == ai_backends::OLLAMA {
            Arc::new(OllamaClassifier::new(
                self.http_client.clone(),
                active_model,
            ))
        } else {
            Arc::new(LlamaServerClassifier::new(self.http_client.clone()))
        };

        // Atomic counter caps Tier 2 full-body fetches to MAX_TIER2_FETCHES_PER_SYNC (3) per pass
        let tier2_budget = Arc::new(AtomicUsize::new(config::MAX_TIER2_FETCHES_PER_SYNC));
        let total_emails = emails.len();
        let completed_count = Arc::new(AtomicUsize::new(0));

        self.report_progress(0, total_emails, SyncPhase::Classifying);

        // Process up to 4 classification tasks concurrently over HTTP
        stream::iter(emails)
            .for_each_concurrent(4, |(email_id, provider_message_id, subject, sender, snippet)| {
                let classifier = Arc::clone(&classifier);
                let criteria = Arc::clone(&criteria);
                let active_criteria = Arc::clone(&active_criteria);
                let tier2_budget = Arc::clone(&tier2_budget);
                let completed_count = Arc::clone(&completed_count);
                let progress_tx = self.progress_tx.clone();
                let storage = self.storage.clone();

                async move {
                    // Tier 1: Fast metadata and server-provided snippet preview scan
                    let tier1_result = classifier
                        .classify(&subject, &sender, snippet.as_deref(), &criteria)
                        .await;

                    let classification = match tier1_result {
                        Ok(Some(c)) if c.confidence >= MIN_CONFIDENCE_TO_ACCEPT => {
                            Some((c, "tier1"))
                        }
                        // Ambiguity Gating: If confidence is ambiguous (0.25..0.50) and this is not
                        // an initial sync pass, attempt to promote to Tier 2 deep body inspection.
                        Ok(Some(c))
                            if c.confidence >= TIER1_AMBIGUITY_THRESHOLD && !is_initial_sync =>
                        {
                            let has_budget = tier2_budget
                                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                                    if count > 0 { Some(count - 1) } else { None }
                                })
                                .is_ok();

                            if has_budget {
                                match provider.fetch_full_message(access_token, &provider_message_id).await {
                                    Ok(full_msg) => {
                                        let body_text = full_msg
                                            .body_text
                                            .as_deref()
                                            .or(full_msg.body_html.as_deref());

                                        if let Some(body) = body_text {
                                            match classifier
                                                .classify_with_body(&subject, &sender, body, &criteria)
                                                .await
                                            {
                                                Ok(Some(t2_c))
                                                    if t2_c.confidence >= MIN_CONFIDENCE_TO_ACCEPT =>
                                                {
                                                    Some((t2_c, "tier2"))
                                                }
                                                Ok(Some(_)) | Ok(None) => None,
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
                                None
                            }
                        }
                        Ok(Some(_)) | Ok(None) => None,
                        Err(e) => {
                            warn!("classification failed for email {email_id}: {e}");
                            None
                        }
                    };

                    // Hallucination Guard: Validate that the returned criterion label exists
                    // in active user criteria before persisting the match.
                    if let Some((match_data, tier_label)) = classification {
                        let Some(criterion) = active_criteria
                            .iter()
                            .find(|c| c.label == match_data.criterion_label)
                        else {
                            return;
                        };

                        if let Err(e) = storage
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
                                "matched [{}] via {tier_label}: email {email_id} ({:.2})",
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

                    let current = completed_count.fetch_add(1, Ordering::SeqCst) + 1;
                    if let Some(tx) = &progress_tx {
                        let _ = tx.send(SyncProgress {
                            current,
                            total: total_emails,
                            phase: SyncPhase::Classifying,
                        });
                    }
                }
            })
            .await;
    }
}

/// Identifies whether a provider error represents an HTTP 404 or missing server message.
fn is_not_found_error(err: &ProviderError) -> bool {
    let msg = err.to_string();
    msg.contains("404") || msg.contains("Not Found") || msg.contains("NotFound")
}
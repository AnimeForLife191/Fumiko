//! RFC 3501 IMAP provider implementation using TLS and standard UID conventions.

use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use common::ProviderError;
use imap::Session;
use mail_parser::{Message, MessageParser, MimeHeaders};
use native_tls::TlsConnector;

use crate::{
    AttachmentMeta, EmailProvider, FullMessage, MessageMetadata, Profile, SyncCursor, SyncOptions,
    SyncPage,
};

/// Universal IMAP client implementing [`EmailProvider`].
pub struct ImapProvider {
    host: String,
    port: u16,
    email_address: String,
}

impl ImapProvider {
    /// Creates an `ImapProvider` targeting the specified host, port, and mailbox address.
    pub fn new(host: impl Into<String>, port: u16, email_address: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            email_address: email_address.into(),
        }
    }

    /// Connects a single TLS socket with explicit 10-second connect and 15-second read/write timeouts.
    ///
    /// Setting explicit timeouts on the underlying TcpStream before wrapping with TLS
    /// prevents dropped connections or unresponsive servers from hanging worker threads.
    fn connect_session(
        &self,
        password: &str,
    ) -> Result<Session<native_tls::TlsStream<std::net::TcpStream>>, ProviderError> {
        let tls = TlsConnector::builder()
            .build()
            .map_err(|e| ProviderError::Other(Box::new(e)))?;

        let addr = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|e| ProviderError::Other(Box::new(e)))?
            .next()
            .ok_or_else(|| ProviderError::Other("Could not resolve IMAP host address".into()))?;

        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
            .map_err(|e| ProviderError::Other(Box::new(e)))?;

        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| ProviderError::Other(Box::new(e)))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| ProviderError::Other(Box::new(e)))?;

        let tls_stream = tls
            .connect(&self.host, stream)
            .map_err(|e| ProviderError::Other(Box::new(e)))?;

        let client = imap::Client::new(tls_stream);

        let session = client
            .login(&self.email_address, password)
            .map_err(|(e, _)| {
                let err_str = e.to_string().to_lowercase();
                if err_str.contains("authenticationfailed")
                    || err_str.contains("invalid credentials")
                    || err_str.contains("no [auth")
                    || err_str.contains("denied")
                {
                    ProviderError::Unauthorized
                } else {
                    ProviderError::Other(Box::new(e))
                }
            })?;

        Ok(session)
    }
}

#[async_trait::async_trait]
impl EmailProvider for ImapProvider {
    async fn get_profile(&self, access_token: &str) -> Result<Profile, ProviderError> {
        let email = self.email_address.clone();
        let password = access_token.to_string();
        let provider = ImapProvider {
            host: self.host.clone(),
            port: self.port,
            email_address: self.email_address.clone(),
        };

        tokio::task::spawn_blocking(move || {
            let mut session = provider.connect_session(&password)?;
            let _ = session.logout();
            Ok(Profile {
                email_address: email,
                display_name: None,
            })
        })
        .await
        .map_err(|e| ProviderError::Other(Box::new(e)))?
    }

    async fn initial_sync(
        &self,
        access_token: &str,
        options: SyncOptions,
    ) -> Result<SyncPage, ProviderError> {
        let password = access_token.to_string();
        let max_results = options.max_results;
        let provider = ImapProvider {
            host: self.host.clone(),
            port: self.port,
            email_address: self.email_address.clone(),
        };

        tokio::task::spawn_blocking(move || {
            let mut session = provider.connect_session(&password)?;
            let mailbox = session
                .select("INBOX")
                .map_err(|e| ProviderError::Other(Box::new(e)))?;

            let uidvalidity = mailbox.uid_validity.unwrap_or(0);

            let uids_set = session
                .uid_search("ALL")
                .map_err(|e| ProviderError::Other(Box::new(e)))?;

            let mut uids: Vec<u32> = uids_set.into_iter().collect();
            uids.sort_unstable();

            let highest_uid = uids.last().copied().unwrap_or(0);

            let slice_start = uids.len().saturating_sub(max_results as usize);
            let recent_uids: Vec<String> = uids[slice_start..]
                .iter()
                .rev()
                .map(|u| u.to_string())
                .collect();

            let _ = session.logout();

            Ok(SyncPage {
                new_message_ids: recent_uids,
                trashed_message_ids: Vec::new(),
                deleted_message_ids: Vec::new(),
                restored_message_ids: Vec::new(),
                next_cursor: SyncCursor(format!("{uidvalidity}:{highest_uid}")),
                has_more: false,
            })
        })
        .await
        .map_err(|e| ProviderError::Other(Box::new(e)))?
    }

    async fn incremental_sync(
        &self,
        access_token: &str,
        cursor: &SyncCursor,
    ) -> Result<SyncPage, ProviderError> {
        let password = access_token.to_string();
        let cursor_str = cursor.0.clone();
        let provider = ImapProvider {
            host: self.host.clone(),
            port: self.port,
            email_address: self.email_address.clone(),
        };

        tokio::task::spawn_blocking(move || {
            let parts: Vec<&str> = cursor_str.split(':').collect();
            if parts.len() != 2 {
                return Err(ProviderError::CursorExpired);
            }

            let expected_validity: u32 = parts[0].parse().unwrap_or(0);
            let last_seen_uid: u32 = parts[1].parse().unwrap_or(0);

            let mut session = provider.connect_session(&password)?;
            let mailbox = session
                .select("INBOX")
                .map_err(|e| ProviderError::Other(Box::new(e)))?;

            let current_validity = mailbox.uid_validity.unwrap_or(0);

            // RFC 3501 UIDVALIDITY Invariant:
            // If the server mailbox is deleted and recreated or its message index is rebuilt,
            // UIDVALIDITY changes. Previous UIDs become invalid; return CursorExpired to force
            // a fresh initial sync baseline.
            if current_validity != expected_validity {
                return Err(ProviderError::CursorExpired);
            }

            let query = format!("UID {}:*", last_seen_uid + 1);
            let uids_set = session
                .uid_search(&query)
                .map_err(|e| ProviderError::Other(Box::new(e)))?;

            let mut new_uids: Vec<u32> = uids_set
                .into_iter()
                .filter(|&uid| uid > last_seen_uid)
                .collect();
            new_uids.sort_unstable();

            let new_highest = new_uids.last().copied().unwrap_or(last_seen_uid);

            let new_message_ids: Vec<String> =
                new_uids.iter().rev().map(|u| u.to_string()).collect();

            let _ = session.logout();

            Ok(SyncPage {
                new_message_ids,
                trashed_message_ids: Vec::new(),
                deleted_message_ids: Vec::new(),
                restored_message_ids: Vec::new(),
                next_cursor: SyncCursor(format!("{current_validity}:{new_highest}")),
                has_more: false,
            })
        })
        .await
        .map_err(|e| ProviderError::Other(Box::new(e)))?
    }

    async fn fetch_message_metadata(
        &self,
        access_token: &str,
        message_ids: &[String],
    ) -> Result<Vec<Result<MessageMetadata, ProviderError>>, ProviderError> {
        if message_ids.is_empty() {
            return Ok(Vec::new());
        }

        let password = access_token.to_string();
        let ids = message_ids.to_vec();
        let provider = ImapProvider {
            host: self.host.clone(),
            port: self.port,
            email_address: self.email_address.clone(),
        };

        tokio::task::spawn_blocking(move || {
            let mut session = provider.connect_session(&password)?;
            session
                .select("INBOX")
                .map_err(|e| ProviderError::Other(Box::new(e)))?;

            // Batch all requested UIDs into a single comma-separated FETCH command
            let sequence_set = ids.join(",");

            // RFC 3501 Invariant: Standard FETCH BODY[TEXT] marks the email as \Seen on the server.
            // Using BODY.PEEK[TEXT]<0.500> samples preview headers without changing read status.
            let fetches = session
                .uid_fetch(
                    &sequence_set,
                    "(UID FLAGS RFC822.HEADER BODY.PEEK[TEXT]<0.500>)",
                )
                .map_err(|e| ProviderError::Other(Box::new(e)))?;

            let mut fetch_map = HashMap::new();
            for fetch in fetches.iter() {
                if let Some(uid) = fetch.uid {
                    fetch_map.insert(uid.to_string(), fetch);
                }
            }

            let mut results = Vec::new();

            for requested_id in &ids {
                match fetch_map.get(requested_id) {
                    Some(fetch) => {
                        let is_read = fetch
                            .flags()
                            .iter()
                            .any(|f| matches!(f, imap::types::Flag::Seen));
                        let header_bytes = fetch.header().unwrap_or_default();
                        let parsed_headers = MessageParser::default().parse_headers(header_bytes);

                        let subject = parsed_headers
                            .as_ref()
                            .and_then(|h| h.subject().map(|s| s.to_string()))
                            .unwrap_or_else(|| "(no subject)".to_string());

                        let from = parsed_headers
                            .as_ref()
                            .and_then(|h| {
                                h.from().and_then(|addrs| {
                                    addrs
                                        .first()
                                        .and_then(|a| a.address.as_ref().map(|s| s.to_string()))
                                })
                            })
                            .unwrap_or_else(|| "(no sender)".to_string());

                        let received_at = parsed_headers
                            .as_ref()
                            .and_then(|h| h.date().map(|d| d.to_timestamp()))
                            .unwrap_or_else(|| chrono::Utc::now().timestamp());

                        // MIME Boundary Scrubbing: Raw partial body text frequently includes boundary
                        // delimiters (e.g. "--boundary_123") and Content-Type lines. Scrub these before saving.
                        let snippet = fetch
                            .text()
                            .map(|bytes| {
                                let text = String::from_utf8_lossy(bytes);
                                text.lines()
                                    .map(|l| l.trim())
                                    .filter(|l| {
                                        !l.is_empty()
                                            && !l.starts_with("--")
                                            && !l.starts_with("Content-Type:")
                                            && !l.starts_with("Content-Transfer-Encoding:")
                                    })
                                    .take(2)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            })
                            .filter(|s| !s.is_empty());

                        results.push(Ok(MessageMetadata {
                            id: requested_id.clone(),
                            subject,
                            from,
                            received_at,
                            is_read,
                            snippet,
                        }));
                    }
                    None => {
                        results.push(Err(ProviderError::InvalidData(format!(
                            "Message {requested_id} not found on IMAP server (404)"
                        ))));
                    }
                }
            }

            let _ = session.logout();
            Ok(results)
        })
        .await
        .map_err(|e| ProviderError::Other(Box::new(e)))?
    }

    async fn fetch_full_message(
        &self,
        access_token: &str,
        message_id: &str,
    ) -> Result<FullMessage, ProviderError> {
        let password = access_token.to_string();
        let uid = message_id.to_string();
        let provider = ImapProvider {
            host: self.host.clone(),
            port: self.port,
            email_address: self.email_address.clone(),
        };

        tokio::task::spawn_blocking(move || {
            let mut session = provider.connect_session(&password)?;
            session
                .select("INBOX")
                .map_err(|e| ProviderError::Other(Box::new(e)))?;

            // BODY.PEEK[] Invariant: Full body download for Tier 2 AI classification
            // must never mark unread mail as read on the server.
            let fetches = session
                .uid_fetch(&uid, "BODY.PEEK[]")
                .map_err(|e| ProviderError::Other(Box::new(e)))?;

            let fetch = fetches
                .first()
                .ok_or_else(|| ProviderError::InvalidData("Message not found (404)".to_string()))?;

            let body_bytes = fetch
                .body()
                .ok_or_else(|| ProviderError::InvalidData("Empty message body".to_string()))?;

            let parsed: Message = MessageParser::default().parse(body_bytes).ok_or_else(|| {
                ProviderError::InvalidData("Failed to parse MIME message".to_string())
            })?;

            let mut body_html = parsed.body_html(0).map(|s| s.to_string());
            let body_text = parsed.body_text(0).map(|s| s.to_string()).or_else(|| {
                body_html
                    .as_ref()
                    .and_then(|html| html2text::from_read(html.as_bytes(), 80).ok())
            });

            let mut attachments = Vec::new();

            for (idx, part) in parsed.attachments().enumerate() {
                let filename = part.attachment_name().unwrap_or("attachment").to_string();
                let mime_type = part
                    .content_type()
                    .map(|c| {
                        if let Some(sub) = &c.c_subtype {
                            format!("{}/{}", c.c_type, sub)
                        } else {
                            c.c_type.to_string()
                        }
                    })
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let size = part.contents().len() as u64;

                // Inline CID images: Convert embedded attachments with Content-ID to Base64 data URIs
                if let Some(cid) = part.content_id() {
                    let clean_cid = cid.trim_start_matches('<').trim_end_matches('>');
                    let b64 = base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        part.contents(),
                    );
                    let data_uri = format!("data:{mime_type};base64,{b64}");

                    if let Some(html) = &mut body_html {
                        if let Ok(re) =
                            regex::Regex::new(&format!(r"(?i)cid:{}", regex::escape(clean_cid)))
                        {
                            *html = re.replace_all(html, &data_uri).into_owned();
                        }
                    }
                } else {
                    attachments.push(AttachmentMeta {
                        id: format!("{uid}_{idx}_{filename}"),
                        filename,
                        mime_type,
                        size,
                    });
                }
            }

            let _ = session.logout();

            Ok(FullMessage {
                id: uid,
                body_text,
                body_html,
                attachments,
                unique_body_html: None,
            })
        })
        .await
        .map_err(|e| ProviderError::Other(Box::new(e)))?
    }
}
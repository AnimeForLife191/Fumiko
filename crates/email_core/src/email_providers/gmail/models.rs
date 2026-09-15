//! Serde deserialization models for Google Gmail API responses.

use serde::Deserialize;

/// Mailbox profile response containing the baseline historyId.
#[derive(Debug, Clone, Deserialize)]
pub struct GmailProfileResponse {
    #[serde(rename = "emailAddress")]
    pub email_address: String,
    #[serde(rename = "historyId")]
    pub history_id: String,
}

/// Lightweight reference containing only a Gmail message ID.
#[derive(Debug, Clone, Deserialize)]
pub struct MessageRefId {
    pub id: String,
}

/// Paginated message listing response from `GET /users/me/messages`.
#[derive(Debug, Clone, Deserialize)]
pub struct ListMessagesResponse {
    pub messages: Option<Vec<MessageRefId>>,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageAdded {
    pub message: MessageRefId,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageDeleted {
    pub message: MessageRefId,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelAdded {
    pub message: MessageRefId,
    #[serde(rename = "labelIds")]
    pub label_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelRemoved {
    pub message: MessageRefId,
    #[serde(rename = "labelIds")]
    pub label_ids: Vec<String>,
}

/// Single change event emitted by `GET /users/me/history`.
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryRecord {
    #[serde(rename = "messagesAdded")]
    pub messages_added: Option<Vec<MessageAdded>>,
    #[serde(rename = "messagesDeleted")]
    pub messages_deleted: Option<Vec<MessageDeleted>>,
    #[serde(rename = "labelsAdded")]
    pub labels_added: Option<Vec<LabelAdded>>,
    #[serde(rename = "labelsRemoved")]
    pub labels_removed: Option<Vec<LabelRemoved>>,
}

/// Paginated history change log response.
#[derive(Debug, Clone, Deserialize)]
pub struct HistoryListResponse {
    pub history: Option<Vec<HistoryRecord>>,
    #[serde(rename = "historyId")]
    pub history_id: String,
    #[serde(rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessagePayloadHeaders {
    pub headers: Vec<MessageHeader>,
}

/// Summary representation returned when querying `format=metadata`.
#[derive(Debug, Clone, Deserialize)]
pub struct GmailMessageSummaryResponse {
    pub payload: Option<MessagePayloadHeaders>,
    #[serde(rename = "internalDate")]
    pub internal_date: String,
    #[serde(rename = "labelIds")]
    pub label_ids: Option<Vec<String>>,
    pub snippet: Option<String>,
}

/// Encoded message body or attachment reference within a MIME part.
#[derive(Debug, Clone, Deserialize)]
pub struct MessagePartBody {
    pub data: Option<String>,
    #[serde(rename = "attachmentId")]
    pub attachment_id: Option<String>,
    pub size: Option<u64>,
}

/// Recursive MIME tree structure representing multipart message payloads.
#[derive(Debug, Clone, Deserialize)]
pub struct MessagePartFull {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub filename: Option<String>,
    pub headers: Option<Vec<MessageHeader>>,
    pub body: Option<MessagePartBody>,
    pub parts: Option<Vec<MessagePartFull>>,
}

/// Attachment data response payload returned by `GET /attachments/{id}`.
#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentDataResponse {
    pub data: String,
}

/// Full message representation returned by `format=full`.
#[derive(Debug, Clone, Deserialize)]
pub struct GmailFullMessageResponse {
    pub payload: Option<MessagePartFull>,
}
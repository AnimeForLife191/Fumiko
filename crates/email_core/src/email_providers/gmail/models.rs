use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct GmailProfileResponse {
    #[serde(rename = "emailAddress")]
    pub email_address: String,
    // history_id is a numeric string representing the current state of the mailbox.
    // It serves as the baseline cursor for incremental sync.
    #[serde(rename = "historyId")]
    pub history_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageRefId {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListMessagesResponse {
    // Note: Gmail omits the `messages` key entirely when a search or label query
    // matches 0 items, rather than returning an empty array `[]`.
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
    // label_ids contains ONLY the newly applied labels in this history event,
    // not the complete set of labels currently attached to the message.
    #[serde(rename = "labelIds")]
    pub label_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelRemoved {
    pub message: MessageRefId,
    #[serde(rename = "labelIds")]
    pub label_ids: Vec<String>,
}

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

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryListResponse {
    pub history: Option<Vec<HistoryRecord>>,
    // Points to the newest history point reached during this sync window.
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

#[derive(Debug, Clone, Deserialize)]
pub struct GmailMessageSummaryResponse {
    pub payload: Option<MessagePayloadHeaders>,
    // Gmail returns internalDate as epoch milliseconds serialized within a JSON string.
    #[serde(rename = "internalDate")]
    pub internal_date: String,
    #[serde(rename = "labelIds")]
    pub label_ids: Option<Vec<String>>,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessagePartBody {
    // Inline Base64 URL-safe encoded data (only present if payload is below size limits).
    pub data: Option<String>,

    // Populated if Gmail offloaded the part's body to attachment storage due to size.
    #[serde(rename = "attachmentId")]
    pub attachment_id: Option<String>,

    pub size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessagePartFull {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub filename: Option<String>,
    pub headers: Option<Vec<MessageHeader>>,
    pub body: Option<MessagePartBody>,
    // Recursive MIME tree: multipart messages contain child parts at arbitrary nesting depths.
    pub parts: Option<Vec<MessagePartFull>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentDataResponse {
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GmailFullMessageResponse {
    pub payload: Option<MessagePartFull>,
}

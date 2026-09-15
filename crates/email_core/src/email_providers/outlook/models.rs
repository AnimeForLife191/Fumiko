//! Serde models for Microsoft Graph API responses.

use serde::Deserialize;

/// User profile response from Microsoft Graph `GET /me`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookProfileResponse {
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
    pub display_name: Option<String>,
}

/// Sparse message representation returned by `$select` queries.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookMessageResponse {
    pub _id: Option<String>,
    pub subject: Option<String>,
    pub from: Option<OutlookRecipient>,
    pub received_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub body: Option<OutlookBody>,
    pub unique_body: Option<OutlookBody>,
    pub body_preview: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookRecipient {
    pub email_address: Option<OutlookEmailAddress>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookEmailAddress {
    pub _name: Option<String>,
    pub address: String,
}

/// Message body payload returned by Microsoft Graph.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookBody {
    pub content_type: String,
    pub content: String,
}

/// Paginated delta query response for change tracking.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaResponse {
    pub value: Vec<DeltaMessageItem>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

/// Individual item inside a delta response.
#[derive(Debug, Clone, Deserialize)]
pub struct DeltaMessageItem {
    pub id: String,
    #[serde(rename = "@removed")]
    pub removed: Option<RemovedReason>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemovedReason {
    pub _reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentListResponse {
    pub value: Vec<OutlookAttachment>,
}

/// Attachment item returned by `GET /messages/{id}/attachments`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookAttachment {
    pub id: String,
    pub name: String,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    pub is_inline: Option<bool>,
    pub content_id: Option<String>,
    pub content_bytes: Option<String>,
}
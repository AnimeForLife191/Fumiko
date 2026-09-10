use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookProfileResponse {
    // Microsoft 365 accounts use `mail`, while personal MS / guest accounts
    // often only populate `userPrincipalName`.
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookMessageResponse {
    // Graph endpoints return sparse objects containing only the fields specified in `$select`.
    // All fields are therefore `Option` to allow reusing this struct across different queries.
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookBody {
    // Graph returns contentType as either "text" or "html".
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaResponse {
    pub value: Vec<DeltaMessageItem>,
    // @odata.nextLink indicates additional result pages remain in this delta pass.
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
    // @odata.deltaLink is ONLY returned on the final page of a delta query and
    // serves as the cursor for the subsequent incremental sync.
    #[serde(rename = "@odata.deltaLink")]
    pub delta_link: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeltaMessageItem {
    pub id: String,
    // Graph marks both hard deletions and items moved out of the folder with `@removed`.
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlookAttachment {
    pub id: String,
    pub name: String,
    pub content_type: Option<String>,
    pub size: Option<i64>,
    pub is_inline: Option<bool>,
    // Matches the Content-ID referenced by `<img src="cid:...">` in HTML bodies.
    pub content_id: Option<String>,
    // Raw Base64 attachment bytes; only populated when explicitly queried with $select on fileAttachment.
    pub content_bytes: Option<String>,
}

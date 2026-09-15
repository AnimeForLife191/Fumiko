//! Responsive split-view mailbox browser supporting multi-account filtering and search.

use common::{config, setting_keys};
use dioxus::prelude::*;
use storage::Storage;
use storage::models::EmailRow;
use uuid::Uuid;

use super::component::{EmptyState, ReadingPane};
use crate::state::{AppState, SyncTarget};
use crate::utils::format_email_timestamp;

#[component]
pub fn Inbox() -> Element {
    let mut state = use_context::<AppState>();
    let storage = use_context::<Storage>();

    let mut search_query = use_signal(String::new);

    let layout_class = if (state.selected_email)().is_some() {
        "split-view split-view--has-selection"
    } else {
        "split-view"
    };

    let current_accounts = (state.accounts)();
    let account_names: std::collections::HashMap<Uuid, String> = current_accounts
        .iter()
        .map(|a| {
            let name = a
                .display_name
                .as_deref()
                .unwrap_or(&a.email_address)
                .to_string();
            (a.id, name)
        })
        .collect();

    let selected = (state.selected_account)();

    let account_display = match selected {
        None => "All Accounts".to_string(),
        Some(account_id) => current_accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| {
                a.display_name
                    .as_deref()
                    .unwrap_or(&a.email_address)
                    .to_string()
            })
            .unwrap_or_else(|| "Unknown Account".to_string()),
    };

    let last_synced_ts = match selected {
        Some(account_id) => current_accounts
            .iter()
            .find(|a| a.id == account_id)
            .and_then(|a| a.last_synced_at),
        None => current_accounts
            .iter()
            .filter_map(|a| a.last_synced_at)
            .max(),
    };

    let emails = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let selected = (state.selected_account)();
            let _ = (state.sync_tick)();
            let _ = (state.refresh_trigger)();
            async move {
                let limit = storage
                    .get_setting(setting_keys::INBOX_CAPACITY)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(config::DEFAULT_INBOX_CAPACITY as i64);

                match selected {
                    Some(account_id) => storage.list_inbox(account_id, limit).await,
                    None => storage.list_inbox_all(limit).await
                }
            }
        }
    });

    let raw_email_list = emails
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    let email_list: Vec<EmailRow> = {
        let query = search_query.read().trim().to_lowercase();
        if query.is_empty() {
            raw_email_list
        } else {
            raw_email_list
                .into_iter()
                .filter(|email| {
                    let subject = email.subject.as_deref().unwrap_or("").to_lowercase();
                    let sender = email.sender.as_deref().unwrap_or("").to_lowercase();
                    let snippet = email.snippet.as_deref().unwrap_or("").to_lowercase();

                    subject.contains(&query) || sender.contains(&query) || snippet.contains(&query)
                })
                .collect()
        }
    };

    let unread_count = email_list
        .iter()
        .filter(|email| !email.is_read && !email.app_has_viewed)
        .count();

    rsx! {
        div { class: "inbox",
            div { class: "{layout_class}",
                div { class: "split-view__list-pane",
                    div { class: "inbox__header",
                        div { class: "inbox__heading",
                            div { class: "inbox__title-wrap",
                                h1 { class: "inbox__title", "{account_display}" }
                                p { class: "inbox__subtitle",
                                    if selected.is_some() {
                                        "Showing emails from {account_display}"
                                    } else {
                                        "Showing emails from all accounts"
                                    }
                                }
                            }

                            div { class: "inbox__actions",
                                button {
                                    class: "inbox__btn inbox__btn--secondary",
                                    disabled: unread_count == 0,
                                    onclick: {
                                        let storage = storage.clone();
                                        move |_| {
                                            let storage = storage.clone();
                                            let selected = (state.selected_account)();
                                            spawn(async move {
                                                if storage.mark_all_read(selected).await.is_ok() {
                                                    state.refresh_trigger.with_mut(|n| *n += 1);
                                                }
                                            });
                                        }
                                    },
                                    "Mark all read"
                                }
                                button {
                                    class: "inbox__btn inbox__btn--primary",
                                    disabled: (state.is_syncing)(),
                                    onclick: move |_| {
                                        let target = match (state.selected_account)() {
                                            Some(account_id) => SyncTarget::One(account_id),
                                            None => SyncTarget::All,
                                        };
                                        let _ = (state.sync_tx)().send(target);
                                    },
                                    if (state.is_syncing)() {
                                        "Syncing..."
                                    } else {
                                        "Sync Now"
                                    }
                                }
                            }
                        }

                        div { class: "inbox__search-wrap",
                            span {
                                class: "icon icon--search inbox__search-icon",
                                aria_hidden: "true",
                            }
                            input {
                                class: "inbox__search-input",
                                r#type: "text",
                                placeholder: "Search subject, sender, or preview...",
                                value: "{search_query}",
                                oninput: move |evt| search_query.set(evt.value()),
                            }
                            if !search_query.read().is_empty() {
                                button {
                                    class: "inbox__search-clear",
                                    onclick: move |_| search_query.set(String::new()),
                                    "Clear"
                                }
                            }
                        }

                        div { class: "inbox__stats",
                            div { class: "inbox__stats-left",
                                span { class: "inbox__count", "{email_list.len()} emails" }
                                if unread_count > 0 {
                                    span { class: "inbox__unread", "{unread_count} unread" }
                                }
                            }
                            if let Some(ts) = last_synced_ts {
                                span { class: "inbox__last-synced",
                                    "Last synced: {format_email_timestamp(ts)}"
                                }
                            }
                        }
                    }

                    div { class: "inbox__message-panel",
                        if email_list.is_empty() {
                            EmptyState {
                                icon_class: "icon icon--mails",
                                title: "No emails found",
                                subtitle: "Link an account or click Sync Now to fetch your messages.",
                            }
                        } else {
                            for email in email_list {
                                InboxEmailRow {
                                    key: "{email.id}",
                                    account_name: account_names
                                        .get(&email.account_id)
                                        .cloned()
                                        .unwrap_or_else(|| "Unknown Account".to_string()),
                                    is_selected: (state.selected_email)() == Some(email.id),
                                    on_select: {
                                        let email_id = email.id;
                                        move |_| state.selected_email.set(Some(email_id))
                                    },
                                    email,
                                }
                            }
                        }
                    }
                }

                div { class: "split-view__detail-pane",
                    if let Some(id) = (state.selected_email)() {
                        ReadingPane {
                            key: "{id}",
                            email_id: id,
                            on_close: move |_| state.selected_email.set(None),
                        }
                    } else {
                        EmptyState {
                            icon_class: "icon icon--mail",
                            title: "Select an email to read",
                            subtitle: "Click any email on the left to preview its content.",
                        }
                    }
                }
            }
        }
    }
}

/// Email row item in the inbox list displaying read status, preview snippets, and quick trash actions.
#[component]
fn InboxEmailRow(
    account_name: String,
    email: EmailRow,
    is_selected: bool,
    on_select: EventHandler<MouseEvent>,
) -> Element {
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    let id = email.id;
    let subject = email.subject.as_deref().unwrap_or("(No subject)");
    let sender = email.sender.as_deref().unwrap_or("(Unknown sender)");
    let snippet = email.snippet.as_deref().unwrap_or("");
    let is_unread = !email.is_read && !email.app_has_viewed;

    let trash = move |evt: Event<MouseData>| {
        evt.stop_propagation();
        let storage = storage.clone();

        if (state.selected_email)() == Some(id) {
            state.selected_email.set(None);
        }

        spawn(async move {
            if let Err(e) = storage.trash_email(id).await {
                tracing::error!("failed to trash email {id}: {e}");
            } else {
                state.refresh_trigger.with_mut(|n| *n += 1);
            }
        });
    };

    let row_class = if is_selected {
        "inbox-email-row inbox-email-row--selected"
    } else if is_unread {
        "inbox-email-row inbox-email-row--unread"
    } else {
        "inbox-email-row"
    };

    rsx! {
        div { class: "{row_class}", onclick: move |evt| on_select.call(evt),
            div { class: "inbox-email-row__top",
                div { class: "inbox-email-row__sender-wrap",
                    if is_unread {
                        span {
                            class: "inbox-email-row__unread-dot",
                            title: "Unread email",
                        }
                    }
                    span { class: "inbox-email-row__sender", "{sender}" }
                }
                span { class: "inbox-email-row__time", "{format_email_timestamp(email.received_at)}" }
            }

            div { class: "inbox-email-row__subject-wrap",
                span { class: "inbox-email-row__subject", "{subject}" }
                button {
                    class: "inbox-email-row__trash-btn",
                    onclick: trash,
                    title: "Move to Trash",
                    span { class: "icon icon--trash", aria_hidden: "true" }
                }
            }

            if !snippet.is_empty() {
                span { class: "inbox-email-row__snippet", "{snippet}" }
            }

            div { class: "inbox-email-row__meta",
                span { class: "inbox-email-row__account", "{account_name}" }
            }
        }
    }
}
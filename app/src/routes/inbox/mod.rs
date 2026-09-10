use dioxus::prelude::*;
use storage::{Storage, models::EmailRow};
use uuid::Uuid;

use super::component::{EmptyState, ReadingPane};
use crate::utils::format_email_timestamp;
use crate::state::{AppState, SyncTarget};

#[component]
pub fn Inbox() -> Element {
    let mut state = use_context::<AppState>();
    let storage = use_context::<Storage>();

    let layout_class = if (state.selected_email)().is_some() {
        "split-view split-view--has-selection"
    } else {
        "split-view"
    };

    // 1. Account name lookup map
    let current_accounts = (state.accounts)() ;
    let account_names: std::collections::HashMap<Uuid, String> = current_accounts
        .iter()
        .map(|a| {
            let name = a.display_name.as_deref().unwrap_or(&a.email_address).to_string();
            (a.id, name)
        })
        .collect();

    let selected = (state.selected_account)();

    let account_display = match selected {
        None => "All Accounts".to_string(),
        Some(account_id) => current_accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.display_name.as_deref().unwrap_or(&a.email_address).to_string())
            .unwrap_or_else(|| "Unknown Account".to_string()),
    };

    let last_synced_ts = match selected {
        Some(account_id) => current_accounts
            .iter()
            .find(|a| a.id == account_id)
            .and_then(|a| a.last_synced_at),
        None => current_accounts.iter().filter_map(|a| a.last_synced_at).max(),
    };

    // 2. Fetch inbox emails
    let emails = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let selected = (state.selected_account)();
            let _ = (state.sync_tick)();
            let _ = (state.refresh_trigger)();
            async move {
                match selected {
                    Some(account_id) => storage.list_inbox(account_id, 200).await,
                    None => storage.list_inbox_all(200).await,
                }
            }
        }
    });

    let email_list = emails
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    let unread_count = email_list.iter().filter(|email| !email.is_read).count();

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
                            button {
                                class: "inbox__sync-button",
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

                        div { class: "inbox__stats",
                            span { class: "inbox__count", "{email_list.len()} emails" }
                            if unread_count > 0 {
                                span { class: "inbox__unread", "{unread_count} unread" }
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
                                icon: "📭",
                                title: "No emails found",
                                subtitle: "Link an account to start receiving emails.",
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
                            icon: "📬",
                            title: "Select an email to read",
                            subtitle: "Click any email on the left to preview its content.",
                        }
                    }
                }
            }
        }
    }
}

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
    let has_been_viewed = email.app_has_viewed;

    let trash = move |evt: Event<MouseData>| {
        evt.stop_propagation();
        let storage = storage.clone();
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
    } else if email.is_read || has_been_viewed {
        "inbox-email-row inbox-email-row--viewed"
    } else {
        "inbox-email-row inbox-email-row--unread"
    };

    rsx! {
        div { class: "{row_class}", onclick: move |evt| on_select.call(evt),

            // 1. Top Bar: Account on left, Timestamp on right
            div { class: "inbox-email-row__top",
                span { class: "inbox-email-row__account", "{account_name}" }
                span { class: "inbox-email-row__time", "{format_email_timestamp(email.received_at)}" }
            }

            // 2. Middle Row: Subject on left, Trash on right
            div { class: "inbox-email-row__subject-wrap",
                span { class: "inbox-email-row__subject", "{subject}" }
                button {
                    class: "inbox-email-row__trash-btn",
                    onclick: trash,
                    title: "Move to Trash",
                    "🗑"
                }
            }

            // 3. Bottom Row: Sender
            span { class: "inbox-email-row__sender", "{sender}" }
        }
    }
}
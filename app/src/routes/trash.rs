//! Soft-deleted email management, retention tracking, and permanent purge actions.

use dioxus::prelude::*;
use storage::{Storage, models::EmailRow};

use crate::AppState;
use crate::utils::format_email_timestamp;

#[component]
pub fn Trash() -> Element {
    let store = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    let retention_resource = use_resource({
        let store = store.clone();
        move || {
            let store = store.clone();
            let _ = (state.refresh_trigger)();
            async move {
                store
                    .get_setting(common::setting_keys::TRASH_RETENTION_DAYS)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|val| val.parse::<i64>().ok())
                    .unwrap_or(30)
            }
        }
    });

    let retention_days = retention_resource.read().cloned().unwrap_or(30);

    // Automated Retention Check:
    // Purges expired trash entries automatically whenever the view opens or background sync ticks
    use_resource({
        let store = store.clone();
        move || {
            let store = store.clone();
            let _ = (state.sync_tick)();
            async move {
                let _ = store.purge_expired_trash(retention_days).await;
            }
        }
    });

    let trashed = use_resource({
        let store = store.clone();
        move || {
            let store = store.clone();
            let _ = (state.refresh_trigger)();
            let _ = (state.sync_tick)();
            async move { store.list_trash(200).await }
        }
    });

    let subtitle_text = if retention_days == 0 {
        "Emails are deleted immediately (0-day retention).".to_string()
    } else {
        format!("Emails are permanently deleted after {retention_days} days.")
    };

    let trash_list = trashed
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    let count = trash_list.len();

    // Manual Immediate Purge: Passing 0 permanently deletes all currently trashed emails during the next sync
    let empty_trash = {
        let store = store.clone();
        move |_| {
            let store = store.clone();
            spawn(async move {
                if let Ok(purged) = store.purge_expired_trash(0).await {
                    tracing::info!("Manually emptied trash: {purged} emails purged");
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
            });
        }
    };

    rsx! {
        div { class: "trash",
            div { class: "trash__header",
                div { class: "trash__header-text",
                    h1 { class: "trash__title", "Trash" }
                    p { class: "trash__subtitle", "{subtitle_text}" }
                }

                if count > 0 {
                    button { class: "trash__empty-btn", onclick: empty_trash,
                        span { class: "icon icon--trash", aria_hidden: "true" }
                        "Empty Trash Now"
                    }
                }
            }

            div { class: "trash__panel",
                if count > 0 {
                    div { class: "trash__panel-header",
                        span { class: "trash__panel-title", "Soft-Deleted Messages" }
                        span { class: "trash__badge", "{count} emails" }
                    }
                }

                if trash_list.is_empty() {
                    div { class: "empty-state",
                        span {
                            class: "empty-state__icon icon icon--trash",
                            aria_hidden: "true",
                        }
                        h3 { class: "empty-state__title", "Trash is empty" }
                        p { class: "empty-state__hint",
                            "Emails soft-deleted during sync or removed from your inbox will appear here."
                        }
                    }
                } else {
                    div { class: "trash__list",
                        for email in trash_list {
                            TrashEmailRow { key: "{email.id}", email }
                        }
                    }
                }
            }
        }
    }
}

/// Trashed email row item supporting instant un-trashing or permanent deletion.
#[component]
fn TrashEmailRow(email: EmailRow) -> Element {
    let store = use_context::<Storage>();
    let mut state = use_context::<AppState>();
    let id = email.id;

    let subject = email.subject.as_deref().unwrap_or("(No subject)");
    let sender = email.sender.as_deref().unwrap_or("(Unknown sender)");

    let trashed_display = email
        .trashed_at
        .map(|ts| format!("Trashed {}", format_email_timestamp(ts)))
        .unwrap_or_else(|| "Trashed recently".to_string());

    let restore = {
        let store = store.clone();
        move |_| {
            let store = store.clone();
            spawn(async move {
                if let Err(e) = store.restore_email(id).await {
                    tracing::error!("failed to restore email {id}: {e}");
                } else {
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
            });
        }
    };

    let delete_forever = {
        let store = store.clone();
        move |_| {
            let store = store.clone();
            spawn(async move {
                if let Err(e) = store.delete_email(id).await {
                    tracing::error!("failed to delete email {id}: {e}");
                } else {
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
            });
        }
    };

    rsx! {
        div { class: "trash-email-row",
            div { class: "trash-email-row__top",
                span { class: "trash-email-row__subject", "{subject}" }
                span { class: "trash-email-row__trashed-at", "{trashed_display}" }
            }

            span { class: "trash-email-row__sender", "{sender}" }

            div { class: "trash-email-row__actions",
                button {
                    class: "trash-email-row__action trash-email-row__action--restore",
                    onclick: restore,
                    "Restore"
                }
                button {
                    class: "trash-email-row__action trash-email-row__action--delete",
                    onclick: delete_forever,
                    "Delete Permanently"
                }
            }
        }
    }
}
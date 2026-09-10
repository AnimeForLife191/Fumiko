use dioxus::prelude::*;
use storage::{Storage, models::EmailRow};

use super::component::EmptyState;
use crate::AppState;
use crate::utils::format_email_timestamp;

#[component]
pub fn Trash() -> Element {
    let store = use_context::<Storage>();
    let state = use_context::<AppState>();

    let trashed = use_resource({
        let store = store.clone();
        move || {
            let store = store.clone();
            let _ = (state.refresh_trigger)();
            let _ = (state.sync_tick)();
            async move { store.list_trash(200).await }
        }
    });

    let trash_list = trashed
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    rsx! {
        div { class: "trash",
            div { class: "trash__header",
                h1 { class: "trash__title", "Trash" }
                p { class: "trash__subtitle",
                    "Emails are permanently deleted after your configured retention period."
                }
            }

            div { class: "trash__panel",
                if trash_list.is_empty() {
                    EmptyState { icon: "🗑", title: "Trash is empty" }
                } else {
                    for email in trash_list {
                        TrashEmailRow { key: "{email.id}", email }
                    }
                }
            }
        }
    }
}

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

    rsx! {
        div { class: "trash-email-row",
            span { class: "trash-email-row__subject", "{subject}" }
            span { class: "trash-email-row__sender", "{sender}" }
            span { class: "trash-email-row__trashed-at", "{trashed_display}" }

            button { class: "trash-email-row__restore-btn", onclick: restore, "Restore" }
        }
    }
}
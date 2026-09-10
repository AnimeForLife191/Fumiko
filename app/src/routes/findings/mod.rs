use dioxus::prelude::*;
use storage::{Storage, models::MatchedEmailWithReason};
use uuid::Uuid;

use super::component::{EmptyState, ReadingPane};
use crate::AppState;
use crate::utils::format_email_timestamp;

#[component]
pub fn Findings() -> Element {
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    // Shared selected_email state for split-view
    let layout_class = if (state.selected_email)().is_some() {
        "split-view split-view--has-selection"
    } else {
        "split-view"
    };

    // 1. Account lookup map
    let account_names: std::collections::HashMap<Uuid, String> = (state.accounts)()
        .into_iter()
        .map(|a| {
            let name = a.display_name.unwrap_or(a.email_address);
            (a.id, name)
        })
        .collect();

    // 2. Fetch findings based on active account filter
    let findings = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let selected = (state.selected_account)();
            let _ = (state.sync_tick)();
            let _ = (state.refresh_trigger)();
            async move {
                match selected {
                    Some(account_id) => storage.list_criteria_for_account(account_id, 200).await,
                    None => storage.list_criteria_for_all(200).await,
                }
            }
        }
    });

    let finding_list = findings
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    rsx! {
        div { class: "findings",
            div { class: "{layout_class}",
                div { class: "split-view__list-pane",
                    div { class: "findings__header",
                        h1 { class: "findings__title", "Findings" }
                        p { class: "findings__subtitle",
                            "Emails that matched one of your watch criteria"
                        }
                    }

                    div { class: "findings__panel",
                        div { class: "findings__panel-header",
                            span { class: "findings__panel-title", "Matches" }
                            span { class: "findings__badge", "{finding_list.len()} found" }
                        }

                        div { class: "findings__panel-body",
                            if finding_list.is_empty() {
                                EmptyState {
                                    icon: "🔍",
                                    title: "No findings yet",
                                    subtitle: "Emails matching your active watch criteria will show up here after your next sync.",
                                }
                            } else {
                                for finding in finding_list {
                                    FindingRow {
                                        key: "{finding.email_id}:{finding.criterion_id}",
                                        account_name: account_names
                                            .get(&finding.account_id)
                                            .cloned()
                                            .unwrap_or_else(|| "Unknown Account".to_string()),
                                        is_selected: (state.selected_email)() == Some(finding.email_id),
                                        on_select: {
                                            let email_id = finding.email_id;
                                            move |_| state.selected_email.set(Some(email_id))
                                        },
                                        finding,
                                    }
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
                            icon: "🏷️",
                            title: "Select a finding to read",
                            subtitle: "Click any matched email on the left to inspect its contents.",
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FindingRow(
    finding: MatchedEmailWithReason,
    account_name: String,
    is_selected: bool,
    on_select: EventHandler<MouseEvent>,
) -> Element {
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    let email_id = finding.email_id;
    let criterion_id = finding.criterion_id;

    let subject = finding.subject.as_deref().unwrap_or("(No subject)");
    let sender = finding.sender.as_deref().unwrap_or("(Unknown sender)");
    let confidence_pct = finding.confidence.map(|c| (c * 100.0).round() as i32);

    let remove_match = {
        let storage = storage.clone();
        move |evt: Event<MouseData>| {
            evt.stop_propagation();
            let storage = storage.clone();
            spawn(async move {
                if let Err(e) = storage.delete_classification(email_id, criterion_id).await {
                    tracing::error!("failed to remove classification for email {email_id}: {e}");
                } else {
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
            });
        }
    };

    let trash_email = {
        let storage = storage.clone();
        move |evt: Event<MouseData>| {
            evt.stop_propagation();
            let storage = storage.clone();
            spawn(async move {
                if let Err(e) = storage.trash_email(email_id).await {
                    tracing::error!("failed to trash email {email_id}: {e}");
                } else {
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
            });
        }
    };

    let row_class = if is_selected {
        "findings__row findings__row--selected"
    } else {
        "findings__row"
    };

    rsx! {
        div { class: "{row_class}", onclick: move |evt| on_select.call(evt),

            // 1. Top Bar: Badge + Confidence on left, Timestamp on right
            div { class: "findings__top",
                div { class: "findings__criterion-wrap",
                    span { class: "findings__criterion-badge", "{finding.criterion_label}" }
                    if let Some(pct) = confidence_pct {
                        span { class: "findings__confidence", "{pct}%" }
                    }
                }
                span { class: "findings__time", "{format_email_timestamp(finding.received_at)}" }
            }

            // 2. Middle lines: Subject & Sender
            span { class: "findings__subject", "{subject}" }
            span { class: "findings__sender", "{sender}" }

            // 3. Bottom Row: Account on left, Clear / Delete buttons on right
            div { class: "findings__meta",
                span { class: "findings__account", "{account_name}" }
                div { class: "findings__actions",
                    button {
                        class: "findings__action findings__action--remove",
                        onclick: remove_match,
                        title: "Remove this AI classification",
                        "Clear"
                    }
                    button {
                        class: "findings__action findings__action--trash",
                        onclick: trash_email,
                        title: "Move this email to trash",
                        "Delete"
                    }
                }
            }
        }
    }
}
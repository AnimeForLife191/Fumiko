//! Priority findings board filtering emails matched against active AI classification criteria.

use std::collections::BTreeSet;

use common::{config, setting_keys};
use dioxus::prelude::*;
use storage::Storage;
use storage::models::MatchedEmailWithReason;
use uuid::Uuid;

use super::component::{EmptyState, ReadingPane};
use crate::state::AppState;
use crate::utils::format_email_timestamp;

#[component]
pub fn Findings() -> Element {
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    let mut selected_label = use_signal(|| Option::<String>::None);
    let mut search_query = use_signal(String::new);

    let layout_class = if (state.selected_email)().is_some() {
        "split-view split-view--has-selection"
    } else {
        "split-view"
    };

    let account_names: std::collections::HashMap<Uuid, String> = (state.accounts)()
        .into_iter()
        .map(|a| {
            let name = a.display_name.unwrap_or(a.email_address);
            (a.id, name)
        })
        .collect();

    let findings = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let selected = (state.selected_account)();
            let _ = (state.sync_tick)();
            let _ = (state.refresh_trigger)();
            async move {
                let limit = storage
                    .get_setting(setting_keys::FINDINGS_CAPACITY)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or(config::DEFAULT_FINDINGS_CAPACITY as i64);

                match selected {
                    Some(account_id) => storage.list_criteria_for_account(account_id, limit).await,
                    None => storage.list_criteria_for_all(limit).await,
                }
            }
        }
    });

    let raw_finding_list = findings
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    let clear_findings = {
        let storage = storage.clone();
        let raw_list = raw_finding_list.clone();
        move |_| {
            let storage = storage.clone();
            let selected_acc = (state.selected_account)();
            let active_label = selected_label.read().clone();

            let target_criterion_id = active_label.as_deref().and_then(|label| {
                raw_list
                    .iter()
                    .find(|f| f.criterion_label == label)
                    .map(|f| f.criterion_id)
            });

            state.selected_email.set(None);

            spawn(async move {
                match storage
                    .clear_classifications(selected_acc, target_criterion_id)
                    .await
                {
                    Ok(count) => {
                        tracing::info!("Cleared {count} findings");
                        state.refresh_trigger.with_mut(|n| *n += 1);
                    }
                    Err(e) => {
                        tracing::error!("Failed to clear findings: {e}");
                    }
                }
            });
        }
    };

    let available_labels: Vec<String> = raw_finding_list
        .iter()
        .map(|f| f.criterion_label.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let finding_list: Vec<MatchedEmailWithReason> = {
        let active_label = selected_label.read().clone();
        let query = search_query.read().trim().to_lowercase();

        raw_finding_list
            .into_iter()
            .filter(|f| {
                let matches_label = match &active_label {
                    Some(label) => &f.criterion_label == label,
                    None => true,
                };

                if !matches_label {
                    return false;
                }

                if query.is_empty() {
                    true
                } else {
                    let subject = f.subject.as_deref().unwrap_or("").to_ascii_lowercase();
                    let sender = f.sender.as_deref().unwrap_or("").to_ascii_lowercase();

                    subject.contains(&query) || sender.contains(&query)
                }
            })
            .collect()
    };

    let count = finding_list.len();

    rsx! {
        div { class: "findings",
            div { class: "{layout_class}",
                div { class: "split-view__list-pane",
                    div { class: "findings__header",
                        div { class: "findings__header-info",
                            h1 { class: "findings__title", "Findings Board" }
                            p { class: "findings__subtitle",
                                "Emails matched against your active AI watch rules"
                            }
                        }

                        div { class: "findings__controls",
                            div { class: "findings__search-wrap",
                                span {
                                    class: "icon icon--search findings__search-icon",
                                    aria_hidden: "true",
                                }
                                input {
                                    class: "findings__search-input",
                                    r#type: "text",
                                    placeholder: "Search subject or sender...",
                                    value: "{search_query}",
                                    oninput: move |evt| search_query.set(evt.value()),
                                }
                                if !search_query.read().is_empty() {
                                    button {
                                        class: "findings__search-clear",
                                        onclick: move |_| search_query.set(String::new()),
                                        "Clear"
                                    }
                                }
                            }

                            select {
                                class: "findings__label-select",
                                value: selected_label.read().as_deref().unwrap_or("ALL"),
                                onchange: move |evt| {
                                    let val = evt.value();
                                    if val == "ALL" {
                                        selected_label.set(None);
                                    } else {
                                        selected_label.set(Some(val));
                                    }
                                },
                                option { value: "ALL", "All Labels" }
                                for label in available_labels {
                                    option { value: "{label}", "{label}" }
                                }
                            }
                        }
                    }

                    div { class: "findings__panel",
                        div { class: "findings__panel-header",
                            span { class: "findings__panel-title", "Priority Matches" }

                            div { class: "findings__panel-header-actions",
                                span { class: "findings__badge", "{count} found" }
                                if count > 0 {
                                    {
                                        let is_filtered = selected_label.read().is_some();
                                        let button_text = if is_filtered { "Clear Category" } else { "Clear All" };
                                        let button_title = if let Some(label) = selected_label.read().as_deref() {
                                            format!("Remove {label} matches from this board")
                                        } else {
                                            "Remove all matches from this board".to_string()
                                        };

                                        rsx! {
                                            button {
                                                class: "findings__clear-all-btn",
                                                onclick: clear_findings,
                                                title: "{button_title}",
                                                "{button_text}"
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        div { class: "findings__panel-body",
                            if finding_list.is_empty() {
                                EmptyState {
                                    icon_class: "icon icon--flower",
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
                            icon_class: "icon icon--search",
                            title: "Select a finding to inspect",
                            subtitle: "Click any matched email on the left to read its full contents.",
                        }
                    }
                }
            }
        }
    }
}

/// Finding row item displaying matched rule badges, confidence scores, and triage actions.
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
    let is_viewed = finding.is_read || finding.app_has_viewed;

    // Triage Action: Clear Classification
    // Removes the AI finding link from SQLite without moving or deleting the underlying email.
    let remove_match = {
        let storage = storage.clone();
        move |evt: Event<MouseData>| {
            evt.stop_propagation();
            let storage = storage.clone();

            if (state.selected_email)() == Some(email_id) {
                state.selected_email.set(None);
            }

            spawn(async move {
                if let Err(e) = storage.delete_classification(email_id, criterion_id).await {
                    tracing::error!("failed to remove classification for email {email_id}: {e}");
                } else {
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
            });
        }
    };

    // Triage Action: Move to Trash
    // Soft-deletes the email itself, which cascades to remove the finding as well.
    let trash_email = {
        let storage = storage.clone();
        move |evt: Event<MouseData>| {
            evt.stop_propagation();
            let storage = storage.clone();

            if (state.selected_email)() == Some(email_id) {
                state.selected_email.set(None);
            }

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
    } else if !is_viewed {
        "findings__row findings__row--unread"
    } else {
        "findings__row"
    };

    rsx! {
        div { class: "{row_class}", onclick: move |evt| on_select.call(evt),
            div { class: "findings__top",
                div { class: "findings__criterion-wrap",
                    if !is_viewed {
                        span {
                            class: "findings__unread-dot",
                            title: "Unread finding",
                        }
                    }
                    span { class: "findings__criterion-badge", "{finding.criterion_label}" }
                    if let Some(pct) = confidence_pct {
                        span { class: "findings__confidence", "{pct}% Match" }
                    }
                }
                span { class: "findings__time", "{format_email_timestamp(finding.received_at)}" }
            }

            span { class: "findings__subject", "{subject}" }
            span { class: "findings__sender", "{sender}" }

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
                        span { class: "icon icon--trash", aria_hidden: "true" }
                        "Delete"
                    }
                }
            }
        }
    }
}
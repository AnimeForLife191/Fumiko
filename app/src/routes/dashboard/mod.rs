use dioxus::prelude::*;
use storage::{Storage, models::MatchedEmailWithReason};
use uuid::Uuid;

use super::component::EmptyState;
use crate::state::AppState;
use crate::utils::format_email_timestamp;
use crate::Route;

#[component]
pub fn Dashboard() -> Element {
    let storage = use_context::<Storage>();
    let state = use_context::<AppState>();

    // 1. Account name lookup
    let account_names: std::collections::HashMap<Uuid, String> = (state.accounts)()
        .into_iter()
        .map(|acc| {
            let name = acc.display_name.unwrap_or(acc.email_address);
            (acc.id, name)
        })
        .collect();

    // 2. Fetch Mailbox Stats from database
    let stats_resource = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let selected = (state.selected_account)();
            let _ = (state.sync_tick)();
            let _ = (state.refresh_trigger)();
            async move {
                storage.get_mailbox_stats(selected).await
            }
        }
    });

    // 3. Fetch ONLY the top 5 recent emails for the preview widget
    let recent_emails = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let selected = (state.selected_account)();
            let _ = (state.sync_tick)();
            let _ = (state.refresh_trigger)();
            async move {
                match selected {
                    Some(account_id) => storage.list_inbox(account_id, 5).await,
                    None => storage.list_inbox_all(5).await,
                }
            }
        }
    });

    // 4. Fetch ONLY the top 5 recent findings for the preview widget
    let recent_findings = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let selected = (state.selected_account)();
            let _ = (state.sync_tick)();
            let _ = (state.refresh_trigger)();
            async move {
                match selected {
                    Some(account_id) => storage.list_criteria_for_account(account_id, 5).await,
                    None => storage.list_criteria_for_all(5).await,
                }
            }
        }
    });

    // 5. Fetch watch criteria
    let all_criteria = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let _ = (state.refresh_trigger)();
            async move { storage.list_criteria().await }
        }
    });

    // Extract exact counts from the database query
    let mailbox_stats = stats_resource.read().as_ref().and_then(|r| r.as_ref().ok()).copied().unwrap_or_default();
    let emails = recent_emails.read().as_ref().and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();
    let findings = recent_findings.read().as_ref().and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();
    let criteria = all_criteria.read().as_ref().and_then(|r| r.as_ref().ok()).cloned().unwrap_or_default();

    let unread_count = mailbox_stats.unread_count as usize;
    let findings_total = mailbox_stats.findings_count as usize;
    let total_count = mailbox_stats.total_count as usize;
    let active_criteria_count = criteria.iter().filter(|c| c.is_active).count();

    rsx! {
        div { class: "dashboard",
            div { class: "dashboard__header",
                h1 { class: "dashboard__title", "Dashboard" }
                p { class: "dashboard__subtitle", "Your email and AI findings overview" }
            }

            // Stat Cards
            div { class: "dashboard__stats",
                StatCard { icon: "📨", label: "Unread", value: unread_count }
                StatCard {
                    icon: "🏷️",
                    label: "Findings",
                    value: findings_total,
                    is_live: true,
                }
                StatCard { icon: "📬", label: "Total", value: total_count }
            }

            // 2-Column Side-by-Side Grid
            div { class: "dashboard__two-col",

                // 1. Left Column: Recent Emails
                div { class: "dashboard__panel",
                    div { class: "dashboard__panel-header",
                        div { class: "dashboard__panel-header-left",
                            span { class: "dashboard__panel-title", "Recent Emails" }
                            span { class: "dashboard__badge", "{unread_count} unread" }
                        }
                        Link {
                            to: Route::Inbox {},
                            class: "dashboard__button dashboard__button--secondary",
                            "View all"
                        }
                    }

                    div { class: "dashboard__panel-body",
                        if emails.is_empty() {
                            EmptyState { icon: "📭", title: "No emails yet" }
                        } else {
                            div { class: "dashboard__emails-list",
                                for email in &emails {
                                    EmailRowView {
                                        key: "{email.id}",
                                        email_id: email.id,
                                        account_id: email.account_id,
                                        subject: email.subject.clone().unwrap_or_else(|| "(No subject)".to_string()),
                                        sender: email.sender.clone().unwrap_or_else(|| "(Unknown sender)".to_string()),
                                        account: account_names
                                            .get(&email.account_id)
                                            .cloned()
                                            .unwrap_or_else(|| "Unknown Account".to_string()),
                                        time: format_email_timestamp(email.received_at),
                                        unread: !email.is_read,
                                        app_has_viewed: email.app_has_viewed,
                                    }
                                }
                            }
                        }
                    }
                }

                // 2. Right Column: Recent Findings
                div { class: "dashboard__panel dashboard__panel--findings",
                    div { class: "dashboard__panel-header",
                        div { class: "dashboard__panel-header-left",
                            span { class: "dashboard__panel-title", "Recent Findings" }
                            span { class: "dashboard__badge dashboard__badge--live",
                                "{findings_total} total"
                            }
                        }
                        Link {
                            to: Route::Findings {},
                            class: "dashboard__button dashboard__button--secondary",
                            "View all"
                        }
                    }

                    div { class: "dashboard__panel-body",
                        if findings.is_empty() {
                            EmptyState {
                                icon: "🔍",
                                title: "No findings yet",
                                subtitle: "Matched emails from your watch criteria will show up here.",
                            }
                        } else {
                            div { class: "dashboard__findings-list",
                                for finding in &findings {
                                    RecentFindingRow {
                                        key: "{finding.email_id}:{finding.criterion_id}",
                                        account_name: account_names
                                            .get(&finding.account_id)
                                            .cloned()
                                            .unwrap_or_else(|| "Unknown Account".to_string()),
                                        finding: finding.clone(),
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Bottom Full-Width Panel: Watch Criteria
            div { class: "dashboard__panel dashboard__panel--criteria",
                div { class: "dashboard__panel-header",
                    div { class: "dashboard__panel-header-left",
                        span { class: "dashboard__panel-title", "Watch Criteria" }
                        span { class: "dashboard__badge dashboard__badge--criteria",
                            "{active_criteria_count} active"
                        }
                    }
                    Link {
                        to: Route::Settings {},
                        class: "dashboard__button dashboard__button--secondary",
                        "Manage in settings"
                    }
                }

                div { class: "dashboard__panel-body",
                    if criteria.is_empty() {
                        p { class: "dashboard__criteria-list-empty empty-state__hint",
                            "No watch criteria yet, add some in Settings."
                        }
                    } else {
                        div { class: "dashboard__criteria-list",
                            for criterion in criteria {
                                CriteriaItem {
                                    key: "{criterion.id}",
                                    id: criterion.id,
                                    label: criterion.label.clone(),
                                    description: criterion.description.clone(),
                                    active: criterion.is_active,
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn StatCard(
    icon: &'static str,
    label: &'static str,
    value: usize,
    #[props(default)] is_live: bool,
) -> Element {
    let card_class = if is_live && value > 0 {
        "dashboard__stat-card dashboard__stat-card--live"
    } else {
        "dashboard__stat-card"
    };

    rsx! {
        div { class: "{card_class}",
            div { class: "dashboard__stat-icon", "{icon}" }
            div { class: "dashboard__stat-info",
                div { class: "dashboard__stat-value", "{value}" }
                div { class: "dashboard__stat-label", "{label}" }
            }
        }
    }
}

#[component]
fn EmailRowView(
    email_id: Uuid,
    account_id: Uuid,
    subject: String,
    sender: String,
    account: String,
    time: String,
    unread: bool,
    app_has_viewed: bool,
) -> Element {
    let nav = use_navigator();
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();
    let is_viewed = !unread || app_has_viewed;

    let trash = move |evt: Event<MouseData>| {
        evt.stop_propagation();
        let storage = storage.clone();
        spawn(async move {
            if let Err(e) = storage.trash_email(email_id).await {
                tracing::error!("failed to trash email {email_id}: {e}");
            } else {
                state.refresh_trigger.with_mut(|n| *n += 1);
            }
        });
    };

    let row_class = if is_viewed {
        "dashboard__card-item dashboard__card-item--viewed"
    } else {
        "dashboard__card-item dashboard__card-item--unread"
    };

    rsx! {
        div {
            class: "{row_class}",
            onclick: move |_| {
                state.selected_email.set(Some(email_id));
                state.selected_account.set(Some(account_id));
                nav.push(Route::Inbox {});
            },

            // 1. Top Bar: Account Badge on left, Time on right (Matches Findings top bar height)
            div { class: "dashboard__card-top",
                div { class: "dashboard__finding-badge-wrap",
                    span { class: "dashboard__account-badge", "{account}" }
                }
                span { class: "dashboard__card-time", "{time}" }
            }

            // 2. Middle Row: Subject on left, Delete on right
            div { class: "dashboard__card-main",
                span { class: "dashboard__card-subject", "{subject}" }
                button {
                    class: "dashboard__email-trash-btn",
                    onclick: trash,
                    title: "Move to Trash",
                    "🗑"
                }
            }

            // 3. Bottom Row: Sender & Account Meta (Matches Findings meta layout)
            div { class: "dashboard__card-meta",
                span { class: "dashboard__card-sender", "{sender}" }
                span { class: "dashboard__card-account-muted", "" }
            }
        }
    }
}

#[component]
fn RecentFindingRow(
    finding: MatchedEmailWithReason,
    account_name: String,
) -> Element {
    let nav = use_navigator();
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    let email_id = finding.email_id;
    let account_id = finding.account_id;
    let subject = finding.subject.as_deref().unwrap_or("(No subject)");
    let sender = finding.sender.as_deref().unwrap_or("(Unknown sender)");
    let confidence_pct = finding.confidence.map(|c| (c * 100.0).round() as i32);
    let is_viewed = finding.is_read || finding.app_has_viewed;

    let trash = move |evt: Event<MouseData>| {
        evt.stop_propagation();
        let storage = storage.clone();
        spawn(async move {
            if let Err(e) = storage.trash_email(email_id).await {
                tracing::error!("failed to trash finding email {email_id}: {e}");
            } else {
                state.refresh_trigger.with_mut(|n| *n += 1);
            }
        });
    };

    let row_class = if is_viewed {
        "dashboard__card-item dashboard__card-item--viewed"
    } else {
        "dashboard__card-item dashboard__card-item--unread"
    };

    rsx! {
        div {
            class: "{row_class}",
            onclick: move |_| {
                state.selected_email.set(Some(email_id));
                state.selected_account.set(Some(account_id));
                nav.push(Route::Findings {});
            },

            // 1. Top Bar: Criterion Badge on left, Time on right
            div { class: "dashboard__card-top",
                div { class: "dashboard__finding-badge-wrap",
                    span { class: "dashboard__criterion-badge", "{finding.criterion_label}" }
                    if let Some(pct) = confidence_pct {
                        span { class: "dashboard__confidence-badge", "{pct}%" }
                    }
                }
                span { class: "dashboard__card-time", "{format_email_timestamp(finding.received_at)}" }
            }

            // 2. Middle Row: Subject on left, Delete on right
            div { class: "dashboard__card-main",
                span { class: "dashboard__card-subject", "{subject}" }
                button {
                    class: "dashboard__email-trash-btn",
                    onclick: trash,
                    title: "Move to Trash",
                    "🗑"
                }
            }

            // 3. Bottom Row: Sender & Account Meta
            div { class: "dashboard__card-meta",
                span { class: "dashboard__card-sender", "{sender}" }
                span { class: "dashboard__card-account-muted", "{account_name}" }
            }
        }
    }
}

#[component]
fn CriteriaItem(id: Uuid, label: String, description: String, active: bool) -> Element {
    let mut is_active = use_signal(|| active);
    let storage = use_context::<Storage>();

    let toggle = move |_| {
        let new_value = !is_active();
        is_active.set(new_value);

        let storage = storage.clone();
        spawn(async move {
            if let Err(e) = storage.set_criterion_active(id, new_value).await {
                tracing::error!("failed to update criterion {id}: {e}");
                is_active.set(!new_value);
            }
        });
    };

    rsx! {
        div { class: "dashboard__criteria-item",
            div { class: "dashboard__criteria-left",
                div {
                    class: if is_active() { "dashboard__toggle-switch dashboard__toggle-switch--active" } else { "dashboard__toggle-switch" },
                    onclick: toggle,
                    div { class: "dashboard__toggle-slider" }
                }
                div { class: "dashboard__criteria-info",
                    div { class: "dashboard__criteria-label", "{label}" }
                    div { class: "dashboard__criteria-description", "{description}" }
                }
            }
            div { class: "dashboard__criteria-status",
                if is_active() {
                    span { class: "dashboard__status-badge dashboard__status-badge--active",
                        "Active"
                    }
                } else {
                    span { class: "dashboard__status-badge dashboard__status-badge--inactive",
                        "Disabled"
                    }
                }
            }
        }
    }
}
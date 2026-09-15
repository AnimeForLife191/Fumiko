//! Main overview dashboard displaying mailbox statistics, recent findings, and active criteria toggles.

use dioxus::prelude::*;
use email_core::SyncPhase;
use storage::Storage;
use storage::models::MatchedEmailWithReason;
use uuid::Uuid;

use super::component::EmptyState;
use crate::Route;
use crate::state::AppState;
use crate::utils::format_email_timestamp;

/// Active tab selection for the dashboard preview panel.
#[derive(Clone, Copy, PartialEq)]
enum DashboardTab {
    Findings,
    RecentEmails,
}

#[component]
pub fn Dashboard() -> Element {
    let storage = use_context::<Storage>();
    let state = use_context::<AppState>();
    let mut active_tab = use_signal(|| DashboardTab::Findings);

    let account_names: std::collections::HashMap<Uuid, String> = (state.accounts)()
        .into_iter()
        .map(|acc| {
            let name = acc.display_name.unwrap_or(acc.email_address);
            (acc.id, name)
        })
        .collect();

    // Reactive Subscriptions:
    // Subscribing to sync_tick and refresh_trigger ensures that mailbox counts,
    // recent email lists, and findings refresh immediately following sync passes.
    let stats_resource = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let selected = (state.selected_account)();
            let _ = (state.sync_tick)();
            let _ = (state.refresh_trigger)();
            async move { storage.get_mailbox_stats(selected).await }
        }
    });

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

    let all_criteria = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let _ = (state.refresh_trigger)();
            async move { storage.list_criteria().await }
        }
    });

    let mailbox_stats = stats_resource
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .copied()
        .unwrap_or_default();
    let emails = recent_emails
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let findings = recent_findings
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let criteria = all_criteria
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    let unread_count = mailbox_stats.unread_count as usize;
    let findings_total = mailbox_stats.findings_count as usize;
    let total_count = mailbox_stats.total_count as usize;
    let active_criteria_count = criteria.iter().filter(|c| c.is_active).count();
    let findings_count = findings.len();
    let emails_count = emails.len();

    let subtitle_text = if findings_total > 0 {
        let plural = if findings_total == 1 { "" } else { "s" };
        format!(
            "Fumiko is watching: {findings_total} priority item{plural} flagged for your review."
        )
    } else {
        "All quiet. Fumiko hasn't detected any urgent emails matching your watch rules.".to_string()
    };

    rsx! {
        div { class: "dashboard",
            div { class: "dashboard__hero",
                div { class: "dashboard__hero-header",
                    div { class: "dashboard__hero-content",
                        div { class: "dashboard__guardian-badge",
                            span { class: if (state.is_syncing)() { "dashboard__guardian-dot dashboard__guardian-dot--pulse" } else { "dashboard__guardian-dot" } }
                            span {
                                if (state.is_syncing)() {
                                    "Scanning now..."
                                } else {
                                    "Watching quietly"
                                }
                            }
                        }
                        h1 { class: "dashboard__title", "Peace of mind for your inbox." }
                        p { class: "dashboard__subtitle", "{subtitle_text}" }
                    }

                    if (state.is_syncing)() {
                        div { class: "dashboard__sync-badge",
                            span { class: "dashboard__sync-dot" }
                            if let Some(progress) = (state.sync_progress)() {
                                match progress.phase {
                                    SyncPhase::Hydrating => rsx! {
                                        span { "Fumiko fetching headers ({progress.current}/{progress.total})..." }
                                    },
                                    SyncPhase::Classifying => rsx! {
                                        span { "Fumiko evaluating ({progress.current}/{progress.total})..." }
                                    },
                                }
                            } else {
                                span { "Fumiko checking inboxes..." }
                            }
                        }
                    }
                }

                div { class: "dashboard__stats-bar",
                    div { class: "dashboard__stat-pill",
                        div { class: "dashboard__stat-icon",
                            span { class: "icon icon--mail", aria_hidden: "true" }
                        }
                        div { class: "dashboard__stat-text",
                            span { class: "dashboard__stat-num", "{unread_count}" }
                            span { class: "dashboard__stat-name", "Unread" }
                        }
                    }
                    div { class: if findings_total > 0 { "dashboard__stat-pill dashboard__stat-pill--highlight" } else { "dashboard__stat-pill" },
                        div { class: "dashboard__stat-icon",
                            span {
                                class: "icon icon--search",
                                aria_hidden: "true",
                            }
                        }
                        div { class: "dashboard__stat-text",
                            span { class: "dashboard__stat-num", "{findings_total}" }
                            span { class: "dashboard__stat-name", "Findings" }
                        }
                    }
                    div { class: "dashboard__stat-pill",
                        div { class: "dashboard__stat-icon",
                            span {
                                class: "icon icon--mails",
                                aria_hidden: "true",
                            }
                        }
                        div { class: "dashboard__stat-text",
                            span { class: "dashboard__stat-num", "{total_count}" }
                            span { class: "dashboard__stat-name", "Total Monitored" }
                        }
                    }
                }
            }

            div { class: "dashboard__panel",
                div { class: "dashboard__panel-header",
                    div { class: "dashboard__segmented-tabs",
                        div { class: if active_tab() == DashboardTab::Findings { "dashboard__tab-slider dashboard__tab-slider--left" } else { "dashboard__tab-slider dashboard__tab-slider--right" } }

                        button {
                            class: if active_tab() == DashboardTab::Findings { "dashboard__tab dashboard__tab--active" } else { "dashboard__tab" },
                            onclick: move |_| active_tab.set(DashboardTab::Findings),
                            span {
                                class: "icon icon--search",
                                aria_hidden: "true",
                            }
                            "Priority Findings"
                            span { class: "dashboard__tab-badge", "{findings_count}" }
                        }
                        button {
                            class: if active_tab() == DashboardTab::RecentEmails { "dashboard__tab dashboard__tab--active" } else { "dashboard__tab" },
                            onclick: move |_| active_tab.set(DashboardTab::RecentEmails),
                            span { class: "icon icon--mail", aria_hidden: "true" }
                            "Recent Inbox"
                            span { class: "dashboard__tab-badge", "{emails_count}" }
                        }
                    }

                    if active_tab() == DashboardTab::Findings {
                        Link {
                            to: Route::Findings {},
                            class: "dashboard__text-action",
                            "Open Findings Board"
                        }
                    } else {
                        Link {
                            to: Route::Inbox {},
                            class: "dashboard__text-action",
                            "Open Full Inbox"
                        }
                    }
                }

                div { class: "dashboard__panel-body",
                    match active_tab() {
                        DashboardTab::Findings => rsx! {
                            if findings.is_empty() {
                                EmptyState {
                                    icon_class: "icon icon--empty",
                                    title: "No flagged emails right now",
                                    subtitle: "When Fumiko finds emails matching your watch criteria, they will be highlighted here.",
                                }
                            } else {
                                div { class: "dashboard__stream-list",
                                    for finding in &findings {
                                        FindingCard {
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
                        },
                        DashboardTab::RecentEmails => rsx! {
                            if emails.is_empty() {
                                EmptyState {
                                    icon_class: "icon icon--empty",
                                    title: "Inbox is quiet",
                                    subtitle: "No recent messages discovered.",
                                }
                            } else {
                                div { class: "dashboard__stream-list",
                                    for email in &emails {
                                        EmailCard {
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
                        },
                    }
                }
            }

            div { class: "dashboard__panel dashboard__panel--criteria",
                div { class: "dashboard__panel-header",
                    div { class: "dashboard__panel-header-left",
                        span { class: "dashboard__panel-title", "Active Watch Rules" }
                        span { class: "dashboard__badge dashboard__badge--criteria",
                            "{active_criteria_count} Active"
                        }
                    }
                    Link {
                        to: Route::Settings {},
                        class: "dashboard__text-action",
                        "Customize Rules"
                    }
                }

                div { class: "dashboard__panel-body",
                    if criteria.is_empty() {
                        p { class: "dashboard__criteria-list-empty empty-state__hint",
                            "No watch criteria configured yet. Add your first rule in Settings."
                        }
                    } else {
                        div { class: "dashboard__criteria-grid",
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

/// Compact preview card for rule-matching findings displayed on the dashboard.
#[component]
fn FindingCard(finding: MatchedEmailWithReason, account_name: String) -> Element {
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

    let card_class = if is_viewed {
        "dashboard__card dashboard__card--viewed"
    } else {
        "dashboard__card dashboard__card--unread"
    };

    rsx! {
        div {
            class: "{card_class}",
            onclick: move |_| {
                state.selected_email.set(Some(email_id));
                if (state.selected_account)().is_some() {
                    state.selected_account.set(Some(account_id));
                }
                nav.push(Route::Findings {});
            },

            div { class: "dashboard__card-left",
                div { class: "dashboard__card-tags",
                    span { class: "dashboard__tag dashboard__tag--coral", "{finding.criterion_label}" }
                    if let Some(pct) = confidence_pct {
                        span { class: "dashboard__tag dashboard__tag--confidence", "{pct}% Match" }
                    }
                    span { class: "dashboard__tag dashboard__tag--account", "{account_name}" }
                }

                div { class: "dashboard__card-subject", "{subject}" }
                div { class: "dashboard__card-sender", "{sender}" }
            }

            div { class: "dashboard__card-right",
                span { class: "dashboard__card-time", "{format_email_timestamp(finding.received_at)}" }
                button {
                    class: "dashboard__action-btn",
                    onclick: trash,
                    title: "Move to Trash",
                    span { class: "icon icon--trash", aria_hidden: "true" }
                }
            }
        }
    }
}

/// Compact preview card for recent messages displayed on the dashboard.
#[component]
fn EmailCard(
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

    let is_unread = unread && !app_has_viewed;

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

    let card_class = if is_unread {
        "dashboard__card dashboard__card--unread"
    } else {
        "dashboard__card dashboard__card--viewed"
    };

    rsx! {
        div {
            class: "{card_class}",
            onclick: move |_| {
                state.selected_email.set(Some(email_id));
                if (state.selected_account)().is_some() {
                    state.selected_account.set(Some(account_id));
                }
                nav.push(Route::Inbox {});
            },

            div { class: "dashboard__card-left",
                div { class: "dashboard__card-tags",
                    span { class: "dashboard__tag dashboard__tag--account", "{account}" }
                    if is_unread {
                        span { class: "dashboard__tag dashboard__tag--unread", "New" }
                    }
                }

                div { class: "dashboard__card-subject", "{subject}" }
                div { class: "dashboard__card-sender", "{sender}" }
            }

            div { class: "dashboard__card-right",
                span { class: "dashboard__card-time", "{time}" }
                button {
                    class: "dashboard__action-btn",
                    onclick: trash,
                    title: "Move to Trash",
                    span { class: "icon icon--trash", aria_hidden: "true" }
                }
            }
        }
    }
}

/// Interactive toggle item for enabling or disabling individual watch criteria directly from the dashboard.
#[component]
fn CriteriaItem(id: Uuid, label: String, description: String, active: bool) -> Element {
    let mut is_active = use_signal(|| active);
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    let toggle = move |_| {
        let new_value = !is_active();
        is_active.set(new_value);

        let storage = storage.clone();
        spawn(async move {
            match storage.set_criterion_active(id, new_value).await {
                Ok(()) => {
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
                Err(e) => {
                    tracing::error!("failed to update criterion {id}: {e}");
                    is_active.set(!new_value);
                }
            }
        });
    };

    rsx! {
        div { class: "dashboard__criteria-card",
            div { class: "dashboard__criteria-header",
                span { class: "dashboard__criteria-name", "{label}" }
                div {
                    class: if is_active() { "dashboard__toggle-switch dashboard__toggle-switch--active" } else { "dashboard__toggle-switch" },
                    onclick: toggle,
                    div { class: "dashboard__toggle-slider" }
                }
            }
            p { class: "dashboard__criteria-desc", "{description}" }
        }
    }
}
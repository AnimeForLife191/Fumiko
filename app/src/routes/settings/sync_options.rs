//! Configuration for initial mailbox synchronization limits.

use common::{config, setting_keys};
use dioxus::prelude::*;
use storage::Storage;

/// Supported preset options for initial email fetch counts.
const PULL_LIMIT_OPTIONS: &[u32] = &[10, 25, 50, 100, 200];

#[component]
pub fn InitialSyncSection() -> Element {
    let storage = use_context::<Storage>();
    let mut sync_limit = use_signal(|| config::DEFAULT_INITIAL_SYNC_LIMIT);

    use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                if let Ok(Some(val)) = storage.get_setting(setting_keys::INITIAL_SYNC_LIMIT).await {
                    if let Ok(num) = val.parse::<u32>() {
                        sync_limit.set(num.clamp(10, config::MAX_INITIAL_SYNC_LIMIT));
                    }
                }
            }
        }
    });

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                h2 { "Initial Email Fetch" }
            }
            div { class: "settings__section-content",
                p { class: "settings__description",
                    "Choose how many recent emails Fumiko retrieves when linking an account for the first time."
                }

                div { class: "settings__control-group",
                    label { class: "settings__select-label", "Initial Pull Limit" }

                    div { class: "settings__pill-group",
                        for & opt in PULL_LIMIT_OPTIONS {
                            button {
                                key: "{opt}",
                                class: if (sync_limit)() == opt { "settings__pill-btn settings__pill-btn--active" } else { "settings__pill-btn" },
                                onclick: {
                                    let storage = storage.clone();
                                    move |_| select_limit(opt, sync_limit, storage.clone())
                                },
                                "{opt}"
                            }
                        }
                    }

                    span { class: "settings__unit-tag", "emails" }
                }

                span { class: "settings__status-hint", "Capped at 200 to prevent provider rate-limits" }
            }
        }
    }
}

fn select_limit(opt: u32, mut sync_limit: Signal<u32>, storage: Storage) {
    sync_limit.set(opt);
    spawn(async move {
        let _ = storage
            .set_setting(setting_keys::INITIAL_SYNC_LIMIT, &opt.to_string())
            .await;
    });
}
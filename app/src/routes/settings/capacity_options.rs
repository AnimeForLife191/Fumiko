//! Configuration controls for inbox and findings display and storage capacities.

use dioxus::prelude::*;
use common::{config, setting_keys};
use storage::Storage;

const INBOX_PRESETS: &[u32] = &[100, 200, 400, 600, 1000];
const FINDINGS_PRESETS: &[u32] = &[100, 200, 400, 600, 1000];

#[component]
pub fn MailboxCapicitySection() -> Element {
    let storage = use_context::<Storage>();

    let mut inbox_capacity = use_signal(|| config::DEFAULT_INBOX_CAPACITY);
    let mut findings_capacity = use_signal(|| config::DEFAULT_FINDINGS_CAPACITY);

    use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                if let Ok(Some(value)) = storage.get_setting(setting_keys::INBOX_CAPACITY).await {
                    if let Ok(num) = value.parse::<u32>() {
                        inbox_capacity.set(num.clamp(config::MIN_INBOX_CAPACITY, config::MAX_INBOX_CAPACITY));
                    }
                }
                if let Ok(Some(value)) = storage.get_setting(setting_keys::FINDINGS_CAPACITY).await {
                    if let Ok(num) = value.parse::<u32>() {
                        findings_capacity.set(num.clamp(config::MIN_FINDINGS_CAPACITY, config::MAX_FINDINGS_CAPACITY));
                    }
                }
            }
        }
    });

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                h2 { "Mailbox & Findings Capacity" }
            }
            div { class: "settings__section-content",
                p { class: "settings__description",
                    "Configure how many emails are stored and displayed locally. When limits are exceeded, older messages are automatically purged from local storage to keep SQLite lean."
                }

                // Inbox Capacity Row
                div { class: "settings__control-group",
                    label { class: "settings__select-label", "Inbox Capacity" }
                    div { class: "settings__pill-group",
                        for & opt in INBOX_PRESETS {
                            button {
                                key: "inbox-{opt}",
                                class: if (inbox_capacity)() == opt { "settings__pill-btn settings__pill-btn--active" } else { "settings__pill-btn" },
                                onclick: {
                                    let storage = storage.clone();
                                    move |_| {
                                        inbox_capacity.set(opt);
                                        let storage = storage.clone();
                                        spawn(async move {
                                            let _ = storage
                                                .set_setting(setting_keys::INBOX_CAPACITY, &opt.to_string())
                                                .await;
                                        });
                                    }
                                },
                                "{opt}"
                            }
                        }
                    }
                    span { class: "settings__unit-tag", "emails per account" }
                }

                // Findings Capacity Row
                div { class: "settings__control-group",
                    label { class: "settings__select-label", "Findings Capacity" }
                    div { class: "settings__pill-group",
                        for & opt in FINDINGS_PRESETS {
                            button {
                                key: "findings-{opt}",
                                class: if (findings_capacity)() == opt { "settings__pill-btn settings__pill-btn--active" } else { "settings__pill-btn" },
                                onclick: {
                                    let storage = storage.clone();
                                    move |_| {
                                        findings_capacity.set(opt);
                                        let storage = storage.clone();
                                        spawn(async move {
                                            let _ = storage
                                                .set_setting(setting_keys::FINDINGS_CAPACITY, &opt.to_string())
                                                .await;
                                        });
                                    }
                                },
                                "{opt}"
                            }
                        }
                    }
                    span { class: "settings__unit-tag", "flagged findings" }
                }
            }
        }
    }
}
//! Configuration for automated trash retention intervals and immediate purge triggers.

use crate::AppState;
use common::setting_keys;
use dioxus::prelude::*;
use storage::Storage;

/// Preset intervals for automatic trash purging.
const QUICK_RETENTION_PRESETS: &[(u32, &str)] = &[
    (0, "Instant (0d)"),
    (7, "7d"),
    (14, "14d"),
    (30, "30d"),
    (90, "90d"),
];

#[component]
pub fn TrashRetentionSection() -> Element {
    let storage = use_context::<Storage>();
    let state = use_context::<AppState>();

    let mut trash_retention_days = use_signal(|| "30".to_string());

    use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                if let Ok(Some(days)) = storage
                    .get_setting(setting_keys::TRASH_RETENTION_DAYS)
                    .await
                {
                    trash_retention_days.set(days);
                }
            }
        }
    });

    let current_days = trash_retention_days.read().parse::<u32>().unwrap_or(30);
    let is_instant = current_days == 0;

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                h2 { "Storage and Retention" }
            }
            div { class: "settings__section-content",
                p { class: "settings__description",
                    "Configure how long trashed emails are preserved before permanent deletion."
                }

                div { class: "settings__control-group",
                    label { class: "settings__select-label", "Retention Period" }

                    div { class: "settings__stepper",
                        button {
                            class: "settings__stepper-btn",
                            disabled: current_days == 0,
                            onclick: {
                                let storage = storage.clone();
                                move |_| {
                                    if current_days > 0 {
                                        save_retention_days(
                                            current_days - 1,
                                            trash_retention_days,
                                            state,
                                            storage.clone(),
                                        );
                                    }
                                }
                            },
                            "−"
                        }
                        input {
                            class: "settings__stepper-input",
                            r#type: "number",
                            min: "0",
                            max: "365",
                            value: "{trash_retention_days}",
                            oninput: {
                                let storage = storage.clone();
                                move |evt: Event<FormData>| {
                                    let val = evt.value();
                                    trash_retention_days.set(val.clone());
                                    if let Ok(num) = val.parse::<u32>() {
                                        save_retention_days(num, trash_retention_days, state, storage.clone());
                                    }
                                }
                            },
                        }
                        button {
                            class: "settings__stepper-btn",
                            disabled: current_days >= 365,
                            onclick: {
                                let storage = storage.clone();
                                move |_| {
                                    if current_days < 365 {
                                        save_retention_days(
                                            current_days + 1,
                                            trash_retention_days,
                                            state,
                                            storage.clone(),
                                        );
                                    }
                                }
                            },
                            "+"
                        }
                    }

                    span { class: "settings__unit-tag",
                        if is_instant {
                            "Instant deletion"
                        } else {
                            "days"
                        }
                    }
                }

                div { class: "settings__chips-row",
                    span { class: "settings__chips-label", "Presets:" }
                    for & (preset_days , label) in QUICK_RETENTION_PRESETS {
                        button {
                            key: "{preset_days}",
                            class: if current_days == preset_days { "settings__chip-btn settings__chip-btn--active" } else { "settings__chip-btn" },
                            onclick: {
                                let storage = storage.clone();
                                move |_| save_retention_days(
                                    preset_days,
                                    trash_retention_days,
                                    state,
                                    storage.clone(),
                                )
                            },
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}

fn save_retention_days(
    days: u32,
    mut trash_retention_days: Signal<String>,
    mut state: AppState,
    storage: Storage,
) {
    let clamped = days.min(365);
    trash_retention_days.set(clamped.to_string());
    spawn(async move {
        let _ = storage
            .set_setting(setting_keys::TRASH_RETENTION_DAYS, &clamped.to_string())
            .await;
        let _ = storage.purge_expired_trash(clamped as i64).await;
        state.refresh_trigger.with_mut(|n| *n += 1);
    });
}
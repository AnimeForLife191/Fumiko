//! Complete application reset, database truncation, and keyring credential wiping.

use crate::{AppState, Route, state::AiCommand};
use dioxus::prelude::*;
use local_ai::builtin::default_models_dir;
use storage::Storage;

#[component]
pub fn DangerZoneSection() -> Element {
    let nav = use_navigator();
    let mut state = use_context::<AppState>();
    let storage = use_context::<Storage>();

    let mut confirming_wipe = use_signal(|| false);
    let mut is_wiping = use_signal(|| false);
    let mut delete_models = use_signal(|| false);
    let mut wipe_error = use_signal(|| None::<String>);

    // Total Application Wipe:
    // 1. Terminate running local AI processes to release file locks and RAM.
    // 2. Optionally delete model weights to reclaim disk space.
    // 3. Purge OS Keyring secrets and truncate SQLite tables inside an atomic transaction.
    let wipe_all_data_action = {
        let storage = storage.clone();
        let nav = nav.clone();
        move |_| {
            let storage = storage.clone();
            let nav = nav.clone();
            let should_delete_models = delete_models();
            is_wiping.set(true);
            wipe_error.set(None);

            spawn(async move {
                let _ = (state.ai_tx)().send(AiCommand::StopBuiltin);

                if should_delete_models {
                    let models_dir = default_models_dir();
                    if let Ok(entries) = std::fs::read_dir(models_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.extension().and_then(|ext| ext.to_str()) == Some("gguf") {
                                let _ = std::fs::remove_file(path);
                            }
                        }
                    }
                }

                match storage.wipe_all_data().await {
                    Ok(()) => {
                        state.selected_account.set(None);
                        state.selected_email.set(None);
                        state.accounts.set(Vec::new());
                        state.refresh_trigger.with_mut(|n| *n += 1);
                        is_wiping.set(false);
                        confirming_wipe.set(false);
                        nav.push(Route::Dashboard {});
                    }
                    Err(e) => {
                        tracing::error!("failed to wipe application data: {e}");
                        wipe_error.set(Some(format!("Failed to wipe data: {e}")));
                        is_wiping.set(false);
                    }
                }
            });
        }
    };

    rsx! {
        div { class: "settings__section settings__section--danger",
            div { class: "settings__section-header",
                h2 { "Danger Zone" }
            }
            div { class: "settings__section-content",
                p { class: "settings__description",
                    "Permanently delete all synced emails, watch criteria, stored settings, and keyring credentials from this device."
                }

                if confirming_wipe() {
                    div { class: "settings__confirm-box",
                        p { class: "settings__confirm-warning",
                            "Are you sure? This action cannot be undone."
                        }

                        label { class: "settings__checkbox-label",
                            input {
                                r#type: "checkbox",
                                checked: delete_models(),
                                onchange: move |evt| delete_models.set(evt.value().parse().unwrap_or(false)),
                            }
                            "Also delete downloaded Built-in AI models to free disk space"
                        }

                        div { class: "settings__confirm-actions",
                            button {
                                class: "settings__button settings__button--danger",
                                disabled: is_wiping(),
                                onclick: wipe_all_data_action,
                                if is_wiping() {
                                    "Wiping data..."
                                } else {
                                    "Yes, delete all data"
                                }
                            }
                            button {
                                class: "settings__button settings__button--secondary",
                                disabled: is_wiping(),
                                onclick: move |_| confirming_wipe.set(false),
                                "Cancel"
                            }
                        }
                    }
                } else {
                    button {
                        class: "settings__button settings__button--danger",
                        onclick: move |_| confirming_wipe.set(true),
                        "Wipe All Local Data"
                    }
                }

                if let Some(err) = wipe_error() {
                    p { class: "settings__error", "{err}" }
                }
            }
        }
    }
}
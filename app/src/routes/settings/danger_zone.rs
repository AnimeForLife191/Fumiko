use dioxus::prelude::*;
use crate::{Route, AppState};
use storage::Storage;

#[component]
pub fn DangerZoneSection() -> Element {
    let nav = use_navigator();
    let mut state = use_context::<AppState>();
    let storage = use_context::<Storage>();

    let mut confirming_wipe = use_signal(|| false);
    let mut is_wiping = use_signal(|| false);
    let mut wipe_error = use_signal(|| None::<String>);

    let wipe_all_data_action = {
        let storage = storage.clone();
        let nav = nav.clone();
        move |_| {
            let storage = storage.clone();
            let nav = nav.clone();
            is_wiping.set(true);
            wipe_error.set(None);

            spawn(async move {
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
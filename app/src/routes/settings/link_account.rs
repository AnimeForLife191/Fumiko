use dioxus::prelude::*;
use crate::{AppState, Route};
use storage::Storage;
use uuid::Uuid;

#[component]
pub fn LinkedAccountsSection() -> Element {
    let nav = use_navigator();
    let mut state = use_context::<AppState>();
    let storage = use_context::<Storage>();

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                h2 { "Linked Accounts" }
            }
            div { class: "settings__section-content",
                for account in (state.accounts)() {
                    div { class: "settings__account-row", key: "{account.id}",
                        EditableAccountName {
                            account_id: account.id,
                            current_name: account.display_name.clone().unwrap_or_else(|| account.email_address.clone()),
                            email_address: account.email_address.clone(),
                            has_custom_name: account.display_name.is_some(),
                        }
                        if let Some(sync_err) = &account.last_sync_error {
                            span {
                                class: "settings__account-status settings__account-status--error",
                                title: "{sync_err}",
                                "Sync Error"
                            }
                        } else {
                            span { class: "settings__account-status settings__account-status--connected",
                                "Connected"
                            }
                        }
                        button {
                            class: "settings__button settings__button--remove",
                            onclick: {
                                let account_id = account.id;
                                let storage = storage.clone();
                                move |_| {
                                    let storage = storage.clone();
                                    spawn(async move {
                                        match storage.delete_account(account_id).await {
                                            Ok(()) => {
                                                if (state.selected_account)() == Some(account_id) {
                                                    state.selected_account.set(None);
                                                }
                                                state.refresh_trigger.with_mut(|n| *n += 1);
                                            }
                                            Err(e) => {
                                                tracing::error!("Failed to delete account {account_id}: {e:?}");
                                            }
                                        }
                                    });
                                }
                            },
                            "Remove"
                        }
                    }
                }
                button {
                    class: "settings__button settings__button--add",
                    onclick: move |_| {
                        nav.push(Route::AddAccount {});
                    },
                    "+ Add Account"
                }
            }
        }
    }
}

#[component]
fn EditableAccountName(
    account_id: Uuid,
    current_name: String,
    email_address: String,
    has_custom_name: bool,
) -> Element {
    let storage = use_context::<Storage>();
    let state = use_context::<AppState>();

    let mut is_editing = use_signal(|| false);
    let mut name_input = use_signal(|| current_name.clone());

    fn save_display_name(
        storage: Storage,
        mut state: AppState,
        mut is_editing: Signal<bool>,
        name_input: Signal<String>,
        account_id: Uuid,
    ) {
        if !is_editing() {
            return;
        }
        is_editing.set(false);

        let name = name_input();
        if name.trim().is_empty() {
            return;
        }

        spawn(async move {
            match storage.update_display_name(account_id, name.trim()).await {
                Ok(()) => {
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
                Err(e) => {
                    tracing::error!("failed to update display name for {account_id}: {e}");
                }
            }
        });
    }

    let do_reset = {
        let storage = storage.clone();
        move |evt: Event<MouseData>| {
            evt.stop_propagation();
            let storage = storage.clone();
            let mut state = state;
            spawn(async move {
                match storage.clear_display_name(account_id).await {
                    Ok(()) => {
                        state.refresh_trigger.with_mut(|n| *n += 1);
                    }
                    Err(e) => {
                        tracing::error!("failed to clear display name for {account_id}: {e}");
                    }
                }
            });
        }
    };

    rsx! {
        if is_editing() {
            input {
                class: "settings__input-field settings__account-name-input",
                value: "{name_input}",
                oninput: move |evt| name_input.set(evt.value()),
                onkeydown: {
                    let storage = storage.clone();
                    move |evt| {
                        if evt.key() == Key::Enter {
                            save_display_name(
                                storage.clone(),
                                state,
                                is_editing,
                                name_input,
                                account_id,
                            );
                        }
                    }
                },
                onblur: {
                    let storage = storage.clone();
                    move |_| {
                        save_display_name(
                            storage.clone(),
                            state,
                            is_editing,
                            name_input,
                            account_id,
                        );
                    }
                },
                autofocus: true,
            }
        } else {
            div { class: "settings__account-name-wrap",
                div {
                    class: "settings__account-name-editable",
                    onclick: move |_| {
                        name_input.set(current_name.clone());
                        is_editing.set(true);
                    },
                    title: "Click to rename account",
                    span { class: "settings__account-name-text", "{current_name}" }
                    span { class: "settings__edit-icon", "✏️" }
                }

                if has_custom_name {
                    button {
                        class: "settings__button settings__button--reset-name",
                        title: "Revert to {email_address}",
                        onclick: do_reset,
                        "↺"
                    }
                }
            }
        }
    }
}
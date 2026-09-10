use dioxus::prelude::*;
use oauth::parse_google_credentials_json;
use storage::Storage;

#[component]
pub fn Credentials() -> Element {
    let store = use_context::<Storage>();

    let mut status_message = use_signal(String::new);

    let mut google_client_id_input = use_signal(String::new);
    let mut google_client_secret_input = use_signal(String::new);
    let mut has_custom_google_credentials = use_signal(|| false);
    let mut google_credentials_refresh = use_signal(|| 0u32);

    let mut microsoft_client_id_input = use_signal(String::new);
    let mut has_custom_microsoft_credentials = use_signal(|| false);
    let mut microsoft_credentials_refresh = use_signal(|| 0u32);

    // 1. Save custom Google credentials (Client ID in SQLite, Client Secret in Keyring)
    let save_google_credentials = {
        let store = store.clone();
        move |_| {
            let store = store.clone();
            let client_id = google_client_id_input();
            let client_secret = google_client_secret_input();

            if client_id.trim().is_empty() || client_secret.trim().is_empty() {
                status_message.set("Please provide both a Google Client ID and Client Secret.".to_string());
                return;
            }

            spawn(async move {
                match store
                    .save_user_google_credentials(&client_id, &client_secret)
                    .await
                {
                    Ok(_) => {
                        status_message.set("Google credentials saved successfully.".to_string());
                        google_credentials_refresh.with_mut(|n| *n += 1);
                    }
                    Err(e) => {
                        status_message.set(format!("Failed to save credentials: {e}"));
                        google_credentials_refresh.with_mut(|n| *n += 1);
                    }
                }
            });
        }
    };

    // 2. Import credentials.json via native file dialog
    let import_google_credentials = move |_| {
        spawn(async move {
            let file = rfd::AsyncFileDialog::new()
                .add_filter("JSON", &["json"])
                .set_title("Select Google credentials.json")
                .pick_file()
                .await;

            let Some(file) = file else {
                return;
            };

            let contents = String::from_utf8_lossy(&file.read().await).to_string();

            match parse_google_credentials_json(&contents) {
                Ok((client_id, client_secret)) => {
                    google_client_id_input.set(client_id);
                    google_client_secret_input.set(client_secret);
                    status_message.set("Credentials imported! Review and click Save Credentials to apply.".to_string());
                }
                Err(e) => {
                    status_message.set(format!("Failed to parse credentials.json: {e}"));
                }
            }
        });
    };

    // 3. Remove custom Google credentials
    let delete_google_credentials = {
        let store = store.clone();
        move |_| {
            let store = store.clone();
            spawn(async move {
                match store.delete_user_google_credentials().await {
                    Ok(_) => {
                        google_client_id_input.set(String::new());
                        google_client_secret_input.set(String::new());
                        status_message.set("Custom Google credentials removed. Reverted to app defaults.".to_string());
                        google_credentials_refresh.with_mut(|n| *n += 1);
                    }
                    Err(e) => status_message.set(format!("Failed to delete credentials: {e}")),
                }
            });
        }
    };

    use_resource({
        let store = store.clone();
        move || {
            let store = store.clone();
            let _ = google_credentials_refresh();
            async move {
                match store.get_user_google_credentials().await {
                    Ok(Some(_)) => has_custom_google_credentials.set(true),
                    Ok(None) => has_custom_google_credentials.set(false),
                    Err(e) => {
                        tracing::error!("failed to check for custom Google credentials: {e}");
                        has_custom_google_credentials.set(false);
                    }
                }
            }
        }
    });

    // 4. Save custom Microsoft Public Client ID
    let save_microsoft_credentials = {
        let store = store.clone();
        move |_| {
            let store = store.clone();
            let client_id = microsoft_client_id_input();

            if client_id.trim().is_empty() {
                status_message.set("Please provide a Microsoft Application (client) ID.".to_string());
                return;
            }

            spawn(async move {
                match store.save_user_microsoft_client_id(&client_id).await {
                    Ok(_) => {
                        status_message.set("Microsoft client ID saved successfully.".to_string());
                        microsoft_credentials_refresh.with_mut(|n| *n += 1);
                    }
                    Err(e) => {
                        status_message.set(format!("Failed to save client ID: {e}"));
                    }
                }
            });
        }
    };

    // 5. Remove custom Microsoft Client ID
    let delete_microsoft_credentials = {
        let store = store.clone();
        move |_| {
            let store = store.clone();
            spawn(async move {
                match store.delete_user_microsoft_client_id().await {
                    Ok(_) => {
                        microsoft_client_id_input.set(String::new());
                        status_message.set("Custom Microsoft client ID removed. Reverted to app defaults.".to_string());
                        microsoft_credentials_refresh.with_mut(|n| *n += 1);
                    }
                    Err(e) => {
                        status_message.set(format!("Failed to remove client ID: {e}"));
                    }
                }
            });
        }
    };

    use_resource({
        let store = store.clone();
        move || {
            let store = store.clone();
            let _ = microsoft_credentials_refresh();
            async move {
                match store.get_user_microsoft_client_id().await {
                    Ok(Some(_)) => has_custom_microsoft_credentials.set(true),
                    Ok(None) => has_custom_microsoft_credentials.set(false),
                    Err(e) => {
                        tracing::error!("failed to check for custom Microsoft client_id: {e}");
                        has_custom_microsoft_credentials.set(false);
                    }
                }
            }
        }
    });

    rsx! {
        div { class: "add-account__credentials-section",
            div { class: "add-account__section-header",
                span { class: "add-account__section-icon", "🔐" }
                h2 { "OAuth Credentials" }
            }

            div { class: "add-account__section-content",
                p { class: "add-account__description",
                    "Use your own OAuth credentials instead of the application's bundled defaults."
                }

                if !status_message().is_empty() {
                    div { class: "add-account__status-box",
                        div { class: "add-account__status-icon", "ℹ️" }
                        div { class: "add-account__status-text", "{status_message}" }
                    }
                }

                div { class: "add-account__credential-provider",
                    h3 { "Google" }

                    div { class: "add-account__credential-status",
                        span { class: if has_custom_google_credentials() { "add-account__status-dot add-account__status-dot--custom" } else { "add-account__status-dot add-account__status-dot--connected" } }
                        span {
                            if has_custom_google_credentials() {
                                "Using your own credentials"
                            } else {
                                "Using no credentials"
                            }
                        }
                    }

                    label { class: "add-account__input-label", "Client ID" }
                    input {
                        class: "add-account__input",
                        placeholder: "Google OAuth Client ID",
                        value: "{google_client_id_input}",
                        oninput: move |evt| google_client_id_input.set(evt.value()),
                    }

                    label { class: "add-account__input-label", "Client Secret" }
                    input {
                        class: "add-account__input",
                        r#type: "password",
                        placeholder: "Google OAuth Client Secret",
                        value: "{google_client_secret_input}",
                        oninput: move |evt| google_client_secret_input.set(evt.value()),
                    }

                    div { class: "add-account__credential-actions",
                        button {
                            class: "add-account__button add-account__button--import",
                            onclick: import_google_credentials,
                            "Import credentials.json"
                        }
                        button {
                            class: "add-account__button add-account__button--save",
                            onclick: save_google_credentials,
                            "Save Credentials"
                        }
                        if has_custom_google_credentials() {
                            button {
                                class: "add-account__button add-account__button--remove",
                                onclick: delete_google_credentials,
                                "Revert to Default"
                            }
                        }
                    }
                }

                div { class: "add-account__credential-provider",
                    h3 { "Microsoft" }

                    div { class: "add-account__credential-status",
                        span { class: if has_custom_microsoft_credentials() { "add-account__status-dot add-account__status-dot--custom" } else { "add-account__status-dot add-account__status-dot--connected" } }
                        span {
                            if has_custom_microsoft_credentials() {
                                "Using your own credentials"
                            } else {
                                "Using application credentials"
                            }
                        }
                    }

                    label { class: "add-account__input-label", "Application (client) ID" }
                    input {
                        class: "add-account__input",
                        placeholder: "Azure Application (client) ID",
                        value: "{microsoft_client_id_input}",
                        oninput: move |evt| microsoft_client_id_input.set(evt.value()),
                    }

                    div { class: "add-account__credential-actions",
                        button {
                            class: "add-account__button add-account__button--save",
                            onclick: save_microsoft_credentials,
                            "Save Credentials"
                        }
                        if has_custom_microsoft_credentials() {
                            button {
                                class: "add-account__button add-account__button--remove",
                                onclick: delete_microsoft_credentials,
                                "Revert to Default"
                            }
                        }
                    }
                }
            }
        }
    }
}
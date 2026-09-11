use common::Provider;
use dioxus::prelude::*;
use email_core::{EmailProvider, GmailProvider, OutlookProvider};
use oauth::get_access_token_for_account;
use self_update::VersionStatus;
use storage::Storage;
use storage::models::{EmailRow, LinkedAccount};
use uuid::Uuid;
use crate::{updater, AppState};
use crate::utils::sanitize_html;

#[component]
pub fn EmptyState(
    #[props(into)] icon: String,
    #[props(into)] title: String,
    #[props(into, default)] subtitle: Option<String>,
) -> Element {
    rsx! {
        div { class: "empty-state",
            span { class: "empty-state__icon", "{icon}" }
            p { class: "empty-state__title", "{title}" }
            if let Some(sub) = subtitle {
                p { class: "empty-state__hint", "{sub}" }
            }
        }
    }
}

fn build_webmail_url(account: &LinkedAccount, email: &EmailRow) -> Option<String> {
    match account.provider {
        Provider::Gmail => {
            Some(format!(
                "https://mail.google.com/mail/?authuser={}#all/{}",
                urlencoding::encode(&account.email_address),
                email.provider_message_id
            ))
        }
        Provider::Outlook => {
            let encoded_id = urlencoding::encode(&email.provider_message_id);
            if account.email_address.ends_with("@outlook.com")
                || account.email_address.ends_with("@hotmail.com")
                || account.email_address.ends_with("@live.com")
            {
                Some(format!("https://outlook.live.com/mail/deeplink/read/{encoded_id}"))
            } else {
                Some(format!("https://outlook.office.com/mail/deeplink/read/{encoded_id}"))
            }
        }
    }
}

#[component]
pub fn ReadingPane(
    email_id: Uuid,
    on_close: Option<EventHandler<()>>,
) -> Element {
    let storage = use_context::<Storage>();
    let mut state = use_context::<crate::state::AppState>();

    use_effect({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            spawn(async move {
                if storage.mark_email_viewed(email_id).await.is_ok() {
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }
            });
        }
    });

    let email_data = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                let email = storage
                    .get_email(email_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "email not found".to_string())?;

                let account = storage
                    .get_account(email.account_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "account not found".to_string())?;

                let client = reqwest::Client::new();
                let tokens = get_access_token_for_account(&storage, &account, &client)
                    .await
                    .map_err(|e| e.to_string())?;

                let provider: Box<dyn EmailProvider> = match account.provider {
                    Provider::Gmail => Box::new(GmailProvider::new(client)),
                    Provider::Outlook => Box::new(OutlookProvider::new(client)),
                };

                let full_message = provider
                    .fetch_full_message(&tokens.access_token, &email.provider_message_id)
                    .await
                    .map_err(|e| e.to_string())?;

                Ok::<_, String>((account, email, full_message))
            }
        }
    });

    rsx! {
        div { class: "reading-pane",
            if let Some(handler) = on_close {
                div { class: "reading-pane__mobile-header",
                    button {
                        class: "reading-pane__back-btn",
                        onclick: move |_| handler.call(()),
                        "<- Back to list"
                    }
                }
            }

            match &*email_data.read() {
                Some(Ok((account, email, full_message))) => {
                    let webmail_url = build_webmail_url(account, email);

                    rsx! {
                        div { class: "reading-pane__header",
                            div { class: "reading-pane__header-top",
                                h2 { class: "reading-pane__title",
                                    "{email.subject.clone().unwrap_or_else(|| \"(No subject)\".to_string())}"
                                }
                                if let Some(url) = webmail_url {
                                    button {
                                        class: "reading-pane__webmail-btn",
                                        title: "Open this email in your browser",
                                        onclick: move |_| {
                                            let _ = webbrowser::open(&url);
                                        },
                                        "Open in Webmail ↗"
                                    }
                                }
                            }
                            p { class: "reading-pane__sender",
                                "From: {email.sender.clone().unwrap_or_else(|| \"(Unknown sender)\".to_string())}"
                            }
                        }

                        div { class: "reading-pane__body",
                            if let Some(html) = &full_message.body_html {
                                iframe {
                                    id: "email-body-frame",
                                    class: "email-body__frame",
                                    srcdoc: "{sanitize_html(html)}",
                                    "sandbox": "allow-same-origin allow-top-navigation-by-user-activation",
                                }
                            } else if let Some(text) = &full_message.body_text {
                                pre { class: "email-body-text", "{text}" }
                            } else {
                                p { "(no content)" }
                            }
                        }

                        if !full_message.attachments.is_empty() {
                            div { class: "reading-pane__attachments",
                                h3 { "Attachments" }
                                for attachment in &full_message.attachments {
                                    div { class: "reading-pane__attachment", "{attachment.filename} ({attachment.size} bytes)" }
                                }
                            }
                        }
                    }
                }
                Some(Err(e)) => rsx! {
                    div { class: "reading-pane__state reading-pane__state--error", "Failed to load email: {e}" }
                },
                None => rsx! {
                    div { class: "reading-pane__state", "Loading email..." }
                },
            }
        }
    }
}

#[component]
pub fn UpdateBanner(version: String) -> Element {
    let mut state = use_context::<AppState>();
    let mut is_updating = use_signal(|| false);
    let mut update_status = use_signal(String::new);
    let mut banner_dismissed = use_signal(|| false);

    if banner_dismissed() {
        return rsx! {};
    }

    rsx! {
        div { class: "update-banner",
            div { class: "update-banner__info",
                span { class: "update-banner__dot" }
                span { class: "update-banner__text",
                    if update_status().is_empty() {
                        span {
                            "A new version of Fumiko ("
                            strong { "v{version}" }
                            ") is available!"
                        }
                    } else {
                        span { "{update_status}" }
                    }
                }
            }
            div { class: "update-banner__actions",
                if update_status().is_empty() {
                    button {
                        class: "update-banner__btn update-banner__btn--primary",
                        disabled: is_updating(),
                        onclick: move |_| {
                            is_updating.set(true);
                            update_status
                                .set("Downloading & installing update in background...".to_string());
                            spawn(async move {
                                match updater::update_app().await {
                                    Ok(VersionStatus::Updated(v)) => {
                                        update_status
                                            .set(format!("Updated to v{}! Please restart Fumiko.", v));
                                        state.available_update.set(None);
                                    }
                                    Ok(VersionStatus::UpToDate(_)) => {
                                        update_status.set("Fumiko is already up to date.".to_string());
                                        state.available_update.set(None);
                                    }
                                    Err(e) => {
                                        update_status.set(format!("Update failed: {}", e));
                                    }
                                    _ => {}
                                }
                                is_updating.set(false);
                            });
                        },
                        if is_updating() {
                            "Updating..."
                        } else {
                            "Update Now"
                        }
                    }
                }
                button {
                    class: "update-banner__btn update-banner__btn--dismiss",
                    onclick: move |_| banner_dismissed.set(true),
                    "✕"
                }
            }
        }
    }
}
//! Shared presentation components, reading pane viewer, and in-app update banner.

use common::Provider;
use dioxus::prelude::*;
use email_core::{EmailProvider, GmailProvider, ImapProvider, OutlookProvider};
use oauth::get_access_token_for_account;
use self_update::VersionStatus;
use storage::Storage;
use storage::models::{EmailRow, LinkedAccount};
use uuid::Uuid;

use crate::utils::sanitize_html;
use crate::{AppState, kill_builtin_server, updater};

/// Renderable placeholder displayed when an inbox, findings stream, or reading pane is empty.
#[component]
pub fn EmptyState(
    #[props(into, default)] icon: Option<String>,
    #[props(into, default)] icon_class: Option<String>,
    #[props(into)] title: String,
    #[props(into, default)] subtitle: Option<String>,
) -> Element {
    let resolved_class = icon_class.unwrap_or_else(|| "icon icon--empty".to_string());

    rsx! {
        div { class: "empty-state",
            if let Some(text_icon) = icon {
                span { class: "empty-state__icon empty-state__icon--text", "{text_icon}" }
            } else {
                span { class: "empty-state__icon {resolved_class}" }
            }
            p { class: "empty-state__title", "{title}" }
            if let Some(sub) = subtitle {
                p { class: "empty-state__hint", "{sub}" }
            }
        }
    }
}

/// Formats byte counts into human-readable attachment sizes (B, KB, MB, GB).
fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Constructs a direct webmail deep link for opening an email in the user's default browser.
///
/// Because Fumiko operates with read-only scopes, users cannot reply to emails directly inside
/// the application. Deep links provide a one-click handoff to webmail with the specific account profile.
fn build_webmail_url(account: &LinkedAccount, email: &EmailRow) -> Option<String> {
    let email_lower = account.email_address.to_ascii_lowercase();

    match account.provider {
        // Gmail: authuser parameter opens the correct Google account session even if multiple accounts are logged in
        Provider::Gmail => Some(format!(
            "https://mail.google.com/mail/?authuser={}#all/{}",
            urlencoding::encode(&account.email_address),
            email.provider_message_id
        )),
        // Outlook: Distinguishes between personal Microsoft accounts and corporate Office 365 tenants
        Provider::Outlook => {
            let encoded_id = urlencoding::encode(&email.provider_message_id);
            if email_lower.ends_with("@outlook.com")
                || email_lower.ends_with("@hotmail.com")
                || email_lower.ends_with("@live.com")
            {
                Some(format!(
                    "https://outlook.live.com/mail/deeplink/read/{encoded_id}"
                ))
            } else {
                Some(format!(
                    "https://outlook.office.com/mail/deeplink/read/{encoded_id}"
                ))
            }
        }
        // IMAP: Providers do not expose internal IMAP UIDs in URL hashes; routes to the provider webmail portal
        Provider::Imap => {
            if email_lower.ends_with("@gmail.com")
                || account.imap_host.as_deref() == Some("imap.gmail.com")
            {
                Some(format!(
                    "https://mail.google.com/mail/?authuser={}",
                    urlencoding::encode(&account.email_address)
                ))
            } else if email_lower.ends_with("@outlook.com")
                || email_lower.ends_with("@hotmail.com")
                || account.imap_host.as_deref() == Some("outlook.office365.com")
            {
                Some("https://outlook.live.com/mail/".to_string())
            } else if email_lower.ends_with("@icloud.com")
                || account.imap_host.as_deref() == Some("imap.mail.me.com")
            {
                Some("https://www.icloud.com/mail".to_string())
            } else if email_lower.ends_with("@yahoo.com")
                || account.imap_host.as_deref() == Some("imap.mail.yahoo.com")
            {
                Some("https://mail.yahoo.com".to_string())
            } else {
                None
            }
        }
    }
}

/// Reading pane component responsible for lazy-fetching full message payloads and rendering sandboxed HTML.
#[component]
pub fn ReadingPane(email_id: Uuid, on_close: Option<EventHandler<()>>) -> Element {
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    // Lazy Message Retrieval:
    // Email bodies are not saved to SQLite during background sync. When an email is selected,
    // this resource loads account credentials and retrieves the full MIME payload on demand.
    let email_data = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                let email = storage
                    .get_email(email_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "Email not found".to_string())?;

                let account = storage
                    .get_account(email.account_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "Linked account not found".to_string())?;

                // Mark as viewed locally without altering remote server read flags
                if !email.app_has_viewed {
                    let _ = storage.mark_email_viewed(email_id).await;
                    state.refresh_trigger.with_mut(|n| *n += 1);
                }

                let client = reqwest::Client::new();
                let tokens = get_access_token_for_account(&storage, &account, &client)
                    .await
                    .map_err(|e| e.to_string())?;

                let provider: Box<dyn EmailProvider> =
                    if account.provider == Provider::Imap || account.imap_host.is_some() {
                        let host = account
                            .imap_host
                            .clone()
                            .unwrap_or_else(|| "imap.gmail.com".to_string());
                        let port = account.imap_port_u16().unwrap_or(993);

                        Box::new(ImapProvider::new(host, port, account.email_address.clone()))
                    } else {
                        match account.provider {
                            Provider::Gmail => Box::new(GmailProvider::new(client)),
                            Provider::Outlook => Box::new(OutlookProvider::new(client)),
                            Provider::Imap => unreachable!("Handled in preceding condition"),
                        }
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
                        "Back to list"
                    }
                }
            }

            match &*email_data.read() {
                Some(Ok((account, email, full_message))) => {
                    let webmail_url = build_webmail_url(account, email);

                    rsx! {
                        div { class: "reading-pane__header",
                            div { class: "reading-pane__header-top",
                                h2 { class: "reading-pane__title", "{email.subject.as_deref().unwrap_or(\"(No subject)\")}" }
                                if let Some(url) = webmail_url {
                                    button {
                                        class: "reading-pane__webmail-btn",
                                        title: "Open this email in your default browser",
                                        onclick: move |_| {
                                            let _ = webbrowser::open(&url);
                                        },
                                        span { "Open in Webmail" }
                                        span { class: "icon icon--external-link", aria_hidden: "true" }
                                    }
                                }
                            }
                            p { class: "reading-pane__sender",
                                "From: {email.sender.as_deref().unwrap_or(\"(Unknown sender)\")}"
                            }
                        }

                        div { class: "reading-pane__body",
                            if let Some(html) = &full_message.body_html {
                                // Sandboxed Iframe:
                                // Scripts and forms remain completely disabled.
                                // allow-top-navigation-by-user-activation permits user clicks to target _top,
                                // where Dioxus desktop navigation handlers trap external URLs safely.
                                iframe {
                                    id: "email-body-frame",
                                    class: "email-body__frame",
                                    srcdoc: "{sanitize_html(html)}",
                                    "sandbox": "allow-same-origin allow-top-navigation-by-user-activation",
                                }
                            } else if let Some(text) = &full_message.body_text {
                                pre { class: "email-body-text", "{text}" }
                            } else {
                                p { class: "reading-pane__empty-text", "(No readable message content)" }
                            }
                        }

                        if !full_message.attachments.is_empty() {
                            div { class: "reading-pane__attachments",
                                h3 { class: "reading-pane__attachments-title", "Attachments" }
                                for attachment in &full_message.attachments {
                                    div { class: "reading-pane__attachment",
                                        "{attachment.filename} ({format_file_size(attachment.size)})"
                                    }
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

/// Notification banner displayed when a newer application release is detected on GitHub.
#[component]
pub fn UpdateBanner(version: String) -> Element {
    let mut state = use_context::<AppState>();
    let mut is_updating = use_signal(|| false);
    let mut update_status = use_signal(String::new);
    let mut is_updated = use_signal(|| false);
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
                if is_updated() {
                    button {
                        class: "update-banner__btn update-banner__btn--primary",
                        onclick: move |_| -> () {
                            // Ensure background AI child processes are terminated prior to exit
                            kill_builtin_server();
                            std::process::exit(0);
                        },
                        "Restart Now"
                    }
                } else if update_status().is_empty() {
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
                                        is_updated.set(true);
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
                    onclick: move |_| {
                        banner_dismissed.set(true);
                        state.available_update.set(None);
                    },
                    "✕"
                }
            }
        }
    }
}
//! View for connecting email accounts via OAuth 2.0 PKCE or TLS IMAP with App Passwords.

mod credentials;
mod provider;

pub use credentials::Credentials;
pub use provider::{provider_icon, provider_name};

use common::{Provider, config, setting_keys};
use dioxus::prelude::*;
use storage::Storage;

use crate::Route;
use crate::state::{AppState, AuthCommand};

#[component]
pub fn AddAccount() -> Element {
    let nav = use_navigator();
    let mut state = use_context::<AppState>();
    let store = use_context::<Storage>();

    let mut selected_provider = use_signal(|| Provider::Gmail);
    let mut was_linking = use_signal(|| (state.is_linking)());

    let has_google_keys_resource = use_resource({
        let store = store.clone();
        move || {
            let store = store.clone();
            let _ = (state.refresh_trigger)();
            async move {
                store
                    .get_user_google_credentials()
                    .await
                    .ok()
                    .flatten()
                    .is_some()
            }
        }
    });
    let has_google_keys = has_google_keys_resource.read().unwrap_or(false);

    let mut imap_preset = use_signal(|| "gmail");
    let mut imap_email = use_signal(String::new);
    let mut imap_password = use_signal(String::new);
    let mut imap_host = use_signal(|| "imap.gmail.com".to_string());
    let mut imap_port = use_signal(|| "993".to_string());
    let mut imap_validation_error = use_signal(String::new);

    // Auto-Redirect on Connection Success:
    // Observes transition from linking active to linking complete with a success message
    use_effect(move || {
        let is_linking = (state.is_linking)();
        let status = (state.linking_status)();
        let previously_linking = *was_linking.peek();

        if previously_linking && !is_linking && status.contains("connected!") {
            state.linking_status.set(String::new());
            nav.push(Route::Dashboard {});
        }

        was_linking.set(is_linking);
    });

    let mut select_preset = move |preset: &'static str| {
        imap_preset.set(preset);
        match preset {
            "gmail" => {
                imap_host.set("imap.gmail.com".to_string());
                imap_port.set("993".to_string());
            }
            "icloud" => {
                imap_host.set("imap.mail.me.com".to_string());
                imap_port.set("993".to_string());
            }
            "yahoo" => {
                imap_host.set("imap.mail.yahoo.com".to_string());
                imap_port.set("993".to_string());
            }
            "fastmail" => {
                imap_host.set("imap.fastmail.com".to_string());
                imap_port.set("993".to_string());
            }
            "custom" => {}
            _ => {}
        }
    };

    let start_oauth = move |_| {
        if (state.is_linking)() {
            return;
        }

        let provider = selected_provider();
        let store = store.clone();

        spawn(async move {
            if provider == Provider::Gmail {
                let has_keys = store
                    .get_user_google_credentials()
                    .await
                    .ok()
                    .flatten()
                    .is_some();
                if !has_keys {
                    state.linking_status.set(
                        "Google OAuth requires custom developer keys. Please add your credentials in the Developer section below, or switch to IMAP with an App Password (Recommended).".to_string()
                    );
                    return;
                }
            }

            state.linking_status.set(String::new());
            let _ = (state.auth_tx)().send(AuthCommand::Start(provider));
        });
    };

    let mut start_imap = move |_| {
        if (state.is_linking)() {
            return;
        }

        let email = imap_email().trim().to_string();
        let mut password = imap_password().trim().to_string();
        let host = imap_host().trim().to_string();
        let port_str = imap_port().trim().to_string();

        // Password Normalization:
        // Automatically strips whitespace groups copied from Google or Apple credential generators
        password.retain(|c| !c.is_whitespace());

        let port = match port_str.parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => {
                imap_validation_error
                    .set("Port must be a valid number between 1 and 65535.".to_string());
                return;
            }
        };

        if email.is_empty() || password.is_empty() || host.is_empty() {
            imap_validation_error
                .set("Please fill in email, app password, and server host.".to_string());
            return;
        }

        imap_validation_error.set(String::new());
        state.linking_status.set(String::new());

        let _ = (state.auth_tx)().send(AuthCommand::StartImap {
            email,
            password,
            host,
            port,
        });
    };

    let cancel_linking = move |_| {
        let _ = (state.auth_tx)().send(AuthCommand::Cancel);
    };

    rsx! {
        div { class: "add-account",
            div { class: "add-account__header",
                h1 { class: "add-account__title", "Add Account" }
                p { class: "add-account__subtitle", "Connect your email accounts securely" }
            }

            PreLinkNotice {}

            div { class: "add-account__provider-grid",
                button {
                    class: if selected_provider() == Provider::Gmail { "add-account__provider-card add-account__provider-card--selected" } else { "add-account__provider-card" },
                    onclick: move |_| {
                        if !(state.is_linking)() {
                            selected_provider.set(Provider::Gmail);
                        }
                    },
                    div { class: "add-account__provider-icon", {provider_icon(Provider::Gmail)} }
                    span { class: "add-account__provider-name", "Gmail" }
                }

                button {
                    class: if selected_provider() == Provider::Outlook { "add-account__provider-card add-account__provider-card--selected" } else { "add-account__provider-card" },
                    onclick: move |_| {
                        if !(state.is_linking)() {
                            selected_provider.set(Provider::Outlook);
                        }
                    },
                    div { class: "add-account__provider-icon", {provider_icon(Provider::Outlook)} }
                    span { class: "add-account__provider-name", "Outlook" }
                }

                button {
                    class: if selected_provider() == Provider::Imap { "add-account__provider-card add-account__provider-card--selected" } else { "add-account__provider-card" },
                    onclick: move |_| {
                        if !(state.is_linking)() {
                            selected_provider.set(Provider::Imap);
                        }
                    },
                    div { class: "add-account__provider-icon", {provider_icon(Provider::Imap)} }
                    span { class: "add-account__provider-name", "IMAP" }
                    span { class: "add-account__provider-badge add-account__provider-badge--highlight",
                        "App Password"
                    }
                }
            }

            if selected_provider() == Provider::Gmail && !has_google_keys {
                div { class: "add-account__notice-box",
                    span { class: "icon icon--info add-account__notice-icon" }
                    div { class: "add-account__notice-content",
                        strong { "Recommended for Gmail: Connect via IMAP & App Password" }
                        p {
                            "Google requires enterprise verification for public OAuth sign-ins. "
                            "Unless you add custom developer keys below, we recommend connecting with an "
                            strong { "App Password" }
                            ": it takes 1 minute and requires zero Google Cloud setup!"
                        }
                        button {
                            class: "add-account__notice-action",
                            onclick: move |_| {
                                selected_provider.set(Provider::Imap);
                                select_preset("gmail");
                            },
                            "Switch to Gmail via App Password →"
                        }
                    }
                }
            }

            if selected_provider() == Provider::Outlook {
                div { class: "add-account__notice-box",
                    span { class: "icon icon--info add-account__notice-icon" }
                    div { class: "add-account__notice-content",
                        strong { "Personal Microsoft Accounts Supported (@outlook, @hotmail, @live)" }
                        p {
                            "Personal accounts connect seamlessly with one click. "
                            "Because Microsoft requires verified corporate publisher credentials for organizational tenants, "
                            strong { "work and school accounts will show a 'Need admin approval' screen." }
                        }
                    }
                }
            }

            if selected_provider() == Provider::Imap {
                div { class: "add-account__imap-section",
                    div { class: "add-account__imap-header",
                        div {
                            h2 { class: "add-account__section-title",
                                "Connect via IMAP & App Password"
                            }
                            p { class: "add-account__description",
                                "Connect any standard email account directly without setting up cloud developer keys."
                            }
                        }
                    }

                    div { class: "add-account__preset-group",
                        label { class: "add-account__input-label", "Provider Preset" }
                        div { class: "add-account__preset-track",
                            for & (id , label) in &[
                                ("gmail", "Gmail"),
                                ("icloud", "iCloud"),
                                ("yahoo", "Yahoo"),
                                ("fastmail", "Fastmail"),
                                ("custom", "Custom Server"),
                            ]
                            {
                                button {
                                    key: "{id}",
                                    class: if imap_preset() == id { "add-account__preset-pill add-account__preset-pill--active" } else { "add-account__preset-pill" },
                                    onclick: move |_| select_preset(id),
                                    "{label}"
                                }
                            }
                        }
                    }

                    div { class: "add-account__guide-card",
                        div { class: "add-account__guide-header",
                            span { class: "add-account__guide-badge", "Setup Guide" }
                        }
                        div { class: "add-account__guide-body",
                            match imap_preset() {
                                "gmail" => rsx! {
                                    p {
                                        "Generate a 16-character App Password at "
                                        a {
                                            href: "https://myaccount.google.com/apppasswords",
                                            class: "add-account__guide-link",
                                            onclick: move |evt| {
                                                evt.prevent_default();
                                                let _ = webbrowser::open("https://myaccount.google.com/apppasswords");
                                            },
                                            "myaccount.google.com/apppasswords"
                                        }
                                        " and paste it below."
                                    }
                                },
                                "icloud" => rsx! {
                                    p {
                                        "Generate an app-specific password at "
                                        a {
                                            href: "https://appleid.apple.com/account/manage",
                                            class: "add-account__guide-link",
                                            onclick: move |evt| {
                                                evt.prevent_default();
                                                let _ = webbrowser::open("https://appleid.apple.com/account/manage");
                                            },
                                            "appleid.apple.com"
                                        }
                                        "."
                                    }
                                },
                                "yahoo" => rsx! {
                                    p {
                                        "Generate an App Password in your "
                                        a {
                                            href: "https://login.yahoo.com/account/security",
                                            class: "add-account__guide-link",
                                            onclick: move |evt| {
                                                evt.prevent_default();
                                                let _ = webbrowser::open("https://login.yahoo.com/account/security");
                                            },
                                            "Yahoo Account Security"
                                        }
                                        " settings and paste it below."
                                    }
                                },
                                "fastmail" => rsx! {
                                    p {
                                        "Generate an App Password under Privacy & Security at "
                                        a {
                                            href: "https://app.fastmail.com/settings/security",
                                            class: "add-account__guide-link",
                                            onclick: move |evt| {
                                                evt.prevent_default();
                                                let _ = webbrowser::open("https://app.fastmail.com/settings/security");
                                            },
                                            "app.fastmail.com/settings/security"
                                        }
                                        " and paste it below."
                                    }
                                },
                                _ => rsx! {
                                    p { "Enter your custom server's secure host and port (standard TLS IMAP port is 993)." }
                                },
                            }
                        }
                    }

                    div { class: "add-account__fields-group",
                        div { class: "add-account__field",
                            label { class: "add-account__input-label", "Email Address" }
                            input {
                                class: "add-account__input",
                                r#type: "email",
                                placeholder: "you@example.com",
                                value: "{imap_email}",
                                oninput: move |evt| imap_email.set(evt.value()),
                            }
                        }

                        div { class: "add-account__field",
                            label { class: "add-account__input-label", "App Password" }
                            input {
                                class: "add-account__input",
                                r#type: "password",
                                placeholder: "•••• •••• •••• ••••",
                                value: "{imap_password}",
                                oninput: move |evt| imap_password.set(evt.value()),
                            }
                        }
                    }

                    div { class: "add-account__server-section",
                        div { class: "add-account__server-header",
                            span { class: "add-account__server-title", "Connection Parameters" }
                            span { class: "add-account__server-sub", "TLS Encrypted (Port 993)" }
                        }

                        div { class: "add-account__server-grid",
                            div { class: "add-account__field",
                                label { class: "add-account__input-label", "Server Host" }
                                input {
                                    class: "add-account__input",
                                    placeholder: "imap.example.com",
                                    value: "{imap_host}",
                                    oninput: move |evt| imap_host.set(evt.value()),
                                }
                            }
                            div { class: "add-account__field",
                                label { class: "add-account__input-label", "Port" }
                                input {
                                    class: "add-account__input",
                                    r#type: "number",
                                    placeholder: "993",
                                    value: "{imap_port}",
                                    oninput: move |evt| imap_port.set(evt.value()),
                                }
                            }
                        }
                    }

                    if !imap_validation_error().is_empty() {
                        div { class: "add-account__error-box",
                            span { "{imap_validation_error}" }
                        }
                    }
                }
            }

            div { class: "add-account__status-section",
                if !(state.linking_status)().is_empty() {
                    div { class: "add-account__status-box",
                        span { class: "icon icon--info add-account__status-icon" }
                        div { class: "add-account__status-text", "{(state.linking_status)()}" }
                    }
                }

                div { class: "add-account__info-box",
                    div { class: "add-account__info-text",
                        strong { "Your credentials are safe." }
                        if selected_provider() == Provider::Imap {
                            p {
                                "Your App Password is encrypted and stored exclusively in your operating system's "
                                "native credential vault (Keyring). It is never sent to external servers."
                            }
                        } else {
                            p {
                                "This app uses OAuth 2.0 PKCE to connect to your provider. "
                                "Your password is never handled, seen, or stored."
                            }
                        }
                    }
                }

                div { class: "add-account__info-box",
                    div { class: "add-account__info-text",
                        strong { "100% On-Device AI." }
                        p {
                            "Email categorization runs directly on your hardware via local models. "
                            "No email content is ever uploaded to external cloud AI services."
                        }
                    }
                }
            }

            if (state.is_linking)() {
                div { class: "add-account__connecting-actions",
                    button {
                        class: "add-account__connect-button add-account__connect-button--loading",
                        disabled: true,
                        "Connecting to {provider_name(selected_provider())}..."
                    }
                    button {
                        class: "add-account__button add-account__button--cancel",
                        onclick: cancel_linking,
                        "Cancel Connecting"
                    }
                }
            } else {
                button {
                    class: "add-account__connect-button",
                    onclick: move |evt| {
                        if selected_provider() == Provider::Imap {
                            start_imap(evt);
                        } else {
                            start_oauth(evt);
                        }
                    },
                    "Connect {provider_name(selected_provider())} Account"
                }
            }

            Credentials {}
        }
    }
}

/// Information card alerting the user to classification status and sync bounds before account linking.
#[component]
fn PreLinkNotice() -> Element {
    let storage = use_context::<Storage>();

    let config_status = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                let model = storage
                    .get_setting(setting_keys::ACTIVE_AI_MODEL_ID)
                    .await
                    .ok()
                    .flatten()
                    .filter(|m| !m.trim().is_empty());

                let limit = storage
                    .get_setting(setting_keys::INITIAL_SYNC_LIMIT)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(config::DEFAULT_INITIAL_SYNC_LIMIT);

                (model, limit)
            }
        }
    });

    let (active_model, pull_limit) = config_status
        .read()
        .as_ref()
        .cloned()
        .unwrap_or((None, 100));

    let has_model = active_model.is_some();

    rsx! {
        div { class: "add-account__pre-link-notice",
            div { class: "add-account__pre-link-content",
                h4 { class: "add-account__pre-link-title", "Before Linking Your Account" }
                ul { class: "add-account__pre-link-list",
                    li {
                        span { class: if !has_model { "add-account__pre-link-alert" } else { "" },
                            if let Some(model_name) = active_model {
                                "AI Classifier: Active ({model_name})"
                            } else {
                                "AI Classifier: No model selected! New emails won't be categorized."
                            }
                        }
                    }
                    li {
                        "Initial Pull: Fumiko will retrieve your latest "
                        strong { "{pull_limit} emails" }
                        " on first sync."
                    }
                }
            }

            Link { to: Route::Settings {}, class: "add-account__pre-link-action", "Adjust in Settings" }
        }
    }
}
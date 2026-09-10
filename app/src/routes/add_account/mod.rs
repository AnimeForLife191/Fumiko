mod credentials;
mod provider;

pub use credentials::Credentials;
pub use provider::{provider_icon, provider_name};

use common::Provider;
use dioxus::prelude::*;

use crate::state::{AppState, AuthCommand};
use crate::Route;

#[component]
pub fn AddAccount() -> Element {
    let nav = use_navigator();
    let mut state = use_context::<AppState>();

    let mut selected_provider = use_signal(|| Provider::Gmail);
    // Track previous linking state to only redirect on an active completion transition
    let mut was_linking = use_signal(|| (state.is_linking)());

    // Auto-redirect to Dashboard only when connection actively finishes
    use_effect(move || {
        let is_linking = (state.is_linking)();
        let status = (state.linking_status)();
        let previously_linking = *was_linking.peek();

        if previously_linking && !is_linking && status.contains("connected!") {
            state.linking_status.set(String::new()); // Clear status so it won't trigger again
            nav.push(Route::Dashboard {});
        }

        was_linking.set(is_linking);
    });

    let start_oauth = move |_| {
        if !(state.is_linking)() {
            state.linking_status.set(String::new());
            let _ = (state.auth_tx)().send(AuthCommand::Start(selected_provider()));
        }
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
                    class: "add-account__provider-card add-account__provider-card--disabled",
                    disabled: true,
                    div { class: "add-account__provider-icon", "🔜" }
                    span { class: "add-account__provider-name", "More coming..." }
                    span { class: "add-account__provider-badge", "Soon" }
                }
            }

            div { class: "add-account__status-section",
                if !(state.linking_status)().is_empty() {
                    div { class: "add-account__status-box",
                        div { class: "add-account__status-icon", "ℹ️" }
                        div { class: "add-account__status-text", "{(state.linking_status)()}" }
                    }
                }

                div { class: "add-account__info-box",
                    div { class: "add-account__info-text",
                        strong { "Your credentials are safe." }
                        p {
                            "This app uses OAuth 2.0 PKCE to connect to your provider. "
                            "Your password is never handled, seen, or stored."
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

            // Connect & Cancel Button Section
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
                    onclick: start_oauth,
                    "Connect {provider_name(selected_provider())} Account"
                }
            }

            Credentials {}
        }
    }
}
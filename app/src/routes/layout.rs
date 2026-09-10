use dioxus::prelude::*;
use crate::state::AppState;
use crate::{Route, routes::component::UpdateBanner};

#[component]
pub fn Layout() -> Element {
    let current_route = use_route::<Route>();
    let mut state = use_context::<AppState>();

    rsx! {
        div { class: "app",
            nav { class: "sidebar",
                div { class: "sidebar-title", "Fumiko (文子)" }

                // 1. Primary Views
                div { class: "nav-section",
                    Link {
                        to: Route::Dashboard {},
                        class: if matches!(current_route, Route::Dashboard {}) { "nav-item nav-item--active" } else { "nav-item" },
                        "Dashboard"
                    }
                    Link {
                        to: Route::Inbox {},
                        class: if matches!(current_route, Route::Inbox {}) { "nav-item nav-item--active" } else { "nav-item" },
                        "Inbox"
                    }
                    Link {
                        to: Route::Findings {},
                        class: if matches!(current_route, Route::Findings {}) { "nav-item nav-item--active" } else { "nav-item" },
                        "Findings"
                    }
                    Link {
                        to: Route::Trash {},
                        class: if matches!(current_route, Route::Trash {}) { "nav-item nav-item--active" } else { "nav-item" },
                        "Trash"
                    }
                }

                div { class: "sidebar-divider" }

                // 2. Account Filter Selector
                div { class: "nav-section",
                    div {
                        class: if (state.selected_account)().is_none() { "account-item account-item--active" } else { "account-item" },
                        onclick: move |_| state.selected_account.set(None),
                        "All Accounts"
                    }

                    for account in (state.accounts)().into_iter() {
                        {
                            let account_id = account.id;
                            let account_name = account
                                .display_name
                                .as_deref()
                                .unwrap_or(&account.email_address)
                                .to_string();
                            rsx! {
                                div {
                                    key: "{account_id}",
                                    class: if (state.selected_account)() == Some(account_id) { "account-item account-item--active" } else { "account-item" },
                                    title: "{account_name}",
                                    onclick: move |_| state.selected_account.set(Some(account_id)),
                                    "{account_name}"
                                }
                            }
                        }
                    }
                }

                div { class: "sidebar-divider" }

                // 3. App Settings & Add Account
                div { class: "nav-section",
                    Link {
                        to: Route::AddAccount {},
                        class: if matches!(current_route, Route::AddAccount {}) { "nav-item nav-item--active" } else { "nav-item" },
                        "Add Account"
                    }
                    Link {
                        to: Route::Settings {},
                        class: if matches!(current_route, Route::Settings {}) { "nav-item nav-item--active" } else { "nav-item" },
                        "Settings"
                    }
                }

                // 4. Pinned Bottom Footer
                div { class: "nav-section nav-section--footer",
                    button {
                        class: "nav-item nav-item--button",
                        onclick: move |_| {
                            let _ = webbrowser::open("https://github.com/AnimeForLife191/Fumiko/issues/new");
                        },
                        "Report a Bug"
                    }
                }
            }

            // Main Content Area
            div { class: "content-area",
                if let Some(ver) = (state.available_update)() {
                    UpdateBanner { version: ver }
                }

                div { class: "main-content", Outlet::<Route> {} }
            }
        }
    }
}
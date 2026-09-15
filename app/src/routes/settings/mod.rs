//! Submodules and two-column layout for application settings.

mod ai_selection;
mod css_setting;
mod danger_zone;
mod link_account;
mod sync_options;
mod trash_retention;
mod watch_criteria;
mod capacity_options;

use ai_selection::AiSelection;
use css_setting::CustomThemeSection;
use danger_zone::DangerZoneSection;
use link_account::LinkedAccountsSection;
use sync_options::InitialSyncSection;
use trash_retention::TrashRetentionSection;
use watch_criteria::WatchCriteriaSection;
use capacity_options::MailboxCapicitySection;

use dioxus::prelude::*;

#[component]
pub fn Settings() -> Element {
    rsx! {
        div { class: "settings",
            div { class: "settings__header",
                div { class: "settings__title",
                    h1 { "Settings" }
                    p { class: "settings__subtitle",
                        "Configure local mailbox syncing, AI models, and interface preferences."
                    }
                }
            }

            div { class: "settings__grid",
                div { class: "settings__column",
                    LinkedAccountsSection {}
                    WatchCriteriaSection {}
                    CustomThemeSection {}
                    DangerZoneSection {}
                }

                div { class: "settings__column",
                    AiSelection {}

                    div { class: "settings__section",
                        div { class: "settings__section-header",
                            span {
                                class: "icon icon--settings",
                                aria_hidden: "true",
                            }
                            h2 { "Sync & Storage" }
                        }
                        div { class: "settings__section-content settings__section-content--stacked",
                            InitialSyncSection {}
                            div { class: "settings__subdivider" }
                            TrashRetentionSection {}
                            div { class: "settings__subdivider" }
                            MailboxCapicitySection {}
                        }
                    }
                
                }
            }
        }
    }
}
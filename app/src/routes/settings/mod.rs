mod link_account;
mod trash_retention;
mod ai_selection;
mod watch_criteria;
mod danger_zone;

use link_account::LinkedAccountsSection;
use trash_retention::TrashRetentionSection;
use ai_selection::AiSelection;
use watch_criteria::WatchCriteriaSection;
use danger_zone::DangerZoneSection;

use dioxus::prelude::*;

#[component]
pub fn Settings() -> Element {
    rsx! {
        div { class: "settings",
            div { class: "settings__header",
                div { class: "settings__title",
                    h1 { "Settings" }
                }
            }

            LinkedAccountsSection {}
            TrashRetentionSection {}
            AiSelection {}
            WatchCriteriaSection {}
            DangerZoneSection {}
        }
    }
}

use dioxus::prelude::*;
use common::setting_keys;
use storage::Storage;

#[component]
pub fn TrashRetentionSection() -> Element {
    let storage = use_context::<Storage>();

    let mut trash_retention_days = use_signal(|| "30".to_string());
    use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                if let Ok(Some(days)) = storage.get_setting(setting_keys::TRASH_RETENTION_DAYS).await {
                    trash_retention_days.set(days);
                }
            }
        }
    });

    let set_trash_retention = {
        let storage = storage.clone();
        move |evt: FormEvent| {
            let storage = storage.clone();
            let value = evt.value();
            trash_retention_days.set(value.clone());

            spawn(async move {
                if let Err(e) = storage.set_setting(setting_keys::TRASH_RETENTION_DAYS, &value).await {
                    tracing::error!("failed to save trash retention setting: {e}");
                }
            });
        }
    };

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                h2 { "Storage and Retention" }
            }
            div { class: "settings__section-content",
                p { class: "settings__description",
                    "Configure how long trashed emails are preserved before permanent deletion."
                }
                div { class: "settings__select-group",
                    label { class: "settings__select-label", "Trash Retention Period" }
                    select {
                        class: "settings__model-select",
                        value: "{trash_retention_days}",
                        onchange: set_trash_retention,
                        option { value: "7", "7 days" }
                        option { value: "14", "14 days" }
                        option { value: "30", "30 days (Default)" }
                        option { value: "60", "60 days" }
                        option { value: "90", "90 days" }
                        option { value: "365", "1 year" }
                    }
                }
            }
        }
    }
}
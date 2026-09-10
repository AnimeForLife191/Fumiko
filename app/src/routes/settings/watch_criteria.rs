use dioxus::prelude::*;
use storage::Storage;
use crate::AppState;

#[component]
pub fn WatchCriteriaSection() -> Element {
    let storage = use_context::<Storage>();
    let mut state = use_context::<AppState>();

    let mut criteria_refresh = use_signal(|| 0u32);
    let mut new_label = use_signal(String::new);
    let mut new_description = use_signal(String::new);

    let criteria = use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let _ = criteria_refresh();
            async move { storage.list_criteria().await }
        }
    });

    let criteria_list = criteria
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned()
        .unwrap_or_default();

    let add_criterion = {
        let storage = storage.clone();
        move |_| {
            let storage = storage.clone();
            let label = new_label();
            let description = new_description();

            if label.trim().is_empty() || description.trim().is_empty() {
                return;
            }

            spawn(async move {
                match storage.add_criterion(&label, &description).await {
                    Ok(_) => {
                        new_label.set(String::new());
                        new_description.set(String::new());
                        criteria_refresh.with_mut(|n| *n += 1);
                        state.refresh_trigger.with_mut(|n| *n += 1); // Updates dashboard stats immediately!
                    }
                    Err(e) => {
                        tracing::error!("failed to add criterion: {e}");
                    }
                }
            });
        }
    };

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                h2 { "Watch Criteria" }
            }
            div { class: "settings__section-content",
                p { class: "settings__description",
                    "The AI looks for these criteria when classifying emails."
                }

                div { class: "settings__criteria-list",
                    for criterion in criteria_list {
                        div {
                            class: "settings__criteria-item",
                            key: "{criterion.id}",
                            span { class: "settings__criteria-label", "{criterion.label}" }
                            span { class: "settings__criteria-description", "{criterion.description}" }
                            button {
                                class: "settings__button settings__button--remove settings__button--small",
                                onclick: {
                                    let storage = storage.clone();
                                    let criterion_id = criterion.id;
                                    move |_| {
                                        let storage = storage.clone();
                                        spawn(async move {
                                            if let Err(e) = storage.delete_criterion(criterion_id).await {
                                                tracing::error!("failed to delete criterion: {e}");
                                            } else {
                                                criteria_refresh.with_mut(|n| *n += 1);
                                                state.refresh_trigger.with_mut(|n| *n += 1);
                                            }
                                        });
                                    }
                                },
                                "×"
                            }
                        }
                    }
                }

                div { class: "settings__criteria-form",
                    input {
                        class: "settings__input-field",
                        placeholder: "Label (e.g. Promotions)",
                        value: "{new_label}",
                        oninput: move |evt| new_label.set(evt.value()),
                    }
                    input {
                        class: "settings__input-field",
                        placeholder: "Description (what to look for)",
                        value: "{new_description}",
                        oninput: move |evt| new_description.set(evt.value()),
                    }
                    button {
                        class: "settings__button settings__button--add",
                        onclick: add_criterion,
                        "Add Criterion"
                    }
                }
            }
        }
    }
}
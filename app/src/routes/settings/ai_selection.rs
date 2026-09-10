use dioxus::prelude::*;
use local_ai::{ModelEntry, OllamaService, PullProgress, list_models_view};
use common::setting_keys;
use storage::Storage;

#[component]
pub fn AiSelection() -> Element {
    let storage = use_context::<Storage>();

    let mut ollama_refresh = use_signal(|| 0u32);
    let ollama_models = use_resource(move || {
        let _ = ollama_refresh();
        async move { list_models_view(&reqwest::Client::new()).await }
    });

    use_hook(move || {
        spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                ollama_refresh.with_mut(|n| *n += 1);
            }
        });
    });

    let is_ollama_available = matches!(&*ollama_models.read(), Some(Ok(_)));

    let installed_models: Vec<ModelEntry> = match &*ollama_models.read() {
        Some(Ok(models)) => models.iter().filter(|m| m.installed).cloned().collect(),
        _ => Vec::new(),
    };

    let pullable_models: Vec<ModelEntry> = match &*ollama_models.read() {
        Some(Ok(models)) => models.iter().filter(|m| !m.installed).cloned().collect(),
        _ => Vec::new(),
    };

    let mut starting_ollama = use_signal(|| false);

    let start_ollama = move |_| {
        starting_ollama.set(true);
        spawn(async move {
            let service = OllamaService::new(reqwest::Client::new());
            match service.serve().await {
                Ok(_) => ollama_refresh.with_mut(|n| *n += 1),
                Err(e) => tracing::error!("failed to start ollama: {e}"),
            }
            starting_ollama.set(false);
        });
    };

    let mut pulling_model = use_signal(|| None::<String>);
    let mut pull_progress = use_signal(|| None::<f32>);
    let mut pull_error = use_signal(|| None::<String>);

    let pull_model = move |model_name: String| {
        move |_| {
            let model_name = model_name.clone();
            pulling_model.set(Some(model_name.clone()));
            pull_progress.set(Some(0.0));
            pull_error.set(None);

            spawn(async move {
                let service = OllamaService::new(reqwest::Client::new());
                let mut on_progress = move |p: PullProgress| {
                    if let Some(frac) = p.fraction() {
                        pull_progress.set(Some(frac));
                    }
                };

                match service.pull_model(&model_name, &mut on_progress).await {
                    Ok(_) => {
                        pulling_model.set(None);
                        pull_progress.set(None);
                        ollama_refresh.with_mut(|n| *n += 1);
                    }
                    Err(e) => {
                        tracing::error!("failed to pull {model_name}: {e}");
                        pull_error.set(Some(e.to_string()));
                        pulling_model.set(None);
                    }
                }
            });
        }
    };

    let mut active_model = use_signal(|| None::<String>);
    use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                if let Ok(Some(model)) = storage.get_setting(setting_keys::ACTIVE_AI_MODEL_ID).await {
                    active_model.set(Some(model));
                }
            }
        }
    });

    let set_active_model = {
        let storage = storage.clone();
        move |evt: FormEvent| {
            let storage = storage.clone();
            let value = evt.value();

            if value.trim().is_empty() {
                active_model.set(None);
                spawn(async move {
                    if let Err(e) = storage.delete_setting(setting_keys::ACTIVE_AI_MODEL_ID).await {
                        tracing::error!("failed to clear active model: {e}");
                    }
                });
                return;
            }

            active_model.set(Some(value.clone()));
            spawn(async move {
                if let Err(e) = storage.set_setting(setting_keys::ACTIVE_AI_MODEL_ID, &value).await {
                    tracing::error!("failed to save active model: {e}");
                }
            });
        }
    };

    let mut uninstalling_model = use_signal(|| None::<String>);
    let mut uninstall_error = use_signal(|| None::<String>);

    let uninstall_model = {
        let storage = storage.clone();
        move |model_name: String| {
            let storage = storage.clone();
            move |_| {
                let storage = storage.clone();
                let model_name = model_name.clone();
                uninstalling_model.set(Some(model_name.clone()));
                uninstall_error.set(None);

                spawn(async move {
                    let service = OllamaService::new(reqwest::Client::new());
                    match service.delete_model(&model_name).await {
                        Ok(_) => {
                            uninstalling_model.set(None);
                            ollama_refresh.with_mut(|n| *n += 1);

                            if active_model().as_deref() == Some(model_name.as_str()) {
                                active_model.set(None);
                                if let Err(e) = storage.delete_setting(setting_keys::ACTIVE_AI_MODEL_ID).await {
                                    tracing::error!("failed to clear active model after uninstall: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("failed to delete {model_name}: {e}");
                            uninstall_error.set(Some(e.to_string()));
                            uninstalling_model.set(None);
                        }
                    }
                });
            }
        }
    };

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                h2 { "AI Model" }
            }
            div { class: "settings__section-content",
                p { class: "settings__description",
                    "Select the local Ollama model for email classification."
                }
                div { class: "settings__select-group",
                    label { class: "settings__select-label", "Model" }
                    select {
                        class: "settings__model-select",
                        onchange: set_active_model,
                        option { value: "", selected: active_model().is_none(), "No AI" }
                        for model in &installed_models {
                            option {
                                value: "{model.name}",
                                selected: active_model().as_deref() == Some(model.name.as_str()),
                                "{model.name}"
                            }
                        }
                    }
                }
                div { class: "settings__status",
                    span { class: if is_ollama_available { "settings__status-dot settings__status-dot--online" } else { "settings__status-dot settings__status-dot--offline" } }
                    span { class: "settings__status-text",
                        if is_ollama_available {
                            "Ollama running"
                        } else {
                            "Ollama not running"
                        }
                    }
                }

                if !installed_models.is_empty() {
                    div { class: "settings__installed-list",
                        label { class: "settings__select-label", "Installed models" }
                        for entry in installed_models.clone() {
                            div { class: "settings__installed-row",
                                span { class: "settings__pull-name", "{entry.name}" }
                                if uninstalling_model().as_deref() == Some(entry.name.as_str()) {
                                    span { class: "settings__pull-progress", "Removing..." }
                                } else {
                                    button {
                                        class: "settings__button settings__button--remove settings__button--small",
                                        disabled: uninstalling_model().is_some(),
                                        onclick: uninstall_model(entry.name.clone()),
                                        "Uninstall"
                                    }
                                }
                            }
                        }
                        if let Some(err) = uninstall_error() {
                            p { class: "settings__error", "{err}" }
                        }
                    }
                }

                if !is_ollama_available {
                    button {
                        class: "settings__action-button",
                        disabled: starting_ollama(),
                        onclick: start_ollama,
                        if starting_ollama() {
                            "Starting Ollama..."
                        } else {
                            "Start Ollama"
                        }
                    }
                }

                if !pullable_models.is_empty() {
                    div { class: "settings__pull-list",
                        label { class: "settings__select-label", "Available to download" }
                        for entry in pullable_models {
                            div { class: "settings__pull-row",
                                div { class: "settings__pull-info",
                                    span { class: "settings__pull-name", "{entry.name}" }
                                    if let Some(catalog) = &entry.catalog {
                                        span { class: "settings__pull-description",
                                            "{catalog.description}"
                                        }
                                        span { class: "settings__pull-size",
                                            "~{catalog.approx_size_gb} GB"
                                        }
                                    }
                                }
                                if pulling_model().as_deref() == Some(entry.name.as_str()) {
                                    span { class: "settings__pull-progress",
                                        if let Some(frac) = pull_progress() {
                                            "{(frac * 100.0) as u32}%"
                                        } else {
                                            "Preparing..."
                                        }
                                    }
                                } else {
                                    button {
                                        class: "settings__pull-button",
                                        disabled: pulling_model().is_some() || !is_ollama_available,
                                        onclick: pull_model(entry.name.clone()),
                                        "Pull"
                                    }
                                }
                            }
                        }
                        if let Some(err) = pull_error() {
                            p { class: "settings__error", "{err}" }
                        }
                    }
                }
            }
        }
    }
}
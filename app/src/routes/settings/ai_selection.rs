//! Local AI inference engine configuration, model downloads, and process supervision.

use crate::state::{AiCommand, AppState};
use common::{ai_backends, setting_keys};
use dioxus::prelude::*;
use local_ai::builtin::{
    BUILTIN_CATALOG, GgufModelInfo, LlamaServerService, ModelDownloader, default_models_dir,
};
use local_ai::ollama::{ModelEntry, OllamaService, PullProgress, list_models_view};
use storage::Storage;

#[component]
pub fn AiSelection() -> Element {
    let storage = use_context::<Storage>();
    let state = use_context::<AppState>();
    let models_dir = default_models_dir();

    let mut viewing_tab = use_signal(|| ai_backends::BUILTIN.to_string());
    let mut active_backend = use_signal(|| ai_backends::BUILTIN.to_string());
    let mut active_model = use_signal(|| None::<String>);

    use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            async move {
                if let Ok(Some(backend)) = storage.get_setting(setting_keys::AI_BACKEND).await {
                    active_backend.set(backend);
                }
                if let Ok(Some(model)) = storage.get_setting(setting_keys::ACTIVE_AI_MODEL_ID).await
                {
                    active_model.set(Some(model));
                }
            }
        }
    });

    let mut is_builtin_online = use_signal(|| false);
    let mut disk_tick = use_signal(|| 0u32);
    let mut downloading_model = use_signal(|| None::<String>);
    let mut download_progress = use_signal(|| None::<f32>);
    let mut download_error = use_signal(|| None::<String>);

    // Built-in Health Polling: Probes 127.0.0.1:11435/health every 1.5s while active
    let builtin_poll_task = spawn(async move {
        let http_client = reqwest::Client::new();
        let service = LlamaServerService::new(http_client);

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

            if active_backend() == ai_backends::BUILTIN {
                let online = service.is_running().await;
                if is_builtin_online() != online {
                    is_builtin_online.set(online);
                }
            } else if is_builtin_online() {
                is_builtin_online.set(false);
            }
        }
    });

    let mut ollama_refresh = use_signal(|| 0u32);
    let ollama_models = use_resource(move || {
        let _ = ollama_refresh();
        let current_tab = viewing_tab();
        async move {
            if current_tab == ai_backends::OLLAMA {
                list_models_view(&reqwest::Client::new()).await
            } else {
                Ok(Vec::new())
            }
        }
    });

    // Ollama Tag Polling: Queries 127.0.0.1:11434/api/tags every 10s while tab is visible
    let ollama_poll_task = spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            if viewing_tab() == ai_backends::OLLAMA {
                ollama_refresh.with_mut(|n| *n += 1);
            }
        }
    });

    // Task Leak Prevention: Cancels both background loops cleanly when Settings unmounts
    use_drop(move || {
        builtin_poll_task.cancel();
        ollama_poll_task.cancel();
    });

    // Streaming GGUF Downloader: Streams weights directly into OS application data folder
    let download_builtin = {
        let models_dir = models_dir.clone();
        move |model: GgufModelInfo| {
            let models_dir = models_dir.clone();
            move |_| {
                let model = model.clone();
                let models_dir = models_dir.clone();
                downloading_model.set(Some(model.id.to_string()));
                download_progress.set(Some(0.0));
                download_error.set(None);

                spawn(async move {
                    let dest = models_dir.join(model.filename.as_ref());
                    let downloader = ModelDownloader::new(reqwest::Client::new());

                    let res = downloader
                        .download_model(&model.download_url, &dest, move |frac| {
                            download_progress.set(Some(frac));
                        })
                        .await;

                    match res {
                        Ok(_) => {
                            downloading_model.set(None);
                            download_progress.set(None);
                            disk_tick.with_mut(|n| *n += 1);
                        }
                        Err(e) => {
                            tracing::error!("failed to download {}: {e}", model.id);
                            download_error.set(Some(e.to_string()));
                            downloading_model.set(None);
                            download_progress.set(None);
                        }
                    }
                });
            }
        }
    };

    // Model Deletion with Process Teardown:
    // Stops the sidecar process, clears the setting in SQLite to prevent orphaned references,
    // waits for file locks to clear, and removes the .gguf file.
    let delete_builtin = {
        let models_dir = models_dir.clone();
        let storage = storage.clone();
        move |filename: String| {
            let models_dir = models_dir.clone();
            let storage = storage.clone();
            move |_| {
                let filename = filename.clone();
                let models_dir = models_dir.clone();
                let storage = storage.clone();

                spawn(async move {
                    if active_backend() == ai_backends::BUILTIN
                        && active_model().as_deref() == Some(filename.as_str())
                    {
                        active_model.set(None);
                        let _ = (state.ai_tx)().send(AiCommand::StopBuiltin);
                        let _ = storage
                            .delete_setting(setting_keys::ACTIVE_AI_MODEL_ID)
                            .await;
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    }

                    match ModelDownloader::delete_model(&models_dir, &filename) {
                        Ok(_) => {
                            tracing::info!("Successfully deleted model file {filename}");
                        }
                        Err(e) => {
                            tracing::error!("Failed to delete model file {filename}: {e}");
                        }
                    }
                    disk_tick.with_mut(|n| *n += 1);
                });
            }
        }
    };

    let is_ollama_available = matches!(&*ollama_models.read(), Some(Ok(m)) if !m.is_empty());

    let installed_ollama_models: Vec<ModelEntry> = match &*ollama_models.read() {
        Some(Ok(models)) => models.iter().filter(|m| m.installed).cloned().collect(),
        _ => Vec::new(),
    };

    let pullable_ollama_models: Vec<ModelEntry> = match &*ollama_models.read() {
        Some(Ok(models)) => models.iter().filter(|m| !m.installed).cloned().collect(),
        _ => Vec::new(),
    };

    let mut starting_ollama = use_signal(|| false);
    let start_ollama = move |_| {
        starting_ollama.set(true);
        spawn(async move {
            let service = OllamaService::new(reqwest::Client::new());
            if service.serve().await.is_ok() {
                ollama_refresh.with_mut(|n| *n += 1);
            }
            starting_ollama.set(false);
        });
    };

    let mut pulling_ollama = use_signal(|| None::<String>);
    let mut pull_ollama_progress = use_signal(|| None::<f32>);
    let mut pull_ollama_error = use_signal(|| None::<String>);

    let pull_ollama_model = move |model_name: String| {
        move |_| {
            let model_name = model_name.clone();
            pulling_ollama.set(Some(model_name.clone()));
            pull_ollama_progress.set(Some(0.0));
            pull_ollama_error.set(None);

            spawn(async move {
                let service = OllamaService::new(reqwest::Client::new());
                let mut on_progress = move |p: PullProgress| {
                    if let Some(frac) = p.fraction() {
                        pull_ollama_progress.set(Some(frac));
                    }
                };

                match service.pull_model(&model_name, &mut on_progress).await {
                    Ok(_) => {
                        pulling_ollama.set(None);
                        pull_ollama_progress.set(None);
                        ollama_refresh.with_mut(|n| *n += 1);
                    }
                    Err(e) => {
                        tracing::error!("failed to pull {model_name}: {e}");
                        pull_ollama_error.set(Some(e.to_string()));
                        pulling_ollama.set(None);
                    }
                }
            });
        }
    };

    let uninstall_ollama_model = {
        let storage = storage.clone();
        move |model_name: String| {
            let storage = storage.clone();
            move |_| {
                let storage = storage.clone();
                let model_name = model_name.clone();

                spawn(async move {
                    let service = OllamaService::new(reqwest::Client::new());
                    if service.delete_model(&model_name).await.is_ok() {
                        ollama_refresh.with_mut(|n| *n += 1);
                        if active_backend() == ai_backends::OLLAMA
                            && active_model().as_deref() == Some(model_name.as_str())
                        {
                            active_model.set(None);
                            let _ = storage
                                .delete_setting(setting_keys::ACTIVE_AI_MODEL_ID)
                                .await;
                        }
                    }
                });
            }
        }
    };

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                h2 { "AI Classification Engine" }
            }
            div { class: "settings__section-content",
                div { class: "settings__active-banner",
                    span { class: if active_model().is_some() { "settings__status-dot settings__status-dot--online" } else { "settings__status-dot settings__status-dot--offline" } }
                    span { class: "settings__status-text",
                        if let Some(m) = active_model() {
                            if active_backend() == ai_backends::BUILTIN {
                                "Active: {m} (Built-in)"
                            } else {
                                "Active: {m} (Ollama)"
                            }
                        } else {
                            "No AI Model Active (Classification Disabled)"
                        }
                    }
                }

                div { class: "settings__tab-group",
                    button {
                        class: if viewing_tab() == ai_backends::BUILTIN { "settings__ai-tab settings__ai-tab--active" } else { "settings__ai-tab" },
                        onclick: move |_| viewing_tab.set(ai_backends::BUILTIN.to_string()),
                        span { class: "settings__ai-tab-title", "Built-in" }
                        span { class: "settings__ai-tab-sub", "Zero Setup" }
                    }
                    button {
                        class: if viewing_tab() == ai_backends::OLLAMA { "settings__ai-tab settings__ai-tab--active" } else { "settings__ai-tab" },
                        onclick: move |_| viewing_tab.set(ai_backends::OLLAMA.to_string()),
                        span { class: "settings__ai-tab-title", "Ollama" }
                        span { class: "settings__ai-tab-sub", "Advanced" }
                    }
                }

                if viewing_tab() == ai_backends::BUILTIN {
                    p { class: "settings__description",
                        "Runs directly on your computer with zero terminal setup. Selecting a model here automatically makes Built-in the active classifier."
                    }

                    div { class: "settings__pull-list",
                        label { class: "settings__select-label", "Available Models" }
                        for model in BUILTIN_CATALOG.iter() {
                            {
                                let _ = disk_tick();

                                let is_downloaded = ModelDownloader::is_model_downloaded(
                                    &models_dir,
                                    &model.filename,
                                );
                                let is_active = active_backend() == ai_backends::BUILTIN
                                    && active_model().as_deref() == Some(model.filename.as_ref());
                                let is_downloading = downloading_model().as_deref() == Some(model.id.as_ref());
                                rsx! {
                                    div { key: "{model.id}", class: "settings__pull-row",
                                        div { class: "settings__pull-info",
                                            div { class: "settings__pull-title-wrap",
                                                span { class: "settings__pull-name", "{model.display_name}" }
                                                if is_active {
                                                    span { class: "settings__active-badge", "ACTIVE" }
                                                }
                                            }
                                            span { class: "settings__pull-description", "{model.description}" }
                                            span { class: "settings__pull-size", "~{model.size_mb} MB" }
                                        }

                                        div { class: "settings__pull-actions",
                                            if !is_downloaded && !is_downloading {
                                                button {
                                                    class: "settings__pull-button",
                                                    onclick: download_builtin(model.clone()),
                                                    "Download"
                                                }
                                            }

                                            if is_downloading {
                                                span { class: "settings__pull-progress",
                                                    if let Some(frac) = download_progress() {
                                                        "{(frac * 100.0) as u32}%"
                                                    } else {
                                                        "Starting..."
                                                    }
                                                }
                                            }

                                            if is_downloaded {
                                                if is_active {
                                                    button {
                                                        class: "settings__button settings__button--small",
                                                        onclick: {
                                                            let storage = storage.clone();
                                                            move |_| deselect_builtin(active_model, storage.clone(), state.ai_tx)
                                                        },
                                                        "Deselect"
                                                    }
                                                } else {
                                                    button {
                                                        class: "settings__button settings__button--small",
                                                        onclick: {
                                                            let filename = model.filename.to_string();
                                                            let storage = storage.clone();
                                                            move |_| {
                                                                select_builtin(
                                                                    filename.clone(),
                                                                    active_backend,
                                                                    active_model,
                                                                    storage.clone(),
                                                                    state.ai_tx,
                                                                )
                                                            }
                                                        },
                                                        "Select"
                                                    }
                                                }

                                                button {
                                                    class: "settings__button settings__button--remove settings__button--small",
                                                    onclick: delete_builtin(model.filename.to_string()),
                                                    "Delete"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(err) = download_error() {
                            p { class: "settings__error", "{err}" }
                        }
                    }
                }

                if viewing_tab() == ai_backends::OLLAMA {
                    p { class: "settings__description",
                        "Connects to an external Ollama daemon running on your system (port 11434)."
                    }

                    div { class: "settings__status-card",
                        div { class: "settings__status-info",
                            span { class: if is_ollama_available { "settings__status-dot settings__status-dot--online" } else { "settings__status-dot settings__status-dot--offline" } }
                            div { class: "settings__status-details",
                                span { class: "settings__status-title",
                                    if is_ollama_available {
                                        "Ollama Daemon Connected"
                                    } else {
                                        "Ollama Daemon Offline"
                                    }
                                }
                                span { class: "settings__status-subtitle", "http://127.0.0.1:11434" }
                            }
                        }
                        if !is_ollama_available {
                            button {
                                class: "settings__button settings__button--secondary settings__button--small",
                                disabled: starting_ollama(),
                                onclick: start_ollama,
                                if starting_ollama() {
                                    "Starting..."
                                } else {
                                    "Start Ollama"
                                }
                            }
                        }
                    }

                    div { class: "settings__pull-list",
                        label { class: "settings__select-label", "Installed Models" }

                        if installed_ollama_models.is_empty() {
                            p { class: "settings__empty-hint",
                                if is_ollama_available {
                                    "No models currently installed in Ollama. Pull one below to get started."
                                } else {
                                    "Start Ollama to view and manage installed models."
                                }
                            }
                        }

                        for entry in installed_ollama_models.clone() {
                            {
                                let is_active = active_backend() == ai_backends::OLLAMA
                                    && active_model().as_deref() == Some(entry.name.as_str());

                                rsx! {
                                    div { key: "{entry.name}", class: "settings__pull-row",
                                        div { class: "settings__pull-info",
                                            div { class: "settings__pull-title-wrap",
                                                span { class: "settings__pull-name", "{entry.name}" }
                                                if is_active {
                                                    span { class: "settings__active-badge", "ACTIVE" }
                                                }
                                            }
                                            if let Some(catalog) = &entry.catalog {
                                                span { class: "settings__pull-description", "{catalog.description}" }
                                                span { class: "settings__pull-size", "~{catalog.approx_size_gb} GB" }
                                            }
                                        }

                                        div { class: "settings__pull-actions",
                                            if is_active {
                                                button {
                                                    class: "settings__button settings__button--small",
                                                    onclick: {
                                                        let storage = storage.clone();
                                                        move |_| deselect_ollama(active_model, storage.clone())
                                                    },
                                                    "Deselect"
                                                }
                                            } else {
                                                button {
                                                    class: "settings__button settings__button--small",
                                                    onclick: {
                                                        let model_name = entry.name.clone();
                                                        let storage = storage.clone();
                                                        move |_| {
                                                            select_ollama(
                                                                model_name.clone(),
                                                                active_backend,
                                                                active_model,
                                                                storage.clone(),
                                                                state.ai_tx,
                                                            )
                                                        }
                                                    },
                                                    "Select"
                                                }
                                            }

                                            button {
                                                class: "settings__button settings__button--remove settings__button--small",
                                                onclick: uninstall_ollama_model(entry.name.clone()),
                                                "Uninstall"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if !pullable_ollama_models.is_empty() {
                        div { class: "settings__pull-list",
                            label { class: "settings__select-label", "Available to Download" }
                            for entry in pullable_ollama_models {
                                {
                                    let is_pulling = pulling_ollama().as_deref() == Some(entry.name.as_str());

                                    rsx! {
                                        div { key: "{entry.name}", class: "settings__pull-row",
                                            div { class: "settings__pull-info",
                                                div { class: "settings__pull-title-wrap",
                                                    span { class: "settings__pull-name", "{entry.name}" }
                                                }
                                                if let Some(catalog) = &entry.catalog {
                                                    span { class: "settings__pull-description", "{catalog.description}" }
                                                    span { class: "settings__pull-size", "~{catalog.approx_size_gb} GB" }
                                                }
                                            }

                                            div { class: "settings__pull-actions",
                                                if is_pulling {
                                                    span { class: "settings__pull-progress",
                                                        if let Some(frac) = pull_ollama_progress() {
                                                            "{(frac * 100.0) as u32}%"
                                                        } else {
                                                            "Preparing..."
                                                        }
                                                    }
                                                } else {
                                                    button {
                                                        class: "settings__pull-button",
                                                        disabled: pulling_ollama().is_some() || !is_ollama_available,
                                                        onclick: pull_ollama_model(entry.name.clone()),
                                                        "Pull"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(err) = pull_ollama_error() {
                                p { class: "settings__error", "{err}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn select_builtin(
    filename: String,
    mut active_backend: Signal<String>,
    mut active_model: Signal<Option<String>>,
    storage: Storage,
    ai_tx: Signal<crate::state::AiRequestSender>,
) {
    active_backend.set(ai_backends::BUILTIN.to_string());
    active_model.set(Some(filename.clone()));
    let _ = (ai_tx)().send(AiCommand::StartBuiltin(filename.clone()));

    spawn(async move {
        let _ = storage
            .set_setting(setting_keys::AI_BACKEND, ai_backends::BUILTIN)
            .await;
        let _ = storage
            .set_setting(setting_keys::ACTIVE_AI_MODEL_ID, &filename)
            .await;
    });
}

fn deselect_builtin(
    mut active_model: Signal<Option<String>>,
    storage: Storage,
    ai_tx: Signal<crate::state::AiRequestSender>,
) {
    active_model.set(None);
    let _ = (ai_tx)().send(AiCommand::StopBuiltin);

    spawn(async move {
        let _ = storage
            .delete_setting(setting_keys::ACTIVE_AI_MODEL_ID)
            .await;
    });
}

fn select_ollama(
    model_name: String,
    mut active_backend: Signal<String>,
    mut active_model: Signal<Option<String>>,
    storage: Storage,
    ai_tx: Signal<crate::state::AiRequestSender>,
) {
    active_backend.set(ai_backends::OLLAMA.to_string());
    active_model.set(Some(model_name.clone()));
    let _ = (ai_tx)().send(AiCommand::StopBuiltin);

    spawn(async move {
        let _ = storage
            .set_setting(setting_keys::AI_BACKEND, ai_backends::OLLAMA)
            .await;
        let _ = storage
            .set_setting(setting_keys::ACTIVE_AI_MODEL_ID, &model_name)
            .await;
    });
}

fn deselect_ollama(mut active_model: Signal<Option<String>>, storage: Storage) {
    active_model.set(None);

    spawn(async move {
        let _ = storage
            .delete_setting(setting_keys::ACTIVE_AI_MODEL_ID)
            .await;
    });
}
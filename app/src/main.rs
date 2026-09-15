//! Application entry point, window lifecycle management, and root service orchestration.

mod routes;
mod state;
mod tray;
mod updater;
pub mod utils;

use dioxus::{
    core::Task,
    desktop::{
        Config, WindowBuilder, WindowCloseBehaviour, tao::window::Icon as TaoIcon, use_window,
    },
    prelude::*,
};
use std::{
    collections::HashMap,
    process::Child,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::mpsc;
use tracing::error;
use uuid::Uuid;

use common::{APP_SERVICE_NAME, config::POLL_INTERVAL_SECS, setting_keys};
use email_core::{
    SyncProgress, SyncService, link_gmail_account, link_imap_account, link_outlook_account,
};
use local_ai::builtin::{LlamaServerService, default_models_dir, find_llama_server_binary};
use routes::{AddAccount, Dashboard, Findings, Inbox, Layout, Settings, Trash};
use state::{AiCommand, AppState, AuthCommand, SyncTarget};
use storage::{Storage, init_pool};
use tray::use_system_tray;
use utils::load_custom_css;

/// Bundled CSS stylesheet concatenating base layout, theme surfaces, and screen-specific styling.
const ALL_CSS: &str = concat!(
    include_str!("../assets/css/default/shared.css"),
    "\n",
    include_str!("../assets/css/default/layout.css"),
    "\n",
    include_str!("../assets/css/default/dashboard.css"),
    "\n",
    include_str!("../assets/css/default/settings.css"),
    "\n",
    include_str!("../assets/css/default/add_account.css"),
    "\n",
    include_str!("../assets/css/default/inbox.css"),
    "\n",
    include_str!("../assets/css/default/findings.css"),
    "\n",
    include_str!("../assets/css/default/trash.css"),
);

const DB_NAME: &str = "fumiko.db";

/// Global handle to the SQLite connection pool and credential store.
static STORAGE: OnceLock<Storage> = OnceLock::new();

/// Global process registry tracking the active `llama-server` sidecar child process.
static ACTIVE_CHILD_PROCESS: OnceLock<Arc<Mutex<Option<Child>>>> = OnceLock::new();

/// Application route hierarchy managed by Dioxus Router.
#[derive(Routable, Clone, PartialEq)]
enum Route {
    #[layout(Layout)]
    #[route("/")]
    Dashboard {},

    #[route("/inbox")]
    Inbox {},

    #[route("/findings")]
    Findings {},

    #[route("/add_account")]
    AddAccount {},

    #[route("/settings")]
    Settings {},

    #[route("/trash")]
    Trash {},
}

/// Loads and decodes the embedded PNG application window icon.
fn load_window_icon() -> TaoIcon {
    let bytes = include_bytes!("../assets/png/logo120x120.png");
    let image = image::load_from_memory(bytes)
        .expect("Failed to load window icon image")
        .into_rgba8();

    let (width, height) = image.dimensions();
    let rgba_bytes = image.into_raw();

    TaoIcon::from_rgba(rgba_bytes, width, height).expect("Failed to create tao window icon")
}

/// Retrieves the shared thread-safe handle for supervising the built-in AI process.
pub fn get_child_process_handle() -> Arc<Mutex<Option<Child>>> {
    ACTIVE_CHILD_PROCESS
        .get_or_init(|| Arc::new(Mutex::new(None)))
        .clone()
}

/// Terminates the running `llama-server` process to prevent orphaned background instances.
///
/// Must be invoked explicitly prior to exiting because `std::process::exit(0)` bypasses
/// standard Rust stack unwinding and `Drop` handlers.
pub fn kill_builtin_server() {
    if let Some(arc) = ACTIVE_CHILD_PROCESS.get() {
        if let Ok(mut lock) = arc.lock() {
            if let Some(mut child) = lock.take() {
                let _ = child.kill();
                let _ = child.wait();
                tracing::info!("Successfully killed built-in llama-server process on exit");
            }
        }
    }
}

fn main() {
    let data_dir = dirs::data_local_dir()
        .expect("Could not determine application data directory")
        .join(APP_SERVICE_NAME);

    std::fs::create_dir_all(&data_dir).expect("Could not create application data directory");

    let db_path = data_dir.join(DB_NAME);
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    // Initialize local SQLite storage and run pending migrations
    let storage = runtime.block_on(async {
        let pool = init_pool(db_path.to_str().expect("Invalid database path"))
            .await
            .expect("Failed to initialize database");

        let storage = Storage::new(pool);

        if let Err(e) = storage.seed_default_criteria().await {
            error!("Failed to seed default criteria: {e}");
        }

        storage
    });

    STORAGE
        .set(storage)
        .expect("Storage has already been initialized");

    let window_icon = load_window_icon();

    LaunchBuilder::new()
        .with_cfg(
            Config::new()
                .with_menu(None)
                .with_window(
                    WindowBuilder::new()
                        .with_title("ShuhariTech | Fumiko")
                        .with_window_icon(Some(window_icon)),
                )
                // Hides window on close button rather than terminating, keeping inbox watchers active
                .with_close_behaviour(WindowCloseBehaviour::WindowHides)
                // Link Trapping: Intercepts webview navigations and routes external protocols
                // to the default system browser, returning false to prevent third-party URLs
                // from navigating or replacing the desktop application window.
                .with_navigation_handler(|url| {
                    if url.starts_with("http://")
                        || url.starts_with("https://")
                        || url.starts_with("mailto:")
                    {
                        let _ = webbrowser::open(url);
                        false
                    } else {
                        true
                    }
                }),
        )
        .launch(Fumiko);

    // Fallback process teardown if the desktop window loop exits directly
    kill_builtin_server();
}

/// Root application component housing global state, background workers, and routing.
#[component]
fn Fumiko() -> Element {
    let window = use_window();
    let storage = STORAGE.get().expect("storage has not been initialized");
    use_context_provider(|| storage.clone());

    let active_custom_css = use_signal(|| load_custom_css());
    use_context_provider(|| active_custom_css);

    // Channel Initializations:
    // Retaining unbounded channels in the root scope guarantees that background sync loops,
    // OAuth authorization listeners, and AI supervision jobs survive view navigation.
    let sync_channel = use_signal(|| {
        let (tx, rx) = mpsc::unbounded_channel::<SyncTarget>();
        (tx, Some(rx))
    });
    let sync_tx = sync_channel.read().0.clone();

    let auth_channel = use_signal(|| {
        let (tx, rx) = mpsc::unbounded_channel::<AuthCommand>();
        (tx, Some(rx))
    });
    let auth_tx = auth_channel.read().0.clone();

    let ai_channel = use_signal(|| {
        let (tx, rx) = mpsc::unbounded_channel::<AiCommand>();
        (tx, Some(rx))
    });
    let ai_tx = ai_channel.read().0.clone();

    // Injects fine-grained reactive state into the component hierarchy
    let mut state = use_context_provider(|| AppState::new(sync_tx.clone(), auth_tx, ai_tx.clone()));

    let active_child_process = use_hook(get_child_process_handle);

    // System Tray Integration: Registers menu handlers for window toggling and quit cleanup
    use_system_tray(window, sync_tx.clone(), active_child_process.clone());

    // Central Background Sync Worker:
    // Consumes SyncTarget commands, forwards progress events, purges expired trash,
    // and invalidates reactive database queries upon completion.
    use_hook({
        let storage = storage.clone();
        let mut sync_channel = sync_channel;

        move || {
            let storage = storage.clone();
            let mut sync_rx = sync_channel
                .write()
                .1
                .take()
                .expect("Sync worker has already been initialized");

            let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<SyncProgress>();

            spawn(async move {
                while let Some(progress) = progress_rx.recv().await {
                    state.sync_progress.set(Some(progress));
                }
            });

            spawn(async move {
                let http_client = reqwest::Client::new();

                while let Some(target) = sync_rx.recv().await {
                    state.is_syncing.set(true);

                    let sync_service = SyncService::new(storage.clone(), http_client.clone())
                        .with_progress_sender(progress_tx.clone());

                    let result = match target {
                        SyncTarget::One(account_id) => sync_service.sync_account(account_id).await,
                        SyncTarget::All => sync_service.sync_all().await,
                    };

                    if let Err(e) = result {
                        error!("sync failed: {e}");
                    }

                    let retention = storage
                        .get_setting(setting_keys::TRASH_RETENTION_DAYS)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|val| val.parse::<i64>().ok())
                        .unwrap_or(30);

                    let _ = storage.purge_expired_trash(retention).await;

                    // Trigger downstream query invalidation and update mailbox badges
                    state.refresh_trigger.with_mut(|n| *n += 1);
                    state.sync_tick.with_mut(|n| *n += 1);
                    state.is_syncing.set(false);
                    state.sync_progress.set(None);
                }
            });

            // Trigger an initial synchronization across all linked accounts on startup
            let _ = sync_channel.read().0.send(SyncTarget::All);
        }
    });

    // Root Authentication Worker:
    // Coordinates OAuth browser loops and IMAP connection tests.
    // Cancels any in-flight task when a new authorization or cancel command is received,
    // triggering the ListenerGuard to release local sockets immediately.
    use_hook({
        let storage = storage.clone();
        let mut auth_channel = auth_channel;

        move || {
            let storage = storage.clone();
            let mut auth_rx = auth_channel
                .write()
                .1
                .take()
                .expect("Auth worker has already been initialized");

            spawn(async move {
                let mut active_task: Option<Task> = None;

                while let Some(command) = auth_rx.recv().await {
                    match command {
                        AuthCommand::Start(provider) => {
                            if let Some(task) = active_task.take() {
                                task.cancel();
                            }

                            state.is_linking.set(true);
                            let storage = storage.clone();

                            let task = spawn(async move {
                                match provider {
                                    common::Provider::Gmail => {
                                        state.linking_status.set(
                                            "Opening browser for Google sign-in...".to_string(),
                                        );
                                        match link_gmail_account(&storage).await {
                                            Ok(account_id) => {
                                                state
                                                    .linking_status
                                                    .set("Gmail connected! Syncing...".to_string());
                                                state.refresh_trigger.with_mut(|n| *n += 1);
                                                let _ = (state.sync_tx)()
                                                    .send(SyncTarget::One(account_id));
                                            }
                                            Err(e) => {
                                                state
                                                    .linking_status
                                                    .set(format!("Failed to link Gmail: {e}"));
                                            }
                                        }
                                    }
                                    common::Provider::Outlook => {
                                        state.linking_status.set(
                                            "Opening browser for Microsoft sign-in...".to_string(),
                                        );
                                        match link_outlook_account(&storage).await {
                                            Ok(account_id) => {
                                                state.linking_status.set(
                                                    "Outlook connected! Syncing...".to_string(),
                                                );
                                                state.refresh_trigger.with_mut(|n| *n += 1);
                                                let _ = (state.sync_tx)()
                                                    .send(SyncTarget::One(account_id));
                                            }
                                            Err(e) => {
                                                state
                                                    .linking_status
                                                    .set(format!("Failed to link Outlook: {e}"));
                                            }
                                        }
                                    }
                                    common::Provider::Imap => {
                                        state
                                            .linking_status
                                            .set("IMAP accounts require credentials.".to_string());
                                    }
                                }
                                state.is_linking.set(false);
                            });

                            active_task = Some(task);
                        }
                        AuthCommand::StartImap {
                            email,
                            password,
                            host,
                            port,
                        } => {
                            if let Some(task) = active_task.take() {
                                task.cancel();
                            }

                            state.is_linking.set(true);
                            state
                                .linking_status
                                .set("Connecting to IMAP server...".to_string());
                            let storage = storage.clone();

                            let task = spawn(async move {
                                match link_imap_account(&storage, &email, &password, &host, port)
                                    .await
                                {
                                    Ok(account_id) => {
                                        state
                                            .linking_status
                                            .set("IMAP account connected! Syncing...".to_string());
                                        state.refresh_trigger.with_mut(|n| *n += 1);
                                        let _ = (state.sync_tx)().send(SyncTarget::One(account_id));
                                    }
                                    Err(e) => {
                                        state
                                            .linking_status
                                            .set(format!("Failed to link IMAP: {e}"));
                                    }
                                }
                                state.is_linking.set(false);
                            });

                            active_task = Some(task);
                        }
                        AuthCommand::Cancel => {
                            if let Some(task) = active_task.take() {
                                task.cancel();
                            }
                            state.is_linking.set(false);
                            state
                                .linking_status
                                .set("Connection attempt cancelled.".to_string());
                        }
                    }
                }
            });
        }
    });

    // Startup Update Checker: Runs GitHub release check in a non-blocking background task
    use_hook(|| {
        spawn(async move {
            if let Ok(Some(new_ver)) = updater::check_for_update().await {
                state.available_update.set(Some(new_ver));
            }
        });
    });

    // Reactive Account Loader: Refetches linked accounts from SQLite whenever refresh_trigger increments
    use_resource({
        let storage = storage.clone();
        move || {
            let storage = storage.clone();
            let _ = (state.refresh_trigger)();
            async move {
                if let Ok(accounts_from_db) = storage.list_accounts().await {
                    state.accounts.set(accounts_from_db);
                }
            }
        }
    });

    // Supervised Account Watchers:
    // Maintains an isolated background polling task for each active mailbox.
    // When an account is removed from storage, watchers.retain detects the missing UUID,
    // cancels the running task, and drops it to prevent ghost polling loops.
    let mut running_watchers = use_signal(HashMap::<Uuid, Task>::new);
    use_effect(move || {
        let current_accounts = (state.accounts)();
        let current_ids: std::collections::HashSet<Uuid> =
            current_accounts.iter().map(|a| a.id).collect();

        running_watchers.with_mut(|watchers| {
            for account in &current_accounts {
                if !watchers.contains_key(&account.id) {
                    let sync_tx = sync_tx.clone();
                    let account_id = account.id;

                    let task = spawn(async move {
                        tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
                        loop {
                            let _ = sync_tx.send(SyncTarget::One(account_id));
                            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
                        }
                    });

                    watchers.insert(account_id, task);
                }
            }

            watchers.retain(|id, task| {
                if !current_ids.contains(id) {
                    task.cancel();
                    false
                } else {
                    true
                }
            });
        });
    });

    // Built-in Local AI Supervisor:
    // Manages the lifecycle of the bundled llama-server sidecar process.
    // Terminates any existing instance before launching a replacement model,
    // ensuring single-slot memory constraints remain intact.
    use_hook({
        let storage = storage.clone();
        let mut ai_channel = ai_channel;
        let active_child = active_child_process.clone();

        move || {
            let storage = storage.clone();
            let mut ai_rx = ai_channel
                .write()
                .1
                .take()
                .expect("AI worker already initialized");

            let initial_tx = ai_tx.clone();
            let active_child = active_child.clone();

            spawn(async move {
                let http_client = reqwest::Client::new();
                let server_service = LlamaServerService::new(http_client);

                let backend = storage
                    .get_setting(setting_keys::AI_BACKEND)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "builtin".to_string());

                let active_model = storage
                    .get_setting(setting_keys::ACTIVE_AI_MODEL_ID)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();

                if backend == "builtin" && !active_model.is_empty() {
                    let _ = initial_tx.send(AiCommand::StartBuiltin(active_model));
                }

                while let Some(command) = ai_rx.recv().await {
                    match command {
                        AiCommand::StartBuiltin(filename) => {
                            if let Ok(mut lock) = active_child.lock() {
                                if let Some(mut child) = lock.take() {
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    tokio::time::sleep(Duration::from_millis(200)).await;
                                }
                            }

                            let model_path = default_models_dir().join(&filename);
                            if let Some(binary) = find_llama_server_binary() {
                                match server_service.start_process(&binary, &model_path) {
                                    Ok(child) => {
                                        tracing::info!(
                                            "Spawned built-in llama-server on 127.0.0.1:11435 for {filename}"
                                        );
                                        if let Ok(mut lock) = active_child.lock() {
                                            *lock = Some(child);
                                        }
                                    }
                                    Err(e) => tracing::error!("Failed to spawn llama-server: {e}"),
                                }
                            } else {
                                tracing::error!("Could not locate llama-server executable");
                            }
                        }
                        AiCommand::StopBuiltin => {
                            if let Ok(mut lock) = active_child.lock() {
                                if let Some(mut child) = lock.take() {
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    tracing::info!("Terminated built-in llama-server process");
                                }
                            }
                        }
                    }
                }
            });
        }
    });

    rsx! {
        style { "{ALL_CSS}" }

        if let Some(css) = (active_custom_css)() {
            style { "{css}" }
        }

        Router::<Route> {}
    }
}
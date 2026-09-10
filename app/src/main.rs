mod routes;
mod state;
mod tray;
mod updater;
pub mod utils;

use dioxus::{
    core::Task,
    desktop::{
        tao::window::Icon as TaoIcon,
        Config, WindowBuilder, WindowCloseBehaviour, use_window
    },
    prelude::*,
};
use std::{collections::HashMap, sync::OnceLock, time::Duration};
use common::{APP_SERVICE_NAME, setting_keys::POLL_INTERVAL_SECS};
use email_core::SyncService;
use storage::{Storage, init_pool};
use tokio::sync::mpsc;
use tracing::error;
use uuid::Uuid;

use routes::{AddAccount, Dashboard, Findings, Inbox, Layout, Settings, Trash};
use state::SyncTarget;
use tray::use_system_tray;
use utils::load_custom_css;

use email_core::{link_gmail_account, link_outlook_account};
use state::{AuthCommand, AppState};

static SHARED_CSS: Asset = asset!("/assets/css/default/shared.css");
static LAYOUT_CSS: Asset = asset!("/assets/css/default/layout.css");
static DASHBOARD_CSS: Asset = asset!("/assets/css/default/dashboard.css");
static SETTINGS_CSS: Asset = asset!("/assets/css/default/settings.css");
static ADD_ACCOUNT_CSS: Asset = asset!("/assets/css/default/add_account.css");
static INBOX_CSS: Asset = asset!("/assets/css/default/inbox.css");
static FINDINGS_CSS: Asset = asset!("/assets/css/default/findings.css");
static TRASH_CSS: Asset = asset!("/assets/css/default/trash.css");

const DB_NAME: &str = "fumiko.db";
static STORAGE: OnceLock<Storage> = OnceLock::new();

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

fn load_window_icon() -> TaoIcon {
    let bytes = include_bytes!("../assets/png/logo120x120.png");
    let image = image::load_from_memory(bytes)
        .expect("Failed to load window icon image")
        .into_rgba8();

    let (width, height) = image.dimensions();
    let rgba_bytes = image.into_raw();

    TaoIcon::from_rgba(rgba_bytes, width, height)
        .expect("Failed to create tao window icon")
}

fn main() {
    let data_dir = dirs::data_local_dir()
        .expect("Could not determine application data directory")
        .join(APP_SERVICE_NAME);

    std::fs::create_dir_all(&data_dir).expect("Could not create application data directory");

    let db_path = data_dir.join(DB_NAME);

    let pool = tokio::runtime::Runtime::new()
        .expect("Failed to create Tokio runtime")
        .block_on(init_pool(db_path.to_str().expect("Invalid database path")))
        .expect("Failed to initialize database");

    STORAGE
        .set(Storage::new(pool))
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
                .with_close_behaviour(WindowCloseBehaviour::WindowHides),
        )
        .launch(Fumiko);
}

#[component]
fn Fumiko() -> Element {
    let window = use_window();
    let storage = STORAGE.get().expect("storage has not been initialized");
    use_context_provider(|| storage.clone());

    // 1. Create Sync & Auth Channels
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

    // 2. Provide AppState
    let mut state = use_context_provider(|| AppState::new(sync_tx.clone(), auth_tx));

    let custom_css = use_signal(load_custom_css);

    // 3. System Tray Integration
    use_system_tray(window, sync_tx.clone());

    // 4. Central Background Sync Worker
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

            spawn(async move {
                let http_client = reqwest::Client::new();

                while let Some(target) = sync_rx.recv().await {
                    state.is_syncing.set(true);

                    let sync_service = SyncService::new(storage.clone(), http_client.clone());

                    let result = match target {
                        SyncTarget::One(account_id) => sync_service.sync_account(account_id).await,
                        SyncTarget::All => sync_service.sync_all().await,
                    };

                    if let Err(e) = result {
                        error!("sync failed: {e}");
                    }

                    state.refresh_trigger.with_mut(|n| *n += 1);
                    state.sync_tick.with_mut(|n| *n += 1);
                    state.is_syncing.set(false);
                }
            });

            let _ = sync_channel.read().0.send(SyncTarget::All);
        }
    });

    // 5. Persistent Root Auth Worker (Lives in root scope, survives route navigation!)
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
                                        state.linking_status.set("Opening browser for Google sign-in...".to_string());
                                        match link_gmail_account(&storage).await {
                                            Ok(account_id) => {
                                                state.linking_status.set("Gmail connected! Syncing...".to_string());
                                                state.refresh_trigger.with_mut(|n| *n += 1);
                                                let _ = (state.sync_tx)().send(SyncTarget::One(account_id));
                                            }
                                            Err(e) => {
                                                state.linking_status.set(format!("Failed to link Gmail: {e}"));
                                            }
                                        }
                                    }
                                    common::Provider::Outlook => {
                                        state.linking_status.set("Opening browser for Microsoft sign-in...".to_string());
                                        match link_outlook_account(&storage).await {
                                            Ok(account_id) => {
                                                state.linking_status.set("Outlook connected! Syncing...".to_string());
                                                state.refresh_trigger.with_mut(|n| *n += 1);
                                                let _ = (state.sync_tx)().send(SyncTarget::One(account_id));
                                            }
                                            Err(e) => {
                                                state.linking_status.set(format!("Failed to link Outlook: {e}"));
                                            }
                                        }
                                    }
                                }
                                state.is_linking.set(false);
                            });

                            active_task = Some(task);
                        }
                        AuthCommand::Cancel => {
                            if let Some(task) = active_task.take() {
                                task.cancel(); // Drops future -> runs ListenerGuard -> releases port immediately
                            }
                            state.is_linking.set(false);
                            state.linking_status.set("Connection attempt cancelled.".to_string());
                        }
                    }
                }
            });
        }
    });

    // 6. Startup Update Checker
    use_hook(|| {
        spawn(async move {
            match updater::check_for_update().await {
                Ok(Some(new_ver)) => {
                    state.available_update.set(Some(new_ver));
                }
                _ => {}
            }
        });
    });

    // 7. Reactive Account List Loader
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

    // 8. Account Polling Watchers
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

    rsx! {
        document::Stylesheet { href: SHARED_CSS }
        document::Stylesheet { href: LAYOUT_CSS }
        document::Stylesheet { href: DASHBOARD_CSS }
        document::Stylesheet { href: SETTINGS_CSS }
        document::Stylesheet { href: ADD_ACCOUNT_CSS }
        document::Stylesheet { href: INBOX_CSS }
        document::Stylesheet { href: FINDINGS_CSS }
        document::Stylesheet { href: TRASH_CSS }

        if let Some(css) = custom_css() {
            style { "{css}" }
        }

        Router::<Route> {}
    }
}
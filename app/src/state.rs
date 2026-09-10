use common::Provider;
use dioxus::prelude::*;
use storage::models::LinkedAccount;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncTarget {
    One(Uuid),
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCommand {
    Start(Provider),
    Cancel,
}

pub type SyncRequestSender = mpsc::UnboundedSender<SyncTarget>;
pub type AuthRequestSender = mpsc::UnboundedSender<AuthCommand>;

/// Global application state container.
#[derive(Clone, Copy)]
pub struct AppState {
    pub accounts: Signal<Vec<LinkedAccount>>,
    pub selected_account: Signal<Option<Uuid>>,
    pub selected_email: Signal<Option<Uuid>>,
    pub refresh_trigger: Signal<u32>,
    pub sync_tick: Signal<u64>,
    pub is_syncing: Signal<bool>,
    pub available_update: Signal<Option<String>>,
    pub sync_tx: Signal<SyncRequestSender>,
    pub auth_tx: Signal<AuthRequestSender>,
    pub is_linking: Signal<bool>,
    pub linking_status: Signal<String>,
}

impl AppState {
    pub fn new(sync_tx: SyncRequestSender, auth_tx: AuthRequestSender) -> Self {
        Self {
            accounts: Signal::new(Vec::new()),
            selected_account: Signal::new(None),
            selected_email: Signal::new(None),
            refresh_trigger: Signal::new(0),
            sync_tick: Signal::new(0),
            is_syncing: Signal::new(false),
            available_update: Signal::new(None),
            sync_tx: Signal::new(sync_tx),
            auth_tx: Signal::new(auth_tx),
            is_linking: Signal::new(false),
            linking_status: Signal::new(String::new()),
        }
    }
}
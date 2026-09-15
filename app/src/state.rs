//! Global application state container and background worker command types.

use common::Provider;
use dioxus::prelude::*;
use email_core::SyncProgress;
use storage::models::LinkedAccount;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Destination targets dispatched to the background sync coordinator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncTarget {
    /// Synchronize a single mailbox by account UUID.
    One(Uuid),
    /// Sequentially synchronize all registered accounts.
    All,
}

/// Commands dispatched to the root authentication worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCommand {
    /// Initiates an OAuth 2.0 PKCE loopback authorization flow.
    Start(Provider),
    /// Tests IMAP credentials and verifies TLS connectivity before linking.
    StartImap {
        email: String,
        password: String,
        host: String,
        port: u16,
    },
    /// Aborts active connection attempts and unbinds local sockets.
    Cancel,
}

/// Commands dispatched to the background local AI supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiCommand {
    /// Spawns the bundled llama-server sidecar with the specified model file.
    StartBuiltin(String),
    /// Terminates the running sidecar process to release system RAM.
    StopBuiltin,
}

pub type SyncRequestSender = mpsc::UnboundedSender<SyncTarget>;
pub type AuthRequestSender = mpsc::UnboundedSender<AuthCommand>;
pub type AiRequestSender = mpsc::UnboundedSender<AiCommand>;

/// Global application state container providing fine-grained reactive signals.
///
/// Implements `Copy` to permit passing through Dioxus component contexts without cloning.
/// Using discrete signals rather than a monolithic struct ensures that updating a single
/// counter or progress frame does not trigger full-screen re-renders.
#[derive(Clone, Copy)]
pub struct AppState {
    /// Cached list of active linked accounts.
    pub accounts: Signal<Vec<LinkedAccount>>,
    /// Currently selected account filter (None represents "All Inboxes").
    pub selected_account: Signal<Option<Uuid>>,
    /// Active email UUID loaded into the reading pane split-view.
    pub selected_email: Signal<Option<Uuid>>,
    /// Monotonic counter bumped to invalidate downstream database queries across views.
    pub refresh_trigger: Signal<u32>,
    /// Counter bumped on sync completion to refresh relative timestamps and badges.
    pub sync_tick: Signal<u64>,
    /// Global flag indicating whether background inbox synchronization is active.
    pub is_syncing: Signal<bool>,
    /// Available release version tag if an update is discovered.
    pub available_update: Signal<Option<String>>,
    /// Channel handle for dispatching sync requests.
    pub sync_tx: Signal<SyncRequestSender>,
    /// Channel handle for dispatching account link and cancellation commands.
    pub auth_tx: Signal<AuthRequestSender>,
    /// Channel handle for managing the built-in AI sidecar process.
    pub ai_tx: Signal<AiRequestSender>,
    /// Flag indicating whether an account authentication handshake is in progress.
    pub is_linking: Signal<bool>,
    /// Real-time status message displayed during account linking loops.
    pub linking_status: Signal<String>,
    /// Granular sync progress tracking item counts and current phase.
    pub sync_progress: Signal<Option<SyncProgress>>,
}

impl AppState {
    /// Creates a new `AppState` instance with initialized reactive signals.
    pub fn new(
        sync_tx: SyncRequestSender,
        auth_tx: AuthRequestSender,
        ai_tx: AiRequestSender,
    ) -> Self {
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
            ai_tx: Signal::new(ai_tx),
            is_linking: Signal::new(false),
            linking_status: Signal::new(String::new()),
            sync_progress: Signal::new(None),
        }
    }
}
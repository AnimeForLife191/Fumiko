# Desktop Application and UI Architecture

The `app` crate serves as the presentation and orchestration layer for Fumiko. Built with Dioxus and Tao, it combines a reactive desktop user interface with background service coordination, local AI supervision, and native desktop integration.

Unlike a standard web frontend, a desktop email watcher operates as a persistent daemon. It must coordinate background inbox polling, manage operating system lifecycles (system tray, window visibility, native keyrings), safely render arbitrary third-party HTML, and manage on-device LLM downloads, all while keeping the UI responsive.

This document details the architecture of the `app` crate, explains our reactive state and worker model, breaks down our HTML sanitization pipeline, and outlines the frontend invariants we maintain.

---

## What We Are Solving

Desktop webview applications face several engineering challenges that standard web apps do not:

* **Route Navigation Tearing**: In component-based frameworks, navigating between views unmounts component trees. If an OAuth handshake or background sync is tied to a specific route component (like `AddAccount`), navigating back to the dashboard will abort the future and leave network sockets hanging.
* **Task Leaks from Background Polling**: Spawning unmanaged background loops to poll email accounts causes task leaks when accounts are deleted, leading to ghost sync passes and wasted provider API quotas.
* **The Email Sanitization Dilemma**: Naive HTML sanitizers strip all `<style>` elements, destroying email layouts, receipts, and invoices. Leaving styles unvetted, however, exposes the desktop webview to CSS data exfiltration (`@import`) and script injection (`javascript:` expressions).
* **Accidental Process Termination**: If closing the application window terminates the binary, background inbox monitoring stops completely, defeating the purpose of an email watcher.
* **Orphaned Configuration References**: Deleting an installed local AI model from disk while leaving it marked as the active classifier in SQLite causes background sync jobs to fail silently.
* **UI Thread Starvation**: GitHub release checks, binary file replacement, and token cryptography will freeze the user interface if executed directly on the main event loop.

---

## View Architecture

Fumiko uses Dioxus `Router` with an overarching `Layout` component that maintains a fixed 250px sidebar, account filter controls, and an animated top update banner:

* **Dashboard (`Dashboard`)**: Displays true mailbox stats (unread, findings, total counts queried from SQLite), a 5-item preview of recent messages and findings, and active watch criteria toggle switches.
* **Inbox (`Inbox`)**: A responsive split-view interface (`minmax(340px, 400px) minmax(0, 1fr)`). The left pane displays a 3-tier email list with a manual sync trigger; the right pane displays the active `ReadingPane`.
* **Findings Board (`Findings`)**: A prioritized stream of emails that matched user-defined watch criteria. Displays model confidence percentages, with instant "Clear Match" and "Delete" triage buttons.
* **Trash (`Trash`)**: Displays soft-deleted emails with timestamps and an instant "Restore" action.
* **Add Account (`AddAccount`)**: Guides provider selection (Gmail or Outlook), displays real-time connection status, and houses the `Credentials` component for custom developer keys.
* **Settings (`Settings`)**: Divided into five focused submodules:
  * `LinkedAccountsSection`: Displays connected accounts with inline editable names (`EditableAccountName`), error indicators, and deletion actions.
  * `TrashRetentionSection`: Configures automatic trash retention periods (7 to 365 days).
  * `AiSelection`: Supervises the local Ollama daemon, lists installed vs. pullable models, streams download progress, and manages active model selection.
  * `WatchCriteriaSection`: CRUD management for user-defined classification rules.
  * `DangerZoneSection`: Two-step confirmation dialog that executes `storage.wipe_all_data()`.

---

## State Management and Reactive Invalidation (`AppState`)

Fumiko decouples persistent relational storage (SQLite) from ephemeral UI state using `AppState`:

### Signal Hierarchy
`AppState` is injected into the root component context and exposes granular signals:
* `accounts`: Cached vector of active `LinkedAccount` rows.
* `selected_account`: Global account filter pointer (`None` represents "All Accounts").
* `selected_email`: Currently active email UUID for split-view reading.
* `is_syncing` & `is_linking`: Boolean flags that drive loading spinners and disable conflicting trigger buttons.
* `refresh_trigger`: Global invalidation counter. Bumping this signal invalidates downstream `use_resource` hooks across all screens, triggering clean database re-queries.
* `sync_tick`: Counter bumped whenever background sync passes finish, refreshing timestamps and mailbox stats.

### Decoupled Worker Channels
To prevent UI code from running asynchronous tasks directly inside event handlers, state changes communicate with background workers via unbounded Tokio MPSC channels:
* `sync_tx: UnboundedSender<SyncTarget>`: Receives `SyncTarget::One(Uuid)` or `SyncTarget::All` commands.
* `auth_tx: UnboundedSender<AuthCommand>`: Receives `AuthCommand::Start(Provider)` or `AuthCommand::Cancel` commands.

---

## Background Worker Lifecycles

All long-running services live in the **root component scope** (`Fumiko`), guaranteeing that background operations survive view navigation.

### 1. Root Authentication Worker
The OAuth authorization lifecycle requires listening on an ephemeral loopback socket (`127.0.0.1:0`) while the user authenticates in their browser.
* **Navigation Resilience**: Because the receiver `auth_rx` is managed inside a root `use_hook`, the user can navigate away from `AddAccount` without aborting the handshake.
* **Deterministic Port Release**: When `AuthCommand::Cancel` is received (or a new authorization flow begins), the previous `Task` is explicitly cancelled via `task.cancel()`. Cancelling the task drops the underlying future, which executes the OAuth listener's drop guard and immediately unbinds the local TCP socket.

### 2. Supervised Account Watchers
Fumiko watches multiple inboxes concurrently using a supervised task pool:
* The root component maintains a `running_watchers: Signal<HashMap<Uuid, Task>>`.
* A `use_effect` monitors changes to the `accounts` signal against the watcher map.
* When a new account is linked, an isolated task is spawned to poll the provider every `POLL_INTERVAL_SECS`.
* **Pruning and Cleanup**: If an account is removed from SQLite, `watchers.retain()` detects that its UUID is no longer in `accounts`, calls `task.cancel()`, and drops the handle, preventing duplicate or ghost polling loops.

### 3. Central Sync Worker
The sync worker serializes sync requests received over `sync_channel`:
* Sets `is_syncing` to `true`.
* Instantiates `SyncService` and dispatches the sync pass to `email_core`.
* Increments `refresh_trigger` and `sync_tick`, updating all visible email lists and dashboard stats.
* Resets `is_syncing` back to `false`.

---

## Lazy Message Loading and HTML Sanitization

### Lazy Full-Message Retrieval
To keep mailbox lists lightweight and save local bandwidth, message bodies and raw MIME attachments are not downloaded during background sync. 

In `ReadingPane`:
1. When a user clicks an email, `use_resource` queries SQLite for the message headers and account record.
2. It requests fresh access credentials via `get_access_token_for_account`.
3. The dynamic `EmailProvider` trait fetches the full payload (`fetch_full_message`) on-demand.
4. Once fetched, the email is marked as viewed in storage, and `refresh_trigger` is bumped to update read status badges across the UI.

### The Three-Stage Sanitization Pipeline (`utils::sanitize_html`)
Rendering untrusted HTML emails in a desktop webview presents severe security risks. Fumiko implements a three-stage sanitization pipeline:

1. **Regex Style Extraction and Cleanse**: Before Ammonia strips `<style>` elements, a regular expression extracts all style blocks. `javascript:` protocol strings are purged, and `@import` directives are commented out to prevent CSS-based data exfiltration.
2. **Ammonia Structural Sanitization**: The body markup is filtered through `ammonia::Builder`. Permitted URL schemes are restricted to `data`, `cid`, `http`, `https`, `mailto`, and `tel`. All `<script>`, `<object>`, `<embed>`, and `<iframe>` elements are stripped, and inline event handlers (`onload`, `onclick`) are purged. Hyperlinks receive `rel="noopener noreferrer"` and `target="_blank"`.
3. **Sandboxed Document Reassembly**: The sanitized body and cleaned styles are reassembled into an HTML5 document. The wrapper enforces an explicit `#ffffff` background and dark text resets to prevent contrast inversion bugs in dark mode. The markup is rendered inside an isolated iframe with `sandbox="allow-same-origin"`.

---

## Local AI Model Management (`AiSelection`)

The `AiSelection` component provides a reactive interface for managing on-device Ollama models:

* **Readiness Polling**: Runs a background loop every 10 seconds to check if the local Ollama daemon is reachable on `127.0.0.1:11434`.
* **Daemon Supervision**: If offline, clicking "Start Ollama" invokes `OllamaService::serve()`, spawning the background daemon process with detached I/O.
* **Streaming Model Downloads**: When pulling a model, `service.pull_model()` streams progress events. The fractional progress is captured via a callback (`p.fraction()`) and bound directly to a reactive progress percentage.
* **Orphan Protection**: When uninstalling a model, the component checks whether the deleted model matches the currently active classifier (`ACTIVE_AI_MODEL_ID`). If it does, the active setting is automatically cleared in SQLite.

---

## Desktop OS Integration

### 1. Window Lifecycle and Background Monitoring
Fumiko is configured with `WindowCloseBehaviour::WindowHides`. Clicking the window close button (`X`) does not exit the process; it hides the window from view. Background sync workers and account polling watchers continue running uninterrupted.

### 2. System Tray Integration (`tray.rs`)
The system tray is initialized using `muda` and `trayicon`:
* **Open Fumiko**: Unhides, un-minimizes, and focuses the main application window.
* **Sync All**: Dispatches a `SyncTarget::All` command to the central sync worker without opening the UI window.
* **Quit**: Performs an explicit process exit (`std::process::exit(0)`).

### 3. Dynamic User Theming
Fumiko supports runtime user styling via `load_custom_css`. At launch, the application checks `%LOCALAPPDATA%/Fumiko/custom.css` (or OS equivalent). If found, the CSS text is injected into a `<style>` block in the document head, overriding default custom properties.

---

## In-Place Binary Updates (`updater.rs`)

Fumiko includes an in-place binary update pipeline powered by `self_update`:

* **Non-Blocking Release Checks**: GitHub release queries are wrapped in `tokio::task::spawn_blocking` so network calls never stutter the UI.
* **Version Comparison**: Compares the compile-time `cargo_crate_version!()` against the latest GitHub release tag.
* **In-App Notification Banner**: If a newer release is detected, `state.available_update` is set, displaying the dismissible `UpdateBanner`.
* **In-Place Replacement**: Clicking "Update Now" downloads the release archive, extracts the binary, replaces the running executable on disk, and prompts the user to restart.

---

## Frontend Architecture Checklist

When adding new views, modifying state hooks, or altering desktop integration, ensure these invariants remain intact:

* Keep Async Workers in Root Scope: Never attach long-lived communication channels or worker loops to individual route components.
* Supervise Watcher Tasks: Always cancel and drop background polling handles when an account is removed from storage.
* Always Route HTML Through `sanitize_html`: Never inject raw, unsanitized email strings directly into the DOM or webview.
* Never Allow Unchecked `@import` in Email CSS: Ensure the style extraction pass neutralizes `@import` and `javascript:` before re-inserting CSS blocks.
* Clean Up Active Model Settings on Uninstall: Clear `ACTIVE_AI_MODEL_ID` in storage whenever the currently active model is deleted.
* Offload Blocking Tasks from Async Runtimes: Always wrap synchronous filesystem operations, heavy cryptography, or `self_update` calls inside `tokio::task::spawn_blocking`.
* Honor `WindowCloseBehaviour::WindowHides`: Ensure quitting the app from the system tray explicitly calls `std::process::exit(0)`.
* Invalidate Views via `refresh_trigger`: Use `refresh_trigger` counter invalidation rather than passing manual update signals between disparate routes.
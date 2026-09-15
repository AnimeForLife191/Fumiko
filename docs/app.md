# Desktop Application and UI Architecture

The `app` crate serves as the presentation and orchestration layer for Fumiko. Built with Dioxus and Tao, it combines a reactive desktop user interface with background service coordination, local AI supervision, and native desktop integration.

Unlike a standard web frontend, a desktop email watcher operates as a persistent daemon. It must coordinate background inbox polling, manage operating system lifecycles (system tray, window visibility, native keyrings), safely render arbitrary third-party HTML, supervise on-device AI sidecars, and stream model downloads, all while keeping the UI responsive.

This document details the architecture of the `app` crate, explains our reactive state and worker model, breaks down our HTML sanitization and link execution pipeline, and outlines our frontend invariants.

## What We Are Solving

Desktop webview applications face several engineering challenges that standard web apps do not:

* **Route Navigation Tearing**: In component-based UI frameworks, navigating between views unmounts component trees. If an OAuth handshake or background sync is tied to a specific route component (like `AddAccount`), navigating back to the dashboard will abort the future and leave local network sockets hanging.
* **Task Leaks from Background Polling**: Spawning unmanaged background loops inside component hooks causes task leaks when routes unmount or accounts are deleted, leading to ghost sync passes and wasted provider API quotas.
* **The Webview Link Trap**: Desktop webview engines (Wry, WebKit, WebView2) disable new window popups by default. Clicking ordinary email links or help links with `target="_blank"` silently fails. Conversely, allowing unvetted link navigation can cause the entire desktop app window to navigate away to a third-party website.
* **The Email Sanitization Dilemma**: Naive HTML sanitizers strip all `<style>` elements, destroying email layouts, receipts, and invoices. Leaving styles unvetted, however, exposes the desktop webview to CSS data exfiltration (`@import`) and script injection (`javascript:` expressions).
* **Accidental Process Termination**: If closing the application window terminates the binary, background inbox monitoring stops completely, defeating the purpose of an email watcher.
* **Orphaned Sidecar Processes**: Terminating the desktop application abruptly or via tray exit bypasses Rust's standard stack unwinding (`Drop`), leaving background inference sidecars (`llama-server`) orphaned in memory.
* **Orphaned Configuration References**: Deleting an installed local AI model from disk while leaving it marked as the active classifier in SQLite causes background sync jobs to fail silently.
* **UI Thread Starvation**: GitHub release checks, binary file replacement, and token cryptography will freeze the user interface if executed directly on the main event loop.

## View Architecture

Fumiko uses Dioxus `Router` with an overarching `Layout` component that maintains a fixed 250px sidebar, account filter controls, and an animated top update banner:

* **Dashboard (`Dashboard`)**: Displays true mailbox stats (unread, findings, and total counts queried from SQLite), a 5-item preview of recent messages and findings, and active watch criteria toggle switches.
* **Inbox (`Inbox`)**: A responsive split-view interface (`minmax(340px, 400px) minmax(0, 1fr)`). The left pane displays a message list with search and instant sync triggers; the right pane displays the active `ReadingPane`.
* **Findings Board (`Findings`)**: A prioritized stream of emails that matched user-defined watch criteria. Displays model confidence percentages, with context-aware triage: clicking "Clear Category" removes matches only for the active filter, while "Clear All" clears all findings. "Delete" moves the underlying email to Trash.
* **Trash (`Trash`)**: Displays soft-deleted emails with timestamps, an instant "Restore" action, permanent deletion, and manual "Empty Trash Now" functionality.
* **Add Account (`AddAccount`)**: Supports connecting accounts via OAuth (Gmail or Outlook) or direct IMAP using App Passwords:
  * **Provider Switcher**: Toggles between Google, Microsoft, and IMAP.
  * **Preset Selector Pills**: Auto-configures server hosts and ports for Gmail (`imap.gmail.com:993`), iCloud (`imap.mail.me.com:993`), Yahoo (`imap.mail.yahoo.com:993`), Fastmail (`imap.fastmail.com:993`), or custom servers.
  * **App Password Normalization**: Automatically strips space groupings copied from Google and Apple credential generators.
  * **Credentials Subcomponent**: Manages custom developer Client IDs and Client Secrets for users who prefer their own Google Cloud or Azure projects.
* **Settings (`Settings`)**: Divided into focused submodules:
  * `LinkedAccountsSection`: Displays connected accounts with inline editable names (`EditableAccountName`), error indicators, and deletion actions.
  * `WatchCriteriaSection`: CRUD management for user-defined classification rules.
  * `CustomThemeSection`: Manages runtime UI theming with CSS file import, enable/disable toggling, and native folder access.
  * `AiSelection`: Supervises local AI engines. Features a tabbed interface allowing users to toggle between the zero-setup Built-in engine and external Ollama. Handles direct streaming downloads for GGUF models, selection, and file deletion.
  * `InitialSyncSection`: Configures the initial email fetch limit (10 to 200 emails) when linking mailboxes.
  * `MailboxCapacitySection`: Configures the maximum inbox capacity (100 to 1,000 emails) and findings capacity (50 to 500 emails) stored and displayed locally.
  * `TrashRetentionSection`: Configures automatic trash retention periods (0 to 365 days, where 0 represents instant deletion).
  * `DangerZoneSection`: Two-step confirmation dialog that executes `storage.wipe_all_data()`, with an optional checkbox to purge downloaded AI model weights.

## State Management and Reactive Invalidation (`AppState`)

Fumiko decouples persistent relational storage (SQLite) from ephemeral UI state using `AppState`:

### Signal Hierarchy
`AppState` is injected into the root component context and exposes granular `Copy` signals. Using discrete signals rather than a monolithic struct ensures that ticking a sync timer or bumping `refresh_trigger` only re-renders components that read those specific signals:
* `accounts`: Cached vector of active `LinkedAccount` rows.
* `selected_account`: Global account filter pointer (`None` represents "All Accounts").
* `selected_email`: Currently active email UUID for split-view reading.
* `is_syncing` and `is_linking`: Boolean flags that drive loading spinners and disable conflicting trigger buttons.
* `linking_status`: Real-time status string displayed during OAuth loops or IMAP connection tests.
* `sync_progress`: Granular progress tracking (`SyncProgress`) showing item counts and active phases (Hydrating vs Classifying).
* `available_update`: Latest release tag string if an update is discovered.
* `refresh_trigger`: Global invalidation counter. Bumping this signal invalidates downstream `use_resource` hooks across all screens, triggering clean database re-queries.
* `sync_tick`: Counter bumped whenever background sync passes finish, refreshing timestamps and mailbox stats.

### Decoupled Worker Channels
To prevent UI code from running asynchronous tasks directly inside event handlers, state changes communicate with background workers via unbounded Tokio MPSC channels:
* `sync_tx: UnboundedSender<SyncTarget>`: Receives `SyncTarget::One(Uuid)` or `SyncTarget::All` commands.
* `auth_tx: UnboundedSender<AuthCommand>`: Receives `AuthCommand::Start(Provider)`, `AuthCommand::StartImap { email, password, host, port }`, or `AuthCommand::Cancel` commands.
* `ai_tx: UnboundedSender<AiCommand>`: Receives `AiCommand::StartBuiltin(filename)` or `AiCommand::StopBuiltin` commands.

## Background Worker Lifecycles

All long-running services live in the **root component scope** (`Fumiko`), guaranteeing that background operations survive view navigation.

### 1. Root Authentication Worker
Authentication handshakes require non-blocking coordination while the user interacts with the app:
* **OAuth Loopback**: Binds to an ephemeral loopback socket (`127.0.0.1:0`) while the user authenticates in their external browser.
* **IMAP Pre-flight Verification**: Opens a background TLS connection to test credentials against the remote IMAP host before saving account records to storage.
* **Navigation Resilience**: Because the receiver `auth_rx` is managed inside a root `use_hook`, the user can navigate away from `AddAccount` without aborting an active connection handshake.
* **Deterministic Teardown**: When `AuthCommand::Cancel` is received (or a new authorization flow begins), the previous `Task` is explicitly cancelled via `task.cancel()`. For OAuth, cancelling drops the future and runs the listener drop guard to unbind the local TCP socket immediately. For IMAP, cancelling drops the pending TCP connection.

### 2. Supervised Account Watchers
Fumiko watches multiple inboxes concurrently using a supervised task pool:
* The root component maintains a `running_watchers: Signal<HashMap<Uuid, Task>>`.
* A `use_effect` monitors changes to the `accounts` signal against the watcher map.
* When a new account is linked, an isolated task is spawned to poll the provider every `POLL_INTERVAL_SECS` (60 seconds).
* **Pruning and Cleanup**: If an account is removed from SQLite, `watchers.retain()` detects that its UUID is no longer in `accounts`, calls `task.cancel()`, and drops the handle, preventing duplicate or ghost polling loops.

### 3. Central Sync Worker
The sync worker serializes sync requests received over `sync_channel`:
* Sets `is_syncing` to `true`.
* Instantiates `SyncService` and forwards real-time progress events to `state.sync_progress`.
* Dispatches the sync pass to `email_core`.
* Automatically purges older messages and expired trash according to configured capacity and retention limits.
* Increments `refresh_trigger` and `sync_tick`, updating all visible email lists and dashboard stats.
* Clears `sync_progress` and resets `is_syncing` back to `false`.

### 4. Built-in Local AI Supervisor and Process Lifecycles
When the built-in engine is active, the root component supervises the `llama-server` background process:
* Monitors the `ai_backend` and `active_ai_model_id` settings in storage.
* If `ai_backend` is `"builtin"` and an active model file exists on disk, it checks whether `llama-server` is responding on `127.0.0.1:11435`. If inactive, it spawns the bundled binary with `-c 2048 -np 1 -ngl 99`.
* **Global Process Registry and Explicit Shutdown**: The child process handle is held in a global `ACTIVE_CHILD_PROCESS` `OnceLock`. Because `std::process::exit(0)` bypasses standard Rust `Drop` implementations, clean termination is enforced via `kill_builtin_server()`. This hook is executed explicitly when:
  * The main desktop window loop exits in `fn main()`.
  * The user selects "Quit" from the system tray menu (`tray.rs`).
  * The user triggers an immediate restart from the in-app update banner (`UpdateBanner`).
  * The user wipes all local data via the Danger Zone.

## Lazy Message Loading, HTML Sanitization, and Link Handling

### Lazy Full-Message Retrieval
To keep mailbox lists lightweight and save local bandwidth, message bodies and raw MIME attachments are not downloaded during background sync. 

In `ReadingPane`:
1. When a user clicks an email, `use_resource` queries SQLite for the message headers and account record.
2. It requests fresh access credentials via `get_access_token_for_account`.
3. The dynamic `EmailProvider` trait fetches the full payload (`fetch_full_message`) on demand using `BODY.PEEK` (for IMAP) or REST endpoints (for Gmail and Outlook).
4. Once fetched, the email is marked as viewed in storage, and `refresh_trigger` is bumped to update read status badges across the UI without dirtying remote server read flags.

### Direct Webmail Triage ("Open in Webmail")
Because Fumiko operates with read-only scopes and watcher credentials, users cannot reply to emails directly inside the app. To streamline triage, `ReadingPane` provides an instant "Open in Webmail" button:
* **Gmail OAuth**: Constructs a direct deep link: `https://mail.google.com/mail/?authuser={email}#all/{provider_message_id}`. Google opens the exact account profile even if multiple accounts are active in the browser.
* **Outlook OAuth**: Constructs a deep link targeting `outlook.live.com` for personal accounts or `outlook.office.com` for corporate accounts with the encoded message ID.
* **IMAP Accounts**: Webmail providers do not expose internal IMAP UIDs in URL hashes. Fumiko directs the user to the webmail homepage corresponding to their preset (for example, `https://mail.google.com/mail/?authuser={email}` for Gmail, `https://app.fastmail.com/mail/` for Fastmail, `icloud.com/mail` for iCloud, and `mail.yahoo.com` for Yahoo).
* Clicking the button calls `webbrowser::open()` to launch the browser without disrupting the desktop app window.

### Desktop Link Trapping in the UI
In desktop webview runtimes (such as WebView2 on Windows), standard anchor tags with `target="_blank"` are blocked by default. In-app links (such as setup links in `AddAccount` for generating app passwords) attach an explicit `onclick` handler that invokes `webbrowser::open()` directly, preventing clicks from being swallowed by the webview.

### The Three-Stage Sanitization and Link Pipeline
Rendering untrusted HTML emails in a desktop webview presents severe security risks. Fumiko implements a three-stage pipeline:

1. **Regex Style Extraction and Cleanse**: Before Ammonia strips `<style>` elements, a regular expression extracts all style blocks. `javascript:` protocol strings are purged, and `@import` directives are commented out to prevent CSS-based data exfiltration.
2. **Ammonia Structural Sanitization**: The body markup is filtered through `ammonia::Builder`. Permitted URL schemes are restricted to `data`, `cid`, `http`, `https`, `mailto`, and `tel`. All `<script>`, `<object>`, `<embed>`, and `<iframe>` elements are stripped, and inline event handlers (`onload`, `onclick`) are purged.
3. **Sandboxed Reassembly and External Link Routing**:
   * The sanitized markup is injected into an isolated document wrapper with `<base target="_top">`.
   * The markup renders inside an `<iframe>` configured with:
     ```html
     <iframe sandbox="allow-same-origin allow-top-navigation-by-user-activation"></iframe>
     ```
   * Because script execution is omitted from sandbox permissions, untrusted JavaScript cannot execute inside the frame.
   * When a user clicks a link inside the email, `allow-top-navigation-by-user-activation` permits the navigation because it was initiated by a user click targeting `_top`.
   * Dioxus desktop's `.with_navigation_handler()` intercepts the request before the webview navigates. If the URL starts with `http://`, `https://`, or `mailto:`, it launches the default operating system browser via `webbrowser::open()` and returns `false`. This allows external links to open smoothly without letting third-party pages replace the Fumiko application interface.

## Local AI Model Management (`AiSelection`)

The `AiSelection` component provides a reactive interface for managing on-device inference engines:

### 1. Dual Backend Switching
Users toggle between **Built-in (Zero Setup)** and **Ollama (Advanced)**:
* Switching updates the `ai_backend` key in SQLite settings and stops running built-in processes via `AiCommand::StopBuiltin`.
* Background daemon health checks use Dioxus's `use_drop` hook to cleanly cancel polling tasks when the Settings view unmounts, preventing task leaks across route navigation.

### 2. Built-in Engine View
* **Readiness Status**: Queries `http://127.0.0.1:11435/health` every 1.5 seconds to confirm the sidecar process is active.
* **Catalog Listing**: Renders verified models from `BUILTIN_CATALOG` with human-readable file sizes and descriptions.
* **Streaming Downloads**: Uses `ModelDownloader` to stream GGUF files directly from Hugging Face into the platform models directory. Completion percentages update reactively on the UI thread without crossing thread bounds.
* **One-Click Deletion with Process Teardown**: Stops the active process, allows a 400ms buffer for OS file locks to clear, deletes the local `.gguf` file via `ModelDownloader::delete_model`, and logs any filesystem errors.
* **Orphan Protection**: If the user deletes a model that is currently marked as active, `ACTIVE_AI_MODEL_ID` is cleared in SQLite automatically.

### 3. Ollama Backend View
* **Readiness Polling**: Checks `http://127.0.0.1:11434/api/tags` every 10 seconds while the Ollama tab is active.
* **Daemon Supervision**: If offline, clicking "Start Ollama" invokes `OllamaService::serve()` to launch the background process with detached I/O, setting `OLLAMA_NUM_PARALLEL=1` and `OLLAMA_MAX_LOADED_MODELS=1`.
* **Tag Pulls**: Streams progress events for models pulled via the Ollama registry.

## Desktop OS Integration

### 1. Window Lifecycle and Background Monitoring
Fumiko is configured with `WindowCloseBehaviour::WindowHides`. Clicking the window close button (`X`) does not exit the process; it hides the window from view. Background sync workers and account polling watchers continue running uninterrupted.

### 2. System Tray Integration (`tray.rs`)
The system tray is initialized using `muda` and `trayicon`:
* **Open Fumiko**: Unhides, un-minimizes, and focuses the main application window.
* **Sync All**: Dispatches a `SyncTarget::All` command to the central sync worker without opening the UI window.
* **Quit**: Executes explicit child termination via `kill_builtin_server()` and calls `std::process::exit(0)`.

### 3. Dynamic User Theming
Fumiko supports runtime user styling via `load_custom_css`:
* At launch, the application checks `%LOCALAPPDATA%\fumiko\custom.css` (or OS equivalent). If found, the CSS text is injected into a `<style>` block in the document head.
* The `CustomThemeSection` in Settings allows users to import `.css` files directly from their file picker, toggle between custom and default themes dynamically (by renaming between `custom.css` and `custom.css.disabled`), or open the local theme directory in their operating system file manager (`explorer`, `open`, or `xdg-open`).

## In-Place Binary Updates (`updater.rs`)

Fumiko includes an in-place binary update pipeline powered by `self_update`:

* **Non-Blocking Release Checks**: GitHub release queries are wrapped in `tokio::task::spawn_blocking` so network calls never stutter the UI.
* **Version Comparison**: Compares the compile-time `cargo_crate_version!()` against the latest GitHub release tag.
* **In-App Notification Banner**: If a newer release is detected, `state.available_update` is set, displaying the dismissible `UpdateBanner`.
* **In-Place Replacement**: Clicking "Update Now" downloads the release archive, extracts the binary, replaces the running executable on disk, and prompts the user to restart. Clicking "Restart Now" terminates sidecars via `kill_builtin_server()` before exiting.

## Frontend Architecture Checklist

When adding new views, modifying state hooks, or altering desktop integration, ensure these rules remain intact:

* Keep Async Workers in Root Scope: Never attach long-lived communication channels, worker loops, or inference supervisors to individual route components.
* Clean Up View-Scoped Polling with `use_drop`: Always pair component-level polling tasks with `use_drop` to cancel them when routes unmount.
* Supervise Watcher Tasks: Always cancel and drop background polling handles when an account is removed from storage.
* Terminate Sidecars Explicitly on Exit: Call `kill_builtin_server()` before invoking `std::process::exit(0)` in tray menus, update banners, or main loop teardown.
* Avoid target="_blank" in Desktop Webviews: Use explicit `webbrowser::open` handlers so external links reliably launch the default system browser.
* Normalize App Passwords: Strip whitespace and spaces from user-entered IMAP app passwords before passing them to backend workers.
* Route Links to the System Browser: Pair `<base target="_top">` and `allow-top-navigation-by-user-activation` with `with_navigation_handler` so external email links launch the default browser without navigating the desktop window.
* Always Route HTML Through `sanitize_html`: Never inject raw, unsanitized email strings directly into the DOM or webview.
* Never Allow Unchecked `@import` in Email CSS: Ensure the style extraction pass neutralizes `@import` and `javascript:` before re-inserting CSS blocks.
* Clean Up Active Model Settings on Uninstall: Clear `ACTIVE_AI_MODEL_ID` in storage whenever the currently active model is deleted.
* Offload Blocking Tasks from Async Runtimes: Always wrap synchronous filesystem operations, heavy cryptography, or `self_update` calls inside `tokio::task::spawn_blocking`.
* Honor `WindowCloseBehaviour::WindowHides`: Ensure quitting the app from the system tray explicitly calls `std::process::exit(0)` after process teardown.
* Invalidate Views via `refresh_trigger`: Use `refresh_trigger` counter invalidation rather than passing manual update signals between disparate routes.
//! In-place binary self-updating using the GitHub Releases API.

use self_update::{VersionStatus, cargo_crate_version};
use tracing::{error, info, warn};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const REPO_OWNER: &str = "AnimeForLife191";
const REPO_NAME: &str = "Fumiko";
const BIN_NAME: &str = "fumiko";

/// Synchronously queries GitHub releases for updates.
fn check_sync() -> Result<Option<String>, BoxError> {
    info!(
        "Checking GitHub releases for {}/{}...",
        REPO_OWNER, REPO_NAME
    );

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .map_err(|e| Box::new(e) as BoxError)?
        .fetch()
        .map_err(|e| Box::new(e) as BoxError)?;

    if let Some(latest) = releases.latest() {
        let current = cargo_crate_version!();
        info!(
            "Current app version: v{}, Latest release on GitHub: v{}",
            current,
            latest.version()
        );

        if self_update::version::bump_is_greater(current, latest.version())
            .map_err(|e| Box::new(e) as BoxError)?
        {
            info!("Update available: v{} -> v{}", current, latest.version());
            return Ok(Some(latest.version().to_string()));
        } else {
            info!("Fumiko is already up to date.");
        }
    } else {
        warn!(
            "No releases found on GitHub repository {}/{}",
            REPO_OWNER, REPO_NAME
        );
    }

    Ok(None)
}

/// Synchronously downloads, extracts, and replaces the running executable on disk.
fn update_sync() -> Result<VersionStatus, BoxError> {
    info!("Starting in-place update process...");

    let status = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(cargo_crate_version!())
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .build()
        .map_err(|e| Box::new(e) as BoxError)?
        .update()
        .map_err(|e| {
            error!("Failed to download and replace binary: {e}");
            Box::new(e) as BoxError
        })?;

    info!("Update completed with status: {:?}", status);
    Ok(status)
}

/// Non-blocking release check offloaded to Tokio's blocking thread pool.
///
/// Running network and JSON operations on Tokio's blocking pool prevents UI thread stutter.
pub async fn check_for_update() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(check_sync)
        .await
        .map_err(|e| format!("Task join error: {}", e))?
        .map_err(|e| e.to_string())
}

/// Non-blocking binary download and file replacement offloaded to Tokio's blocking thread pool.
pub async fn update_app() -> Result<VersionStatus, String> {
    tokio::task::spawn_blocking(update_sync)
        .await
        .map_err(|e| format!("Task join error: {}", e))?
        .map_err(|e| e.to_string())
}
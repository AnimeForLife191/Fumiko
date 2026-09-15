//! Standard directory paths and binary discovery for the built-in engine.

use std::path::PathBuf;

use common::APP_SERVICE_NAME;

/// Returns the platform-specific local application data directory:
/// - **Windows**: `%LOCALAPPDATA%\fumiko`
/// - **macOS**: `~/Library/Application Support/fumiko`
/// - **Linux**: `$XDG_DATA_HOME/fumiko` or `~/.local/share/fumiko`
pub fn default_app_data_dir() -> PathBuf {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data).join(APP_SERVICE_NAME)
    } else if let Ok(home) = std::env::var("HOME") {
        #[cfg(target_os = "macos")]
        {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(APP_SERVICE_NAME)
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
                PathBuf::from(xdg).join(APP_SERVICE_NAME)
            } else {
                PathBuf::from(home)
                    .join(".local")
                    .join("share")
                    .join(APP_SERVICE_NAME)
            }
        }
    } else {
        PathBuf::from("./data")
    }
}

/// Returns the directory where downloaded GGUF model files are stored.
pub fn default_models_dir() -> PathBuf {
    default_app_data_dir().join("models")
}

/// Locates the bundled `llama-server` executable in this priority order:
/// 1. Next to the executable in a subfolder: `<exe_dir>/bin/llama-server`
/// 2. Directly beside the running executable: `<exe_dir>/llama-server`
/// 3. In a macOS App Bundle: `Contents/Resources/bin/llama-server`
/// 4. In the repository / working directory: `<cwd>/bin/llama-server`
/// 5. In the persistent OS AppData folder: `<app_data>/bin/llama-server`
/// 6. Anywhere on system `PATH`
pub fn find_llama_server_binary() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let bin_name = "llama-server.exe";
    #[cfg(not(target_os = "windows"))]
    let bin_name = "llama-server";

    // 1 & 2: Check relative to running executable (standard release / installer)
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            let candidate_sub = exe_dir.join("bin").join(bin_name);
            if candidate_sub.is_file() {
                return Some(candidate_sub);
            }

            let candidate_flat = exe_dir.join(bin_name);
            if candidate_flat.is_file() {
                return Some(candidate_flat);
            }

            #[cfg(target_os = "macos")]
            {
                // In macOS App Bundle: Fumiko.app/Contents/MacOS/../Resources/bin/llama-server
                if let Some(contents_dir) = exe_dir.parent() {
                    let candidate_bundle = contents_dir.join("Resources").join("bin").join(bin_name);
                    if candidate_bundle.is_file() {
                        return Some(candidate_bundle);
                    }
                }
            }
        }
    }

    // 3. Check relative to current working directory (development with `cargo run` or `dx serve`)
    if let Ok(cwd) = std::env::current_dir() {
        let dev_bin = cwd.join("bin").join(bin_name);
        if dev_bin.is_file() {
            return Some(dev_bin);
        }
    }

    // 4. Check OS AppData folder (for standalone / portable users)
    let app_data_bin = default_app_data_dir().join("bin").join(bin_name);
    if app_data_bin.is_file() {
        return Some(app_data_bin);
    }

    // 5. Fallback: check system PATH
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate_path = dir.join(bin_name);
            if candidate_path.is_file() {
                return Some(candidate_path);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_models_dir_structure() {
        let dir = default_models_dir();
        assert!(dir.ends_with("models"));
    }
}
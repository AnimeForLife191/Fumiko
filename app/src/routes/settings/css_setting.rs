//! Runtime UI theming, CSS file imports, and native theme folder access.

use crate::utils::{
    disable_custom_css, enable_custom_css, get_custom_css_dir, has_disabled_custom_css,
    open_theme_folder, save_custom_css,
};
use dioxus::prelude::*;

#[component]
pub fn CustomThemeSection() -> Element {
    let mut active_custom_css = use_context::<Signal<Option<String>>>();
    let mut status_message = use_signal(String::new);
    let mut disabled_exists = use_signal(has_disabled_custom_css);

    let folder_path_display = get_custom_css_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    let handle_file_upload = move |evt: FormEvent| async move {
        let files = evt.files();
        if let Some(first_file) = files.first() {
            match first_file.read_string().await {
                Ok(css_text) => {
                    if let Err(e) = save_custom_css(&css_text) {
                        status_message.set(format!("Failed to save theme: {e}"));
                    } else {
                        active_custom_css.set(Some(css_text));
                        disabled_exists.set(false);
                        status_message.set("Custom theme imported and applied!".to_string());
                    }
                }
                Err(e) => {
                    status_message.set(format!("Failed to read file: {e}"));
                }
            }
        }
    };

    let is_custom_active = active_custom_css().is_some();

    rsx! {
        div { class: "settings__section",
            div { class: "settings__section-header",
                span { class: "icon icon--settings", aria_hidden: "true" }
                h2 { "Custom Theming" }
            }

            div { class: "settings__section-content",
                p { class: "settings__description",
                    "Import a custom .css file to customize Fumiko's interface. Imported styles apply immediately."
                }

                p { class: "settings__folder-hint",
                    "Theme folder: "
                    code { class: "settings__code-path", "{folder_path_display}" }
                }

                div { class: "settings__theme-actions",
                    label { class: "settings__button settings__button--file",
                        span { class: "icon icon--edit", aria_hidden: "true" }
                        span { class: "settings__button-text", "Import .css File" }
                        input {
                            r#type: "file",
                            accept: ".css",
                            class: "settings__hidden-file-input",
                            onchange: handle_file_upload,
                        }
                    }

                    button {
                        class: "settings__button settings__button--secondary",
                        onclick: move |_| open_theme_folder(),
                        "Open Theme Folder"
                    }

                    if is_custom_active {
                        button {
                            class: "settings__button settings__button--secondary",
                            onclick: move |_| {
                                if let Err(e) = disable_custom_css() {
                                    status_message.set(format!("Failed to disable theme: {e}"));
                                } else {
                                    active_custom_css.set(None);
                                    disabled_exists.set(true);
                                    status_message
                                        .set(
                                            "Default theme restored. Your custom CSS was preserved.".to_string(),
                                        );
                                }
                            },
                            "Use Default Theme"
                        }
                    }

                    if !is_custom_active && *disabled_exists.read() {
                        button {
                            class: "settings__button settings__button--secondary",
                            onclick: move |_| {
                                match enable_custom_css() {
                                    Ok(Some(content)) => {
                                        active_custom_css.set(Some(content));
                                        disabled_exists.set(false);
                                        status_message.set("Custom theme re-enabled!".to_string());
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        status_message.set(format!("Failed to enable theme: {e}"));
                                    }
                                }
                            },
                            "Enable Custom Theme"
                        }
                    }
                }

                div { class: "settings__theme-status-wrap",
                    if is_custom_active {
                        span { class: "settings__status-text settings__status-text--success",
                            "Active: Custom theme loaded"
                        }
                    } else if *disabled_exists.read() {
                        span { class: "settings__status-text",
                            "Active: Default theme (custom theme is saved and paused)"
                        }
                    } else {
                        span { class: "settings__status-text", "Active: Default theme" }
                    }
                }

                if !status_message().is_empty() {
                    p { class: "settings__status-text settings__status-text--toast",
                        "{status_message}"
                    }
                }
            }
        }
    }
}
//! Helper utilities for custom theming, timestamp formatting, and email HTML sanitization.

use chrono::{DateTime, Local};
use common::APP_SERVICE_NAME;
use regex::Regex;
use std::sync::LazyLock;

/// Returns the theme directory, ensuring it exists on disk.
pub fn get_custom_css_dir() -> Option<std::path::PathBuf> {
    let data_dir = dirs::data_local_dir()?.join(APP_SERVICE_NAME);
    let _ = std::fs::create_dir_all(&data_dir);
    Some(data_dir)
}

/// Returns the path to the active custom.css file.
pub fn get_custom_css_path() -> Option<std::path::PathBuf> {
    Some(get_custom_css_dir()?.join("custom.css"))
}

/// Returns the path to the disabled custom.css.disabled file.
pub fn get_disabled_custom_css_path() -> Option<std::path::PathBuf> {
    Some(get_custom_css_dir()?.join("custom.css.disabled"))
}

/// Reads the active custom.css stylesheet at application launch.
pub fn load_custom_css() -> Option<String> {
    std::fs::read_to_string(get_custom_css_path()?).ok()
}

/// Writes user-imported stylesheet content to custom.css.
pub fn save_custom_css(content: &str) -> Result<(), std::io::Error> {
    if let Some(path) = get_custom_css_path() {
        std::fs::write(path, content)?;
    }
    Ok(())
}

/// Re-enables the custom theme by renaming custom.css.disabled to custom.css.
pub fn enable_custom_css() -> Result<Option<String>, std::io::Error> {
    if let (Some(active), Some(disabled)) = (get_custom_css_path(), get_disabled_custom_css_path())
    {
        if disabled.exists() {
            std::fs::rename(disabled, &active)?;
            let content = std::fs::read_to_string(active)?;
            return Ok(Some(content));
        }
    }
    Ok(None)
}

/// Disables the custom theme by renaming custom.css to custom.css.disabled without deleting customizations.
pub fn disable_custom_css() -> Result<(), std::io::Error> {
    if let (Some(active), Some(disabled)) = (get_custom_css_path(), get_disabled_custom_css_path())
    {
        if active.exists() {
            std::fs::rename(active, disabled)?;
        }
    }
    Ok(())
}

/// Checks whether a previously saved but disabled theme file exists on disk.
pub fn has_disabled_custom_css() -> bool {
    get_disabled_custom_css_path()
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// Opens the local theme folder in the host operating system's native file explorer.
pub fn open_theme_folder() {
    if let Some(dir) = get_custom_css_dir() {
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(&dir).spawn();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
        }
    }
}

/// Formats a Unix timestamp into a localized human-readable string (e.g. "Oct 24, 02:30 PM").
pub fn format_email_timestamp(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .map(|utc| {
            let local: DateTime<Local> = utc.with_timezone(&Local);
            local.format("%b %d, %I:%M %p").to_string()
        })
        .unwrap_or_else(|| "Just now".to_string())
}

static STYLE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<style\b[^>]*>(.*?)</style>").expect("Invalid regex"));

/// Sanitizes untrusted third-party email HTML markup through a three-stage pipeline.
///
/// Pipeline stages:
/// 1. Style Block Extraction: Extracts all `<style>` elements before Ammonia strips them.
///    Purges `javascript:` expressions and comments out `@import` directives to block CSS-based data exfiltration.
/// 2. Structural Cleansing with Ammonia: Strips dangerous tags (`<script>`, `<object>`, `<iframe>`),
///    removes inline event handlers (`onclick`, `onload`), and restricts URI schemes to safe protocols.
/// 3. Document Sandbox Assembly: Wraps the sanitized content with `<base target="_top">` and base typography resets,
///    ensuring that user clicks inside an iframe navigate the top browsing context where Dioxus navigation handlers trap them.
pub fn sanitize_html(raw: &str) -> String {
    // 1. Extract and cleanse embedded style blocks
    let mut extracted_styles = String::new();
    for cap in STYLE_REGEX.captures_iter(raw) {
        if let Some(css_match) = cap.get(1) {
            let clean_css = css_match
                .as_str()
                .replace("javascript:", "")
                .replace("@import", "/* @import blocked */");
            extracted_styles.push_str(&clean_css);
            extracted_styles.push('\n');
        }
    }

    // 2. Configure Ammonia for HTML structure and safe attribute cleansing
    let mut builder = ammonia::Builder::default();

    let mut url_schemes = builder.clone_url_schemes();
    url_schemes.insert("data");
    url_schemes.insert("cid");
    url_schemes.insert("http");
    url_schemes.insert("https");
    url_schemes.insert("mailto");
    url_schemes.insert("tel");
    builder.url_schemes(url_schemes);

    builder.link_rel(Some("noopener noreferrer"));

    builder
        .add_tags(&[
            "table",
            "thead",
            "tbody",
            "tfoot",
            "tr",
            "th",
            "td",
            "span",
            "div",
            "p",
            "br",
            "img",
            "font",
            "center",
            "a",
            "b",
            "strong",
            "i",
            "em",
            "u",
            "s",
            "strike",
            "ul",
            "ol",
            "li",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "blockquote",
            "hr",
            "picture",
            "source",
            "section",
            "article",
            "header",
            "footer",
            "pre",
            "code",
        ])
        .add_generic_attributes(&[
            "style", "class", "id", "align", "valign", "bgcolor", "width", "height",
        ])
        .add_tag_attributes("a", &["href", "title"])
        .add_tag_attributes(
            "img",
            &[
                "src", "alt", "width", "height", "style", "srcset", "sizes", "border",
            ],
        )
        .add_tag_attributes(
            "table",
            &[
                "cellpadding",
                "cellspacing",
                "border",
                "width",
                "bgcolor",
                "align",
            ],
        )
        .add_tag_attributes(
            "td",
            &[
                "colspan", "rowspan", "nowrap", "width", "height", "bgcolor", "align", "valign",
            ],
        )
        .add_tag_attributes(
            "th",
            &[
                "colspan", "rowspan", "nowrap", "width", "height", "bgcolor", "align", "valign",
            ],
        )
        .add_tag_attributes("font", &["color", "size", "face"]);

    let cleaned_body = builder.clean(raw).to_string();

    // 3. Assemble self-contained sandboxed document with base target and resets
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <base target="_top">
    <!-- Preserved Email Styles -->
    <style>
        {extracted_styles}
    </style>
    <!-- Base Client Resets -->
    <style>
        html, body {{
            margin: 0;
            padding: 18px 22px;
            background-color: #ffffff;
            color: #1a1a1a;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
            font-size: 14px;
            line-height: 1.55;
            -webkit-font-smoothing: antialiased;
            overflow-wrap: break-word;
            word-wrap: break-word;
        }}
        img {{
            max-width: 100%;
            height: auto;
        }}
        table {{
            border-collapse: collapse;
        }}
        a {{
            color: #2563eb;
        }}
    </style>
</head>
<body>
    {cleaned_body}
</body>
</html>"#
    )
}
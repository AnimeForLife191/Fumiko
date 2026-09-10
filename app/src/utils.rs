use chrono::{DateTime, Local};
use common::APP_SERVICE_NAME;

pub fn load_custom_css() -> Option<String> {
    let data_dir = dirs::data_local_dir()?.join(APP_SERVICE_NAME);
    let custom_css_path = data_dir.join("custom.css");

    std::fs::read_to_string(custom_css_path).ok()
}

pub fn format_email_timestamp(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .map(|utc| {
            let local: DateTime<Local> = utc.with_timezone(&Local);
            local.format("%b %d, %I:%M %p").to_string()
        })
        .unwrap_or_else(|| "Just now".to_string())
}

use std::sync::LazyLock;
use regex::Regex;

static STYLE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<style\b[^>]*>(.*?)</style>").expect("Invalid regex")
});

pub fn sanitize_html(raw: &str) -> String {
    // 1. Extract all <style> blocks so Ammonia does not discard header styling
    let mut extracted_styles = String::new();
    for cap in STYLE_REGEX.captures_iter(raw) {
        if let Some(css_match) = cap.get(1) {
            let clean_css = css_match.as_str()
                .replace("javascript:", "")
                .replace("@import", "/* @import blocked */");
            extracted_styles.push_str(&clean_css);
            extracted_styles.push('\n');
        }
    }

    // 2. Configure Ammonia for email body sanitization
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
            "table", "thead", "tbody", "tfoot", "tr", "th", "td",
            "span", "div", "p", "br", "img", "font", "center", "a", "b", "strong",
            "i", "em", "u", "s", "strike", "ul", "ol", "li", "h1", "h2", "h3",
            "h4", "h5", "h6", "blockquote", "hr", "picture", "source", "section",
            "article", "header", "footer", "pre", "code"
        ])
        .add_generic_attributes(&["style", "class", "id", "align", "valign", "bgcolor", "width", "height"])
        .add_tag_attributes("a", &["href", "target", "title"])
        .add_tag_attributes("img", &["src", "alt", "width", "height", "style", "srcset", "sizes", "border"])
        .add_tag_attributes("table", &["cellpadding", "cellspacing", "border", "width", "bgcolor", "align"])
        .add_tag_attributes("td", &["colspan", "rowspan", "nowrap", "width", "height", "bgcolor", "align", "valign"])
        .add_tag_attributes("th", &["colspan", "rowspan", "nowrap", "width", "height", "bgcolor", "align", "valign"])
        .add_tag_attributes("font", &["color", "size", "face"]);

    let cleaned_body = builder.clean(raw).to_string();

    // 3. Assemble full document with preserved styles and base resets
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <base target="_blank">
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
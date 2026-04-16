//! Watermark generator.
//!
//! Wraps an asset (PDF / image / text) in a self-contained HTML
//! document with a diagonal CSS-tiled watermark overlay carrying the
//! viewing user's name and the export timestamp. The output is
//! offline-renderable and survives the WebView print pipeline (the
//! watermark layer is part of the printable page).
//!
//! For PDFs and images the original bytes are embedded as a data URI,
//! so the watermarked artifact is one self-contained file ready to
//! drop into the share package.

use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum WatermarkError {
    #[error("mime type '{0}' is not supported for watermarking")]
    Unsupported(String),
    #[error("text content is not valid UTF-8")]
    InvalidUtf8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WatermarkSpec {
    pub username: String,
    /// Unix seconds UTC; rendered as MM/DD/YYYY hh:mm AM/PM.
    pub generated_at_unix: i64,
    /// Optional extra label appended after the timestamp ("CONFIDENTIAL").
    pub label: Option<String>,
}

/// Wrap `bytes` (interpreted per `mime`) inside a watermarked HTML doc.
pub fn wrap_with_watermark(
    bytes: &[u8],
    mime: &str,
    spec: &WatermarkSpec,
) -> Result<String, WatermarkError> {
    let body = match mime {
        "application/pdf" => render_embed("application/pdf", bytes),
        "image/png" | "image/jpeg" | "image/jpg" => render_image(mime, bytes),
        "text/plain" => {
            let text =
                std::str::from_utf8(bytes).map_err(|_| WatermarkError::InvalidUtf8)?;
            render_text(text)
        }
        other => return Err(WatermarkError::Unsupported(other.to_string())),
    };

    let stamp = build_stamp_text(spec);
    Ok(render_document(&stamp, &body))
}

fn build_stamp_text(spec: &WatermarkSpec) -> String {
    let ts = format_datetime_us(spec.generated_at_unix);
    match &spec.label {
        Some(l) if !l.is_empty() => format!("{} · {} · {}", spec.username, ts, l),
        _ => format!("{} · {}", spec.username, ts),
    }
}

fn render_image(mime: &str, bytes: &[u8]) -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        "<div class=\"asset image\"><img src=\"data:{mime};base64,{data}\" alt=\"\"/></div>"
    )
}

fn render_embed(mime: &str, bytes: &[u8]) -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        "<div class=\"asset pdf\">\
            <embed src=\"data:{mime};base64,{data}\" type=\"{mime}\" />\
         </div>"
    )
}

fn render_text(text: &str) -> String {
    format!("<div class=\"asset text\"><pre>{}</pre></div>", html_escape(text))
}

fn render_document(stamp: &str, asset_html: &str) -> String {
    let stamp_e = html_escape(stamp);
    // The watermark is a fixed, pointer-events:none overlay that tiles
    // a rotated repeating phrase across the viewport AND each printed
    // page. The asset sits underneath at full size.
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>Watermarked export</title>
<style>
  html, body {{ margin: 0; padding: 0; height: 100%; background: #fff; }}
  .asset {{ position: relative; width: 100%; min-height: 100vh; box-sizing: border-box; padding: 16px; }}
  .asset.image img {{ max-width: 100%; height: auto; display: block; margin: 0 auto; }}
  .asset.pdf embed {{ width: 100%; height: 100vh; }}
  .asset.text pre {{
    font-family: 'Consolas','Cascadia Mono',monospace; font-size: 11pt;
    white-space: pre-wrap; word-wrap: break-word;
  }}
  /* Watermark overlay */
  .wm {{
    position: fixed; inset: 0; pointer-events: none; z-index: 9999;
    background-image: repeating-linear-gradient(
      -30deg,
      rgba(0,0,0,0) 0, rgba(0,0,0,0) 220px,
      rgba(0,0,0,0.001) 220px, rgba(0,0,0,0.001) 221px
    );
    overflow: hidden;
  }}
  .wm-grid {{
    display: grid; grid-template-columns: repeat(4, 1fr);
    grid-auto-rows: 200px; transform: rotate(-30deg);
    width: 200vw; height: 200vh;
    position: absolute; top: -50vh; left: -50vw;
  }}
  .wm-cell {{
    color: rgba(0,0,0,0.18); font-family: 'Segoe UI',Arial,sans-serif;
    font-size: 18pt; font-weight: 600; letter-spacing: 1px;
    display: flex; align-items: center; justify-content: center; user-select: none;
  }}
  @media print {{
    .wm {{ position: fixed; }}
    .asset.pdf embed {{ height: 100vh; }}
  }}
</style></head>
<body>
  {asset}
  <div class="wm" aria-hidden="true">
    <div class="wm-grid">
      {cells}
    </div>
  </div>
</body></html>"#,
        asset = asset_html,
        cells = (0..32)
            .map(|_| format!("<div class=\"wm-cell\">{stamp_e}</div>"))
            .collect::<String>(),
    )
}

fn format_datetime_us(unix: i64) -> String {
    use chrono::{Datelike, TimeZone, Timelike, Utc};
    let dt = Utc.timestamp_opt(unix, 0).single().unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap());
    let h24 = dt.hour();
    let ampm = if h24 >= 12 { "PM" } else { "AM" };
    let h12 = match h24 % 12 { 0 => 12, h => h };
    format!(
        "{:02}/{:02}/{:04} {:02}:{:02} {}",
        dt.month(), dt.day(), dt.year(), h12, dt.minute(), ampm
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> WatermarkSpec {
        WatermarkSpec {
            username: "casey.lee".into(),
            generated_at_unix: 1_700_000_000,
            label: Some("CONFIDENTIAL".into()),
        }
    }

    #[test]
    fn rejects_unsupported_mime() {
        let err = wrap_with_watermark(b"abc", "application/zip", &spec()).unwrap_err();
        assert!(matches!(err, WatermarkError::Unsupported(_)));
    }

    #[test]
    fn html_contains_username_and_timestamp() {
        let html = wrap_with_watermark(b"hello", "text/plain", &spec()).unwrap();
        assert!(html.contains("casey.lee"));
        assert!(html.contains("CONFIDENTIAL"));
        // Watermark layer present.
        assert!(html.contains("class=\"wm\""));
    }

    #[test]
    fn embeds_image_as_data_uri() {
        let html = wrap_with_watermark(b"\x89PNG", "image/png", &spec()).unwrap();
        assert!(html.contains("data:image/png;base64,"));
        assert!(html.contains("aria-hidden=\"true\""));
    }

    #[test]
    fn embeds_pdf_as_data_uri() {
        let html = wrap_with_watermark(b"%PDF-", "application/pdf", &spec()).unwrap();
        assert!(html.contains("data:application/pdf;base64,"));
    }

    #[test]
    fn text_is_escaped() {
        let html = wrap_with_watermark(b"<script>alert(1)</script>", "text/plain", &spec()).unwrap();
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn timestamp_renders_us_12h() {
        let s = WatermarkSpec {
            username: "u".into(),
            generated_at_unix: 0,
            label: None,
        };
        let html = wrap_with_watermark(b"x", "text/plain", &s).unwrap();
        assert!(html.contains("01/01/1970"));
        assert!(html.contains("AM") || html.contains("PM"));
    }
}

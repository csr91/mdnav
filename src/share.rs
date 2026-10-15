use std::{
    env,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use reqwest::blocking::Client;
use serde_json::json;

/// Mirrors the MAX_CONTENT_LENGTH check on mdnav-web so oversized payloads
/// fail fast locally instead of round-tripping to get a 413.
const MAX_CONTENT_LENGTH: usize = 1_000_000;

pub fn infer_kind_from_extension(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("log") => "log",
        Some("csv") => "csv",
        Some("json") => "json",
        Some("diff") | Some("patch") => "diff",
        _ => "text",
    }
}

pub fn publish_share(
    kind: &str,
    content: &str,
    title: Option<&str>,
    filename: Option<&str>,
    ttl_seconds: Option<u64>,
) -> Result<String> {
    if content.trim().is_empty() {
        return Err(anyhow::anyhow!("No hay contenido para compartir"));
    }
    if content.len() > MAX_CONTENT_LENGTH {
        return Err(anyhow::anyhow!(
            "El contenido supera el limite de {} bytes",
            MAX_CONTENT_LENGTH
        ));
    }

    let base_url = env::var("MDNAV_WEB_BASE_URL")
        .unwrap_or_else(|_| String::from("https://mdnav-web.vercel.app"));
    let trimmed_base = base_url.trim_end_matches('/');
    let hash = generate_share_hash();
    let api_url = format!("{trimmed_base}/api/diagrams/{hash}");

    let mut payload = json!({
        "kind": kind,
        "content": content,
        "ttlSeconds": ttl_seconds.unwrap_or(3600),
    });
    if let Some(title) = title {
        payload["title"] = json!(title);
    }
    if let Some(filename) = filename {
        payload["filename"] = json!(filename);
    }

    let client = Client::new();
    let mut request = client.post(&api_url).json(&payload);
    if let Ok(token) = env::var("MDNAV_WEB_WRITE_TOKEN") {
        if !token.trim().is_empty() {
            request = request.header("x-mdnav-token", token);
        }
    }

    let response = request.send()?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .unwrap_or_else(|_| String::from("sin detalle"));
        return Err(anyhow::anyhow!("Error web share {status}: {body}"));
    }

    let body: serde_json::Value = response.json()?;
    let url = body
        .get("url")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{trimmed_base}/{hash}"));

    Ok(url)
}

fn generate_share_hash() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("mdnav-{timestamp:x}")
}

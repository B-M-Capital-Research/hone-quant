//! The web UI, embedded in the binary at build time (`web/dist`), or served from
//! `HONE_QUANT_WEB_DIR` during development. Unknown paths fall back to `index.html` so the
//! client-side router can handle deep links.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::state::SharedState;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

fn respond(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(mime.as_ref()) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn load(state: &SharedState, path: &str) -> Option<Vec<u8>> {
    if let Some(dir) = &state.config.web_dir {
        let candidate = dir.join(path);
        // Never serve outside the web directory.
        let canonical = candidate.canonicalize().ok()?;
        if !canonical.starts_with(dir.canonicalize().ok()?) {
            return None;
        }
        return std::fs::read(canonical).ok();
    }
    Assets::get(path).map(|f| f.data.into_owned())
}

/// Refuses to start when the embedded UI was built for another base path than
/// `HONE_QUANT_BASE_PATH`: its asset URLs would point outside the app and the page would stay
/// blank.
pub fn check_build(config: &crate::config::Config) -> anyhow::Result<()> {
    if config.web_dir.is_some() {
        return Ok(());
    }
    let Some(index) = Assets::get("index.html") else {
        return Ok(());
    };
    let html = String::from_utf8_lossy(&index.data);
    if html.contains(&format!("=\"{}/assets/", config.base_path)) {
        return Ok(());
    }
    anyhow::bail!(
        "the embedded web UI was built for base path {:?} but HONE_QUANT_BASE_PATH is {:?}; rebuild the UI with HONE_QUANT_BASE_PATH={} (cd web && bun run build) and then the server",
        built_base(&html).unwrap_or_default(),
        config.base_path,
        config.base_path
    )
}

/// Base path an `index.html` was built for: the prefix of its first `/assets/` URL.
pub fn built_base(html: &str) -> Option<String> {
    let at = html.find("/assets/")?;
    let start = html[..at].rfind('"')? + 1;
    Some(html[start..at].to_string())
}

pub async fn serve(State(state): State<SharedState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(bytes) = load(&state, path) {
        return respond(path, bytes);
    }
    // Missing files with an extension are real 404s; everything else is a client route.
    let last = path.rsplit('/').next().unwrap_or(path);
    if last.contains('.') {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    match load(&state, "index.html") {
        Some(bytes) => respond("index.html", bytes),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The web UI has not been built. Run `bun run build` in web/ and rebuild the server.",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::built_base;

    #[test]
    fn built_base_reads_the_asset_prefix() {
        let root = r#"<script type="module" crossorigin src="/assets/index-a1.js"></script>"#;
        let quant = r#"<link rel="icon" href="/quant/favicon.svg"><script src="/quant/assets/index-a1.js"></script>"#;
        assert_eq!(built_base(root).as_deref(), Some(""));
        assert_eq!(built_base(quant).as_deref(), Some("/quant"));
        assert_eq!(built_base("<html></html>"), None);
    }
}

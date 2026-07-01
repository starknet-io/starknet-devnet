use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

struct Asset {
    path: &'static str,
    bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));

pub(crate) fn routes() -> Router {
    Router::new()
        .route("/ui", get(index))
        .route("/ui/", get(index))
        .route("/ui/{*path}", get(asset))
}

async fn index() -> Response {
    response_for_path("index.html")
}

async fn asset(Path(path): Path<String>) -> Response {
    let Some(path) = normalize_asset_path(&path) else {
        return not_found();
    };

    response_for_path(path)
}

fn response_for_path(path: &str) -> Response {
    match get_asset(path) {
        Some(asset) => asset_response(asset),
        None if should_fallback_to_index(path) => response_for_path("index.html"),
        None => not_found(),
    }
}

fn get_asset(path: &str) -> Option<&'static Asset> {
    UI_ASSETS.iter().find(|asset| asset.path == path)
}

fn normalize_asset_path(path: &str) -> Option<&str> {
    let path = path.trim_start_matches('/');

    if path.is_empty()
        || path.contains('\\')
        || path.split('/').any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }

    Some(path)
}

fn should_fallback_to_index(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|file_name| !file_name.contains('.'))
}

fn asset_response(asset: &'static Asset) -> Response {
    let mut response = Response::new(Body::from(asset.bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type_for_path(asset.path)));

    let cache_control =
        if asset.path == "index.html" { "no-cache" } else { "public, max-age=31536000, immutable" };
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control));

    response
}

fn content_type_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("ico") => "image/x-icon",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("json") | Some("map") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "UI asset not found").into_response()
}

//! Docker registry proxy with transparent blob caching.
//!
//! Architecture:
//!   Client → [packt-proxy] → Upstream Registry
//!              ├── Blob GET:  cache hit → serve local; miss → fetch+tee
//!              ├── Blob POST/PATCH/PUT: pass through, cache on complete
//!              └── Background worker: CDC+dedup cached blobs
//!
//! Passes through all non-blob endpoints (manifests, tags, etc.) unmodified.

use axum::{
    Router,
    extract::{Path, State, Query},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get, head},
    body::Body,
};
use opendal::services::Fs;
use opendal::Operator;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;
use tracing::{info, warn, error};

/// Shared application state.
pub struct AppState {
    pub upstream: String,
    pub client: reqwest::Client,
    pub cache: Operator,
    #[allow(dead_code)]
    pub upstream_host: String,
    pub bytes_proxied: AtomicU64,
    pub bytes_cached: AtomicU64,
}

impl AppState {
    pub fn new(upstream: &str, cache_dir: &PathBuf, _chunk_size: &str) -> anyhow::Result<Self> {
        let builder = Fs::default()
            .root(cache_dir.to_str().unwrap_or("/tmp/packt-proxy-cache"));
        let cache = Operator::new(builder)?.finish();

        let upstream_host = upstream
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();

        Ok(Self {
            upstream: upstream.to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()?,
            cache,
            upstream_host,
            bytes_proxied: AtomicU64::new(0),
            bytes_cached: AtomicU64::new(0),
        })
    }
}

/// Build the axum router with OCI proxy endpoints.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Blob endpoints — intercept for caching
        .route("/v2/{name}/blobs/{digest}", get(handle_blob_get))
        .route("/v2/{name}/blobs/{digest}", head(handle_blob_head))
        // Upload endpoints — pass through
        .route("/v2/{name}/blobs/uploads/{*rest}", any(handle_upstream))
        .route("/v2/{name}/blobs/uploads", any(handle_upstream))
        // Manifest endpoints — pass through
        .route("/v2/{name}/manifests/{reference}", any(handle_upstream))
        // All other /v2/ endpoints — pass through
        .route("/v2/{*rest}", any(handle_upstream))
        .route("/v2/", any(handle_upstream))
        .with_state(state)
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct NsQuery {
    ns: Option<String>,
}

/// Handle GET /v2/{name}/blobs/{digest}
/// Check local cache first, upstream on miss.
async fn handle_blob_get(
    State(state): State<Arc<AppState>>,
    Path((name, digest)): Path<(String, String)>,
    Query(query): Query<NsQuery>,
    headers: HeaderMap,
) -> Response {
    let cache_key = format!("blobs/{}/{}", name, digest);

    // Check local cache
    match state.cache.stat(&cache_key).await {
        Ok(meta) => {
            // Cache hit — serve locally
            info!("Cache HIT: {cache_key} ({} bytes)", meta.content_length());
            match state.cache.read(&cache_key).await {
                Ok(bytes) => {
                    state.bytes_proxied.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    state.bytes_cached.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    return Response::builder()
                        .status(200)
                        .header(header::CONTENT_TYPE, "application/octet-stream")
                        .header(header::CONTENT_LENGTH, bytes.len().to_string())
                        .header("docker-content-digest", &digest)
                        .body(Body::from(bytes.to_vec()))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
                }
                Err(e) => {
                    warn!("Cache read error: {e}, falling back to upstream");
                }
            }
        }
        Err(ref e) if e.kind() == opendal::ErrorKind::NotFound => {
            info!("Cache MISS: {cache_key}");
        }
        Err(e) => {
            warn!("Cache stat error: {e}, falling through to upstream");
        }
    }

    // Cache miss — fetch from upstream
    proxy_upstream_blob(&state, &name, &digest, &cache_key, &query, &headers).await
}

/// Handle HEAD /v2/{name}/blobs/{digest}
async fn handle_blob_head(
    State(state): State<Arc<AppState>>,
    Path((name, digest)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let cache_key = format!("blobs/{}/{}", name, digest);

    match state.cache.stat(&cache_key).await {
        Ok(meta) => {
            return Response::builder()
                .status(200)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CONTENT_LENGTH, meta.content_length().to_string())
                .header("docker-content-digest", &digest)
                .body(Body::empty())
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
        Err(_) => {}
    }

    proxy_upstream_head(&state, &name, &digest, &headers).await
}

/// Fetch blob from upstream, cache locally, stream to client.
async fn proxy_upstream_blob(
    state: &Arc<AppState>,
    name: &str,
    digest: &str,
    cache_key: &str,
    _query: &NsQuery,
    headers: &HeaderMap,
) -> Response {
    let upstream_url = format!("{}/v2/{}/blobs/{}", state.upstream, name, digest);
    let mut req = state.client.get(&upstream_url);

    // Forward auth header if present
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        req = req.header(header::AUTHORIZATION, auth);
    }

    match req.send().await {
        Ok(upstream_resp) => {
            if !upstream_resp.status().is_success() {
                let status = upstream_resp.status();
                let body = upstream_resp.text().await.unwrap_or_default();
                return (status, body).into_response();
            }

            // Stream response body, writing to both client and cache
            // Collect full body for cache write
            match upstream_resp.bytes().await {
                Ok(bytes) => {
                    let len = bytes.len() as u64;
                    state.bytes_proxied.fetch_add(len, Ordering::Relaxed);

                    // Write to cache (background, don't block client response)
                    let cache = state.cache.clone();
                    let key = cache_key.to_string();
                    let data = bytes.to_vec();
                    tokio::spawn(async move {
                        match cache.write(&key, opendal::Buffer::from(data)).await {
                            Ok(_) => info!("Cached: {key}"),
                            Err(e) => warn!("Cache write error: {e}"),
                        }
                    });

                    Response::builder()
                        .status(200)
                        .header(header::CONTENT_TYPE, "application/octet-stream")
                        .header(header::CONTENT_LENGTH, len.to_string())
                        .header("docker-content-digest", digest)
                        .body(Body::from(bytes.to_vec()))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
                }
                Err(e) => {
                    error!("Failed to read upstream body: {e}");
                    StatusCode::BAD_GATEWAY.into_response()
                }
            }
        }
        Err(e) => {
            error!("Upstream request failed: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// Forward HEAD to upstream.
async fn proxy_upstream_head(
    state: &Arc<AppState>,
    name: &str,
    digest: &str,
    headers: &HeaderMap,
) -> Response {
    let upstream_url = format!("{}/v2/{}/blobs/{}", state.upstream, name, digest);
    let mut req = state.client.head(&upstream_url);
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        req = req.header(header::AUTHORIZATION, auth);
    }
    match req.send().await {
        Ok(resp) => {
            let mut response = Response::builder().status(resp.status());
            for (k, v) in resp.headers() {
                response = response.header(k, v);
            }
            response.body(Body::empty())
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(e) => {
            error!("Upstream HEAD failed: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// Generic upstream passthrough handler for non-blob endpoints.
async fn handle_upstream(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
) -> Response {
    let path = req.uri().path();
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let upstream_url = format!("{}{}{}", state.upstream, path, query);

    let method = req.method().clone();
    let mut upstream_req = state.client.request(method.clone(), &upstream_url);

    // Forward headers (skip hop-by-hop)
    for (k, v) in req.headers() {
        let name = k.as_str().to_lowercase();
        if name == "host" || name == "connection" || name == "transfer-encoding" {
            continue;
        }
        upstream_req = upstream_req.header(k, v);
    }

    // Forward body if present
    let body_bytes = axum::body::to_bytes(req.into_body(), 50 * 1024 * 1024).await;
    match body_bytes {
        Ok(bytes) if !bytes.is_empty() => {
            upstream_req = upstream_req.body(bytes.to_vec());
        }
        _ => {}
    }

    match upstream_req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let mut response = Response::builder().status(status);

            // Forward response headers
            for (k, v) in resp.headers() {
                let name = k.as_str().to_lowercase();
                if name == "connection" || name == "transfer-encoding" {
                    continue;
                }
                response = response.header(k, v);
            }

            let body = resp.bytes().await.unwrap_or_default();
            response
                .body(Body::from(body.to_vec()))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(e) => {
            error!("Upstream passthrough failed for {upstream_url}: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

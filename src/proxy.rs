use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde_json::Value;

use crate::clock;
use crate::session;
use crate::upstream::UpstreamClient;

pub const WHITELIST_PREFIX: &str =
    "You are MiMoCode, an interactive CLI tool that helps users with software engineering tasks.";

#[derive(Clone)]
pub struct AppState {
    pub upstream: Arc<UpstreamClient>,
    pub default_model: String,
}

pub async fn chat_completions(State(state): State<AppState>, body: Bytes) -> Response {
    let mut req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return error_json(StatusCode::BAD_REQUEST, "invalid_json", &e.to_string()),
    };

    inject_whitelist_system(&mut req);

    let stream = req.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

    let body_bytes = match serde_json::to_vec(&req) {
        Ok(b) => b,
        Err(e) => {
            return error_json(
                StatusCode::INTERNAL_SERVER_ERROR,
                "encode_failed",
                &e.to_string(),
            );
        }
    };

    let session_id = session::gen_session_affinity(clock::now_ms());
    tracing::debug!(%session_id, stream, "正在将 chat 转发到上游");

    let upstream_resp = match state
        .upstream
        .chat(Bytes::from(body_bytes), &session_id)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("上游错误: {e:#}");
            return error_json(StatusCode::BAD_GATEWAY, "upstream_error", &format!("{e:#}"));
        }
    };

    relay_response(upstream_resp, stream).await
}

async fn relay_response(upstream: reqwest::Response, stream: bool) -> Response {
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    let mut headers = HeaderMap::new();
    for (name, value) in upstream.headers() {
        if !should_strip_response_header(name.as_str()) {
            headers.append(name.clone(), value.clone());
        }
    }

    let body = if stream {
        if !headers.contains_key("cache-control") {
            headers.append("cache-control", HeaderValue::from_static("no-cache"));
        }
        Body::from_stream(upstream.bytes_stream())
    } else {
        match upstream.bytes().await {
            Ok(b) => Body::from(b),
            Err(e) => return error_json(StatusCode::BAD_GATEWAY, "upstream_read", &e.to_string()),
        }
    };

    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    *resp.headers_mut() = headers;
    resp
}

fn should_strip_response_header(name: &str) -> bool {
    if name.starts_with("access-control-") {
        return true;
    }
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
            | "content-encoding"
    )
}

pub fn inject_whitelist_system(req: &mut Value) {
    let messages = match req.get_mut("messages").and_then(|m| m.as_array_mut()) {
        Some(m) => m,
        None => {
            tracing::warn!("请求没有 messages 数组,原样转发");
            return;
        }
    };

    let Some(sys) = messages
        .iter_mut()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
    else {
        messages.insert(
            0,
            serde_json::json!({"role":"system","content":WHITELIST_PREFIX}),
        );
        return;
    };

    if let Some(original) = sys.get("content").and_then(|c| c.as_str()) {
        if !original.starts_with(WHITELIST_PREFIX) {
            sys["content"] = Value::String(format!("{WHITELIST_PREFIX}\n{original}"));
        }
    } else if let Some(parts) = sys.get_mut("content").and_then(|c| c.as_array_mut()) {
        let already = parts
            .first()
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.starts_with(WHITELIST_PREFIX));
        if !already {
            parts.insert(
                0,
                serde_json::json!({"type": "text", "text": WHITELIST_PREFIX}),
            );
        }
    } else {
        sys["content"] = Value::String(WHITELIST_PREFIX.to_string());
    }
}

pub async fn models(State(state): State<AppState>) -> Response {
    json_response(
        StatusCode::OK,
        serde_json::json!({
            "object": "list",
            "data": [
                { "id": state.default_model, "object": "model", "created": 0, "owned_by": "xiaomi" }
            ]
        }),
    )
}

pub async fn health() -> &'static str {
    "ok"
}

fn error_json(status: StatusCode, code: &str, message: &str) -> Response {
    json_response(
        status,
        serde_json::json!({ "error": { "code": code, "message": message } }),
    )
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, axum::Json(body)).into_response()
}

#[cfg(test)]
#[path = "../tests/proxy.rs"]
mod tests;

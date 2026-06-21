use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use base64::Engine;
use bytes::Bytes;
use serde::Deserialize;
use tokio::sync::RwLock;

pub const API_BASE: &str = "https://api.xiaomimimo.com";
const BOOTSTRAP_PATH: &str = "/api/free-ai/bootstrap";
const CHAT_PATH: &str = "/api/free-ai/openai/chat";

pub const UA_BOOTSTRAP: &str = concat!("mimocode/", env!("CARGO_PKG_VERSION"));
pub const UA_CHAT: &str = concat!(
    "mimocode/",
    env!("CARGO_PKG_VERSION"),
    " ai-sdk/provider-utils/4.0.23 runtime/bun/1.3.14"
);
pub const X_MIMO_SOURCE: &str = "mimocode-cli-free";

const RENEW_THRESHOLD_MS: i64 = 5 * 60 * 1000;
const FALLBACK_TTL_MS: i64 = 55 * 60 * 1000;

#[derive(Clone)]
pub struct JwtState {
    pub jwt: String,
    pub exp_ms: i64,
}

pub struct UpstreamClient {
    http: reqwest::Client,
    client_id: String,
    jwt: Arc<RwLock<Option<JwtState>>>,
}

impl UpstreamClient {
    pub async fn new(client_file: &Path) -> Result<Self> {
        let client_id = load_or_create_client(client_file).await?;
        let http = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .context("构建 reqwest 客户端")?;
        Ok(Self {
            http,
            client_id,
            jwt: Arc::new(RwLock::new(None)),
        })
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub async fn chat(&self, body: Bytes, session: &str) -> Result<reqwest::Response> {
        let resp = self.do_chat(&body, session).await?;
        if resp.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(resp);
        }
        tracing::warn!("上游返回 401,正在刷新 JWT 并重试一次");
        let _ = resp.text().await;

        self.bootstrap_force().await?;
        let resp = self.do_chat(&body, session).await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let status = resp.status();
            let _ = resp.text().await;
            anyhow::bail!("刷新 JWT 后上游仍返回 401: {status}");
        }
        Ok(resp)
    }

    async fn do_chat(&self, body: &Bytes, session: &str) -> Result<reqwest::Response> {
        let jwt = self.ensure_jwt().await?;
        let resp = self
            .http
            .post(format!("{API_BASE}{CHAT_PATH}"))
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {jwt}"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("user-agent", UA_CHAT)
            .header("x-mimo-source", X_MIMO_SOURCE)
            .header("x-session-affinity", session)
            .header(reqwest::header::ACCEPT, "*/*")
            .body(body.clone())
            .send()
            .await
            .context("上游 chat 请求")?;
        Ok(resp)
    }

    async fn ensure_jwt(&self) -> Result<String> {
        if let Some(jwt) = fresh_jwt(self.jwt.read().await.as_ref()) {
            return Ok(jwt);
        }
        let mut guard = self.jwt.write().await;
        if let Some(jwt) = fresh_jwt(guard.as_ref()) {
            return Ok(jwt);
        }
        let state = self.fetch_jwt().await?;
        let jwt = state.jwt.clone();
        *guard = Some(state);
        Ok(jwt)
    }

    async fn bootstrap_force(&self) -> Result<()> {
        let state = self.fetch_jwt().await?;
        *self.jwt.write().await = Some(state);
        Ok(())
    }

    async fn fetch_jwt(&self) -> Result<JwtState> {
        #[derive(Deserialize)]
        struct BootResp {
            jwt: String,
        }

        let body = serde_json::to_vec(&serde_json::json!({ "client": self.client_id }))
            .context("序列化 bootstrap 请求体")?;
        let resp = self
            .http
            .post(format!("{API_BASE}{BOOTSTRAP_PATH}"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("user-agent", UA_BOOTSTRAP)
            .header(reqwest::header::ACCEPT, "*/*")
            .body(body)
            .send()
            .await
            .context("bootstrap 请求")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("bootstrap 失败: {status} body={text}");
        }
        let parsed: BootResp = resp
            .json()
            .await
            .context("解析 bootstrap 响应(期望 {\"jwt\":\"...\"})")?;
        let exp_ms = jwt_exp_ms(&parsed.jwt).unwrap_or_else(|| {
            tracing::warn!("无法从 JWT 解析 exp,回退到 55 分钟 ttl");
            crate::clock::now_ms() + FALLBACK_TTL_MS
        });
        tracing::info!(client = %self.client_id, "bootstrap 成功,已获取新 JWT");
        Ok(JwtState {
            jwt: parsed.jwt,
            exp_ms,
        })
    }
}

fn fresh_jwt(state: Option<&JwtState>) -> Option<String> {
    let state = state?;
    (state.exp_ms - crate::clock::now_ms() > RENEW_THRESHOLD_MS).then(|| state.jwt.clone())
}

fn jwt_exp_ms(jwt: &str) -> Option<i64> {
    let mut parts = jwt.split('.');
    let _header = parts.next()?;
    let payload = parts.next()?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    let exp = value.get("exp")?.as_i64()?;
    Some(exp * 1000)
}

async fn load_or_create_client(client_file: &Path) -> Result<String> {
    if let Ok(s) = tokio::fs::read_to_string(client_file).await {
        let trimmed = s.trim().to_string();
        if is_valid_client(&trimmed) {
            tracing::info!(client = %trimmed, "复用已持久化的 client id");
            return Ok(trimmed);
        }
        tracing::warn!(
            file = %client_file.display(),
            "已有 client 文件无效,正在重新生成"
        );
    }
    let id = random_hex(32);
    if let Some(parent) = client_file.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("创建目录 {}", parent.display()))?;
    }
    tokio::fs::write(client_file, &id)
        .await
        .with_context(|| format!("写入 client 文件 {}", client_file.display()))?;
    tracing::info!(client = %id, "已生成并持久化新的 client id");
    Ok(id)
}

fn is_valid_client(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn random_hex(nbytes: usize) -> String {
    let mut buf = vec![0u8; nbytes];
    getrandom::fill(&mut buf).expect("getrandom 失败");
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
#[path = "../tests/upstream.rs"]
mod tests;

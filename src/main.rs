mod clock;
mod config;
mod proxy;
mod session;
mod upstream;

use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,mimocode2api=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config = Config::from_env();
    tracing::info!(
        port = config.port,
        model = %config.default_model,
        "正在启动 mimocode2api 代理"
    );

    let upstream = Arc::new(upstream::UpstreamClient::new(&config.client_file).await?);
    tracing::info!(client = upstream.client_id(), "client id 就绪");

    let state = proxy::AppState {
        upstream,
        default_model: config.default_model.clone(),
    };

    let app = Router::new()
        .route("/v1/chat/completions", post(proxy::chat_completions))
        .route("/v1/models", get(proxy::models))
        .route("/health", get(proxy::health))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .layer(DefaultBodyLimit::disable())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    tracing::info!("正在监听 http://0.0.0.0:{}", config.port);
    axum::serve(listener, app).await?;
    Ok(())
}

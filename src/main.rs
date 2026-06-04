mod config;
mod crypto;
mod db;
mod engine;
mod models;

use axum::{routing::get, Json, Router};
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::config::Config;

#[tokio::main]
async fn main() {
    // 加载 .env(开发期);生产由 systemd EnvironmentFile 注入。
    let _ = dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "instatt_saas=info".into()),
        )
        .init();

    let cfg = Config::from_env();

    let pool = db::connect_and_migrate(&cfg.database_url)
        .await
        .expect("数据库连接/迁移失败");
    db::seed::seed(&pool, &cfg.superadmin_username, &cfg.superadmin_password)
        .await
        .expect("seed 失败");
    tracing::info!("数据库就绪,迁移与 seed 完成");

    let app = Router::new()
        .route("/api/health", get(health))
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .expect("绑定监听地址失败");
    tracing::info!("listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await.expect("HTTP 服务异常退出");
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

/// 极简 .env 读取(避免额外依赖):逐行 KEY=VALUE 注入未设置的环境变量。
fn dotenv() -> std::io::Result<()> {
    let content = std::fs::read_to_string(".env")?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            if std::env::var(k).is_err() {
                std::env::set_var(k, v.trim());
            }
        }
    }
    Ok(())
}

mod api;
mod auth;
mod config;
mod crypto;
mod db;
mod engine;
mod models;
mod state;

use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::engine::instatt::InstAttClient;
use crate::state::AppState;

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

    // 启动签到引擎(轮询器 + 每日刷新器)
    let engine = engine::run(pool.clone(), cfg.encryption_key).await;
    tracing::info!("签到引擎已启动");

    let state = AppState {
        pool: pool.clone(),
        jwt_secret: cfg.jwt_secret.clone(),
        enc_key: cfg.encryption_key,
        client: InstAttClient::new(),
        engine,
        superadmin_username: cfg.superadmin_username.clone(),
        cookie_secure: cfg.cookie_secure,
    };

    let app = api::router(state).layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr)
        .await
        .expect("绑定监听地址失败");
    tracing::info!("listening on {}", cfg.bind_addr);
    axum::serve(listener, app).await.expect("HTTP 服务异常退出");
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

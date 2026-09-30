//! 签到引擎:上游客户端、token 管理、轮询器、worker、计费。
//! 模块边界清晰,将来可整体拆为独立 binary。

pub mod billing;
pub mod instatt;
pub mod listener;
pub mod poller;
pub mod refresher;
pub mod tokens;
pub mod worker;

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use sqlx::PgPool;
use tokio::sync::Semaphore;

use crate::engine::instatt::InstAttClient;
use crate::engine::tokens::TokenManager;
use crate::engine::worker::Engine;

/// 读取 system_config 中的整数项,缺失/非法用默认值。
pub async fn config_i64(pool: &PgPool, key: &str, default: i64) -> i64 {
    let v: Option<String> = sqlx::query_scalar("SELECT value FROM system_config WHERE key=$1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    v.and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// 构建引擎并启动轮询器 + 每日刷新器。返回 Arc<Engine> 供 API 复用。
pub async fn run(pool: PgPool, enc_key: [u8; 32]) -> Arc<Engine> {
    let price_cents = config_i64(&pool, "price_per_sign_cents", 100).await;
    let poll_interval_sec = config_i64(&pool, "poll_interval_sec", 5).await;
    let max_concurrency = config_i64(&pool, "max_concurrency", 50).await.max(1) as usize;
    let jitter_ms_max = config_i64(&pool, "jitter_ms_max", 1500).await.max(0) as u64;
    // 实时监听 ongoingClasses(1 开 / 0 关);关了就只剩定时轮询
    let realtime_listen = config_i64(&pool, "realtime_listen", 1).await != 0;

    let client = InstAttClient::new();
    let tokens = Arc::new(TokenManager::new(pool.clone(), client.clone(), enc_key));
    let engine = Arc::new(Engine {
        pool,
        tokens: tokens.clone(),
        client,
        enc_key,
        price_cents,
        poll_interval: Duration::from_secs(poll_interval_sec.max(1) as u64),
        jitter_ms_max,
        sem: Arc::new(Semaphore::new(max_concurrency)),
        acct_locks: DashMap::new(),
        inflight: DashMap::new(),
        wake: tokio::sync::Notify::new(),
    });

    tokio::spawn(poller::run_poller(engine.clone()));
    tokio::spawn(refresher::run_refresher(tokens));
    if realtime_listen {
        tokio::spawn(listener::run_listener(engine.clone()));
    } else {
        tracing::info!("realtime_listen=0,仅定时轮询");
    }
    engine
}

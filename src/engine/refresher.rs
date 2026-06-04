//! 每日刷新器:周期性跑完整链保活长 token,提前发现失效账号。

use std::sync::Arc;
use std::time::Duration;

use crate::engine::tokens::TokenManager;

const DAY: Duration = Duration::from_secs(24 * 3600);

pub async fn run_refresher(tokens: Arc<TokenManager>) {
    loop {
        tokio::time::sleep(DAY).await;
        if let Err(e) = tokens.daily_refresh_all().await {
            tracing::warn!("每日刷新出错: {e}");
        }
    }
}

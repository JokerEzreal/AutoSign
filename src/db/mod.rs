//! 数据库连接池与迁移。

pub mod seed;

use sqlx::postgres::{PgPool, PgPoolOptions};

/// 创建连接池并执行嵌入式迁移(migrations/ 目录)。
pub async fn connect_and_migrate(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

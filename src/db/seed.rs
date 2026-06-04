//! 启动期 seed:内置超管账号 + 默认 system_config。幂等。

use sqlx::PgPool;

use crate::auth::password::hash as hash_password;

/// 默认运行期配置(键, 值)。
const DEFAULT_CONFIG: &[(&str, &str)] = &[
    ("price_per_sign_cents", "100"),
    ("poll_interval_sec", "5"),
    ("max_concurrency", "50"),
    ("jitter_ms_max", "1500"),
];

/// seed 超管(若不存在)与默认配置(缺哪个补哪个)。可重复调用。
pub async fn seed(pool: &PgPool, superadmin_username: &str, superadmin_password: &str) -> anyhow::Result<()> {
    // 默认配置:存在则跳过
    for (k, v) in DEFAULT_CONFIG {
        sqlx::query("INSERT INTO system_config (key, value) VALUES ($1, $2) ON CONFLICT (key) DO NOTHING")
            .bind(k)
            .bind(v)
            .execute(pool)
            .await?;
    }

    // 超管:仅当该用户名不存在时插入
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT id FROM super_admin WHERE username = $1")
            .bind(superadmin_username)
            .fetch_optional(pool)
            .await?;
    if exists.is_none() {
        let hash = hash_password(superadmin_password)?;
        sqlx::query("INSERT INTO super_admin (username, password_hash) VALUES ($1, $2)")
            .bind(superadmin_username)
            .bind(hash)
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn seed_is_idempotent(pool: PgPool) {
        seed(&pool, "admin", "changeme-strong-pass").await.unwrap();
        seed(&pool, "admin", "changeme-strong-pass").await.unwrap();

        let admin_count: i64 = sqlx::query_scalar("SELECT count(*) FROM super_admin")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(admin_count, 1, "重复 seed 不应产生多条超管");

        let cfg_count: i64 = sqlx::query_scalar("SELECT count(*) FROM system_config")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(cfg_count as usize, DEFAULT_CONFIG.len());

        let price: String =
            sqlx::query_scalar("SELECT value FROM system_config WHERE key='price_per_sign_cents'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(price, "100");
    }
}

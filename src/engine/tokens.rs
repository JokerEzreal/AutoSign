//! Token 管理器:两层续期 + 按账号 single-flight 去重。
//!
//! 第 1 层(本模块 get_valid_id_token):签到前按需刷新短 token(Firebase ID Token,
//!   缓存 10 分钟)。第 2 层(daily_refresh_all):每日跑完整 Azure→Firebase 链保活长 token。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use sqlx::PgPool;
use tokio::sync::Mutex;

use crate::crypto;
use crate::engine::instatt::InstAttClient;

const CACHE_TTL: Duration = Duration::from_secs(600); // 10 分钟

struct Cached {
    id_token: String,
    expires_at: Instant,
}

pub struct TokenManager {
    pool: PgPool,
    client: InstAttClient,
    enc_key: [u8; 32],
    cache: DashMap<i64, Cached>,
    locks: DashMap<i64, Arc<Mutex<()>>>,
}

impl TokenManager {
    pub fn new(pool: PgPool, client: InstAttClient, enc_key: [u8; 32]) -> Self {
        TokenManager {
            pool,
            client,
            enc_key,
            cache: DashMap::new(),
            locks: DashMap::new(),
        }
    }

    fn lock_for(&self, account_id: i64) -> Arc<Mutex<()>> {
        self.locks
            .entry(account_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn cached(&self, account_id: i64) -> Option<String> {
        self.cache.get(&account_id).and_then(|c| {
            if Instant::now() < c.expires_at {
                Some(c.id_token.clone())
            } else {
                None
            }
        })
    }

    fn store(&self, account_id: i64, id_token: String) {
        self.cache.insert(
            account_id,
            Cached {
                id_token,
                expires_at: Instant::now() + CACHE_TTL,
            },
        );
    }

    /// 取有效短 token。缓存命中直接返回;否则按账号 single-flight 刷新。
    pub async fn get_valid_id_token(&self, account_id: i64) -> anyhow::Result<String> {
        if let Some(t) = self.cached(account_id) {
            return Ok(t);
        }
        let lock = self.lock_for(account_id);
        let _guard = lock.lock().await;
        // 二次检查:可能在等锁期间已有其他任务刷新好了
        if let Some(t) = self.cached(account_id) {
            return Ok(t);
        }

        let (firebase_rt, azure_rt) = self.load_refresh_tokens(account_id).await?;

        // 第 1 层便宜路径:Firebase RT 直接刷短 token
        if !firebase_rt.is_empty() {
            if let Ok(id) = self.client.refresh_firebase(&firebase_rt).await {
                self.store(account_id, id.clone());
                return Ok(id);
            }
        }

        // 兜底:完整 Azure→Firebase 链(并轮换持久化新 firebase_rt)
        if !azure_rt.is_empty() {
            match self.refresh_full(account_id, &azure_rt).await {
                Ok(id) => {
                    self.store(account_id, id.clone());
                    return Ok(id);
                }
                Err(e) => {
                    tracing::warn!("[{account_id}] 完整刷新失败: {e}");
                }
            }
        }

        self.mark_needs_relogin(account_id).await;
        Err(anyhow::anyhow!("账号 {account_id} token 刷新彻底失败,需重新登录"))
    }

    /// 跑完整链:azure access → firebase custom → firebase id token,
    /// 持久化新的 firebase_rt,更新 last_refresh_at。返回新的 id_token。
    async fn refresh_full(&self, account_id: i64, azure_rt: &str) -> anyhow::Result<String> {
        let azure_access = self.client.refresh_azure(azure_rt).await?;
        let custom = self.client.firebase_custom_token(&azure_access).await?;
        let (id_token, new_fb_rt) = self.client.firebase_id_token(&custom).await?;
        let enc = crypto::encrypt_str(&self.enc_key, &new_fb_rt)?;
        sqlx::query("UPDATE accounts SET firebase_rt=$1, last_refresh_at=now(), updated_at=now() WHERE id=$2")
            .bind(enc)
            .bind(account_id)
            .execute(&self.pool)
            .await?;
        Ok(id_token)
    }

    /// 第 2 层:每日对所有 active 账号跑完整链保活长 token,失败标记 needs_relogin。
    pub async fn daily_refresh_all(&self) -> anyhow::Result<()> {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM accounts WHERE status='active'",
        )
        .fetch_all(&self.pool)
        .await?;
        tracing::info!("每日刷新:{} 个 active 账号", ids.len());
        for id in ids {
            let (_, azure_rt) = match self.load_refresh_tokens(id).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            if azure_rt.is_empty() {
                self.mark_needs_relogin(id).await;
                continue;
            }
            match self.refresh_full(id, &azure_rt).await {
                Ok(id_token) => self.store(id, id_token),
                Err(e) => {
                    tracing::warn!("[{id}] 每日刷新失败: {e}");
                    self.mark_needs_relogin(id).await;
                }
            }
        }
        Ok(())
    }

    /// 读取并解密 (firebase_rt, azure_rt)。
    async fn load_refresh_tokens(&self, account_id: i64) -> anyhow::Result<(String, String)> {
        let row: (Option<Vec<u8>>, Option<Vec<u8>>) = sqlx::query_as(
            "SELECT firebase_rt, azure_rt FROM accounts WHERE id=$1",
        )
        .bind(account_id)
        .fetch_one(&self.pool)
        .await?;
        let fb = crypto::decrypt_str(&self.enc_key, &row.0.unwrap_or_default())?;
        let az = crypto::decrypt_str(&self.enc_key, &row.1.unwrap_or_default())?;
        Ok((fb, az))
    }

    async fn mark_needs_relogin(&self, account_id: i64) {
        let _ = sqlx::query("UPDATE accounts SET status='needs_relogin', updated_at=now() WHERE id=$1 AND status='active'")
            .bind(account_id)
            .execute(&self.pool)
            .await;
    }
}

/// 占位,消除未使用告警。
#[allow(dead_code)]
fn _unused() -> HashMap<String, String> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    const KEY: [u8; 32] = *b"0123456789abcdef0123456789abcdef";

    /// 计数每次被调用的 responder。
    struct Counting {
        hits: Arc<AtomicUsize>,
        body: serde_json::Value,
    }
    impl Respond for Counting {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            self.hits.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(self.body.clone())
        }
    }

    async fn insert_account(pool: &PgPool, firebase_rt: &str, azure_rt: &str) -> i64 {
        let fb = crypto::encrypt_str(&KEY, firebase_rt).unwrap();
        let az = crypto::encrypt_str(&KEY, azure_rt).unwrap();
        sqlx::query_scalar(
            "INSERT INTO accounts (account_name, firebase_rt, azure_rt) VALUES ($1,$2,$3) RETURNING id",
        )
        .bind("tester")
        .bind(fb)
        .bind(az)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    async fn single_flight_refreshes_once(pool: PgPool) {
        let server = MockServer::start().await;
        let hits = Arc::new(AtomicUsize::new(0));
        Mock::given(method("POST"))
            .and(path("/v1/token"))
            .respond_with(Counting {
                hits: hits.clone(),
                body: serde_json::json!({"id_token": "SHORT_TOK"}),
            })
            .mount(&server)
            .await;
        let base = server.uri();
        let client = InstAttClient::with_bases(&base, &base, &base, &base, &base, &base);
        let id = insert_account(&pool, "fbrt", "").await;
        let tm = Arc::new(TokenManager::new(pool, client, KEY));

        // 20 个并发请求,缓存初始为空 → single-flight 只应刷新一次
        let mut handles = vec![];
        for _ in 0..20 {
            let tm = tm.clone();
            handles.push(tokio::spawn(async move { tm.get_valid_id_token(id).await }));
        }
        for h in handles {
            assert_eq!(h.await.unwrap().unwrap(), "SHORT_TOK");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1, "single-flight 应只刷新一次");
    }

    #[sqlx::test]
    async fn falls_back_to_azure_chain(pool: PgPool) {
        let server = MockServer::start().await;
        // Firebase 短刷失败(无 id_token)
        Mock::given(method("POST"))
            .and(path("/v1/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"error": "bad"})))
            .mount(&server)
            .await;
        // Azure refresh 成功
        Mock::given(method("POST"))
            .and(path(format!("/{}/oauth2/v2.0/token", crate::engine::instatt::AZURE_TENANT_ID)))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token": "AZ_ACC"})))
            .mount(&server)
            .await;
        // Firebase custom token
        Mock::given(method("POST"))
            .and(path("/userLogin"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"result": {"statusCode": 200, "token": "CUSTOM"}})))
            .mount(&server)
            .await;
        // Firebase id token(走完整链)
        Mock::given(method("POST"))
            .and(path("/v1/accounts:signInWithCustomToken"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"idToken": "FULL_TOK", "refreshToken": "NEW_FBRT"})))
            .mount(&server)
            .await;
        let base = server.uri();
        let client = InstAttClient::with_bases(&base, &base, &base, &base, &base, &base);
        let id = insert_account(&pool, "stale_fbrt", "azrt").await;
        let tm = TokenManager::new(pool.clone(), client, KEY);

        let tok = tm.get_valid_id_token(id).await.unwrap();
        assert_eq!(tok, "FULL_TOK");

        // 新的 firebase_rt 应已持久化(加密)
        let stored: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT firebase_rt FROM accounts WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let dec = crypto::decrypt_str(&KEY, &stored.unwrap()).unwrap();
        assert_eq!(dec, "NEW_FBRT");
    }

    #[sqlx::test]
    async fn total_failure_marks_needs_relogin(pool: PgPool) {
        let server = MockServer::start().await;
        // 所有刷新都失败
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"error": "bad"})))
            .mount(&server)
            .await;
        let base = server.uri();
        let client = InstAttClient::with_bases(&base, &base, &base, &base, &base, &base);
        let id = insert_account(&pool, "fbrt", "azrt").await;
        let tm = TokenManager::new(pool.clone(), client, KEY);

        assert!(tm.get_valid_id_token(id).await.is_err());
        let status: String = sqlx::query_scalar("SELECT status FROM accounts WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "needs_relogin");
    }
}

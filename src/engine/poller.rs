//! 共享轮询器:每 N 秒拉一次全局 ongoingClasses(O(1),与用户数无关),
//! 匹配订阅该课程的账号并扇出签到 job。去重靠 inflight + sign_records 唯一约束。

use std::collections::HashSet;
use std::sync::Arc;

use tokio::task::JoinHandle;

use crate::engine::instatt::OngoingClass;
use crate::engine::worker::{job_key, Engine};

/// 账号模块是否匹配该课程代码(对应 Python 的 module_code.upper() in m.upper())。
fn modules_match(modules: &[String], module_code: &str) -> bool {
    let code = module_code.to_uppercase();
    modules.iter().any(|m| m.to_uppercase().contains(&code))
}

/// 拉一次并扇出 job。返回本轮新派发的 job 句柄(生产环境忽略,测试用于等待)。
pub async fn poll_once(engine: Arc<Engine>) -> Vec<JoinHandle<()>> {
    let classes = match engine.client.get_ongoing_classes().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("拉取 ongoingClasses 失败: {e}");
            return vec![];
        }
    };
    if classes.is_empty() {
        return vec![];
    }

    // 最近 24h 已处理的课程键(跨重启幂等,避免重复上游调用)
    let signed: HashSet<String> = match load_recent_signed_keys(&engine).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("加载已签记录失败: {e}");
            return vec![];
        }
    };

    // 活跃且可签的账号(余额 ≥ 单价、开启挂机)
    let accounts: Vec<(i64, Vec<String>)> = match load_signable_accounts(&engine).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("加载账号失败: {e}");
            return vec![];
        }
    };

    let mut handles = vec![];
    for class in &classes {
        let code = class.module_code().to_string();
        for (account_id, modules) in &accounts {
            if !modules_match(modules, &code) {
                continue;
            }
            let key = job_key(*account_id, class);
            if signed.contains(&key) || engine.inflight.contains_key(&key) {
                continue;
            }
            engine.inflight.insert(key, ());
            let eng = engine.clone();
            let c: OngoingClass = class.clone();
            let aid = *account_id;
            handles.push(tokio::spawn(eng.process_job(aid, c)));
        }
    }
    handles
}

async fn load_recent_signed_keys(engine: &Engine) -> anyhow::Result<HashSet<String>> {
    let rows: Vec<(i64, String, i64, i64)> = sqlx::query_as(
        "SELECT account_id, module_key, class_date, start_time FROM sign_records
         WHERE created_at > now() - interval '24 hours'",
    )
    .fetch_all(&engine.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(aid, mk, cd, st)| format!("{aid}|{mk}|{cd}|{st}"))
        .collect())
}

async fn load_signable_accounts(engine: &Engine) -> anyhow::Result<Vec<(i64, Vec<String>)>> {
    let rows: Vec<(i64, serde_json::Value)> = sqlx::query_as(
        "SELECT id, my_modules FROM accounts
         WHERE status='active' AND auto_sign = true AND balance_cents >= $1",
    )
    .bind(engine.price_cents)
    .fetch_all(&engine.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, mods)| (id, serde_json::from_value(mods).unwrap_or_default()))
        .collect())
}

/// 生产轮询循环:周期性 poll_once,不阻塞等待 job。
pub async fn run_poller(engine: Arc<Engine>) {
    tracing::info!(
        "签到轮询启动,间隔 {:?},并发上限 {}",
        engine.poll_interval,
        engine.sem.available_permits()
    );
    loop {
        let _ = poll_once(engine.clone()).await;
        tokio::time::sleep(engine.poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use dashmap::DashMap;
    use sqlx::PgPool;
    use tokio::sync::Semaphore;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::crypto;
    use crate::engine::instatt::{InstAttClient, FIREBASE_PROJECT_ID};
    use crate::engine::tokens::TokenManager;

    const KEY: [u8; 32] = *b"0123456789abcdef0123456789abcdef";

    #[sqlx::test]
    async fn end_to_end_poll_signs_and_charges(pool: PgPool) {
        let server = MockServer::start().await;
        // 解锁课程:COMP4082 @ F3C04(有 BSSID)
        Mock::given(method("GET"))
            .and(path(format!(
                "/v1/projects/{}/databases/(default)/documents/ongoingClasses",
                FIREBASE_PROJECT_ID
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "documents": [{"fields": {
                    "moduleKey": {"stringValue": "COMP4082_FML_25-26"},
                    "moduleName": {"stringValue": "AI"},
                    "venue": {"stringValue": "F3C04"},
                    "classDate": {"integerValue": "20251126"},
                    "startTime": {"integerValue": "900"},
                    "endTime": {"integerValue": "1100"}
                }}]
            })))
            .mount(&server)
            .await;
        // 短 token 刷新
        Mock::given(method("POST"))
            .and(path("/v1/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id_token": "TOK"})))
            .mount(&server)
            .await;
        // 签到成功
        Mock::given(method("POST"))
            .and(path("/signAttendance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"result": {"statusCode": 200, "body": "ok"}})))
            .mount(&server)
            .await;

        // 账号:有设备、firebase_rt、订阅 COMP4082、余额 500
        let dev = crypto::encrypt_str(&KEY, "12345678abcd").unwrap();
        let fb = crypto::encrypt_str(&KEY, "fbrt").unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO accounts (account_name, student_id, device_uid, firebase_rt, my_modules, balance_cents)
             VALUES ('t','12345678',$1,$2,$3,500) RETURNING id",
        )
        .bind(dev)
        .bind(fb)
        .bind(serde_json::json!(["COMP4082"]))
        .fetch_one(&pool)
        .await
        .unwrap();

        let base = server.uri();
        let client = InstAttClient::with_bases(&base, &base, &base, &base, &base, &base);
        let tokens = Arc::new(TokenManager::new(pool.clone(), client.clone(), KEY));
        let engine = Arc::new(Engine {
            pool: pool.clone(),
            tokens,
            client,
            enc_key: KEY,
            price_cents: 100,
            poll_interval: Duration::from_secs(5),
            jitter_ms_max: 0,
            sem: Arc::new(Semaphore::new(50)),
            acct_locks: DashMap::new(),
            inflight: DashMap::new(),
        });

        let handles = poll_once(engine.clone()).await;
        assert_eq!(handles.len(), 1, "应派发 1 个 job");
        for h in handles {
            h.await.unwrap();
        }

        // 验证:签到记录 ok_200 charged 100,余额 400
        let (result, charged): (String, i64) = sqlx::query_as(
            "SELECT result, charged_cents FROM sign_records WHERE account_id=$1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(result, "ok_200");
        assert_eq!(charged, 100);
        let bal: i64 = sqlx::query_scalar("SELECT balance_cents FROM accounts WHERE id=$1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(bal, 400);

        // 再轮询一次:幂等,不应重复扣费
        let handles2 = poll_once(engine.clone()).await;
        for h in handles2 { h.await.unwrap(); }
        let bal2: i64 = sqlx::query_scalar("SELECT balance_cents FROM accounts WHERE id=$1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(bal2, 400, "重复轮询不应再扣费");
    }
}

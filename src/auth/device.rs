//! Device Code 登录:= 登录 + 绑定 + 建账号(按 account_name upsert)。

use chrono::Utc;
use serde::Serialize;
use sqlx::PgPool;

use crate::crypto;
use crate::engine::instatt::{DevicePoll, InstAttClient};

#[derive(Debug, Serialize)]
pub struct StartResp {
    pub session_id: i64,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: i64,
    pub expires_in: i64,
}

#[derive(Debug)]
pub enum PollResult {
    Pending,
    Declined,
    Expired,
    Done { account_id: i64, role: String },
}

/// 登录链解析出的账号数据。
#[derive(Debug, Clone)]
pub struct ResolvedLogin {
    pub account_name: String,
    pub student_id: String,
    pub device_uid: String,
    pub azure_rt: String,
    pub firebase_rt: String,
    pub modules: Vec<String>,
}

/// 发起 Device Code,落库一条 pending 会话。
pub async fn start_device_login(pool: &PgPool, client: &InstAttClient) -> anyhow::Result<StartResp> {
    let dc = client.device_code_start().await?;
    let expires_at = Utc::now() + chrono::Duration::seconds(dc.expires_in);
    let session_id: i64 = sqlx::query_scalar(
        "INSERT INTO device_code_sessions (device_code, user_code, verification_uri, interval_sec, expires_at)
         VALUES ($1,$2,$3,$4,$5) RETURNING id",
    )
    .bind(&dc.device_code)
    .bind(&dc.user_code)
    .bind(&dc.verification_uri)
    .bind(dc.interval)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    Ok(StartResp {
        session_id,
        user_code: dc.user_code,
        verification_uri: dc.verification_uri,
        interval: dc.interval,
        expires_in: dc.expires_in,
    })
}

/// 轮询会话。成功则跑完整登录链并 upsert 账号。
pub async fn poll_device_login(
    pool: &PgPool,
    client: &InstAttClient,
    enc_key: &[u8; 32],
    session_id: i64,
) -> anyhow::Result<PollResult> {
    let row: Option<(String, chrono::DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT device_code, expires_at, status FROM device_code_sessions WHERE id=$1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    let (device_code, expires_at, status) = match row {
        Some(r) => r,
        None => return Ok(PollResult::Expired),
    };
    if status == "done" {
        // 已完成的会话:返回其账号
        if let Some(name) = result_account(pool, session_id).await? {
            if let Some((id, role)) = account_id_role(pool, &name).await? {
                return Ok(PollResult::Done { account_id: id, role });
            }
        }
    }
    if Utc::now() > expires_at {
        set_session_status(pool, session_id, "expired", None).await?;
        return Ok(PollResult::Expired);
    }

    match client.device_code_poll(&device_code).await? {
        DevicePoll::Pending => Ok(PollResult::Pending),
        DevicePoll::Declined => {
            set_session_status(pool, session_id, "error", None).await?;
            Ok(PollResult::Declined)
        }
        DevicePoll::Expired => {
            set_session_status(pool, session_id, "expired", None).await?;
            Ok(PollResult::Expired)
        }
        DevicePoll::Success { access_token, refresh_token } => {
            let login = resolve_login(client, &access_token, &refresh_token).await?;
            let (account_id, role) = upsert_account(pool, enc_key, &login).await?;
            set_session_status(pool, session_id, "done", Some(&login.account_name)).await?;
            Ok(PollResult::Done { account_id, role })
        }
    }
}

/// 用 Azure access/refresh 跑完整链,拿账号名/学号/设备/课程。
async fn resolve_login(
    client: &InstAttClient,
    azure_access: &str,
    azure_refresh: &str,
) -> anyhow::Result<ResolvedLogin> {
    let user = client.get_user_info(azure_access).await?;
    let account_name = user
        .mail
        .split('@')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let custom = client.firebase_custom_token(azure_access).await?;
    let (id_token, firebase_rt) = client.firebase_id_token(&custom).await?;

    let numeric_id = user.employee_id.clone();
    let device_uid = if !numeric_id.is_empty() {
        client
            .get_student_info(&id_token, &numeric_id)
            .await?
            .map(|s| s.device_uid)
            .unwrap_or_default()
    } else {
        String::new()
    };
    let student_id = if !numeric_id.is_empty() {
        numeric_id
    } else {
        device_uid.chars().take(8).collect()
    };
    let modules = if !student_id.is_empty() {
        client
            .get_student_modules(&id_token, &student_id, "25-26")
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.module_id)
            .collect()
    } else {
        vec![]
    };

    Ok(ResolvedLogin {
        account_name,
        student_id,
        device_uid,
        azure_rt: azure_refresh.to_string(),
        firebase_rt,
        modules,
    })
}

/// 按 account_name upsert 账号(加密敏感字段)。返回 (id, role)。
/// 设备/课程为空时保留旧值(避免某次上游查询失败抹掉数据)。
pub async fn upsert_account(
    pool: &PgPool,
    enc_key: &[u8; 32],
    login: &ResolvedLogin,
) -> anyhow::Result<(i64, String)> {
    let dev_enc: Option<Vec<u8>> = if login.device_uid.is_empty() {
        None
    } else {
        Some(crypto::encrypt_str(enc_key, &login.device_uid)?)
    };
    let modules_json: Option<serde_json::Value> = if login.modules.is_empty() {
        None
    } else {
        Some(serde_json::json!(login.modules))
    };
    let az_enc = crypto::encrypt_str(enc_key, &login.azure_rt)?;
    let fb_enc = crypto::encrypt_str(enc_key, &login.firebase_rt)?;

    let (id, role): (i64, String) = sqlx::query_as(
        "INSERT INTO accounts
            (account_name, student_id, device_uid, azure_rt, firebase_rt, my_modules, status, last_synced_at, last_refresh_at)
         VALUES ($1,$2,$3,$4,$5, COALESCE($6,'[]'::jsonb), 'active', now(), now())
         ON CONFLICT (account_name) DO UPDATE SET
            student_id = CASE WHEN EXCLUDED.student_id <> '' THEN EXCLUDED.student_id ELSE accounts.student_id END,
            device_uid = COALESCE($3, accounts.device_uid),
            azure_rt = EXCLUDED.azure_rt,
            firebase_rt = EXCLUDED.firebase_rt,
            my_modules = COALESCE($6, accounts.my_modules),
            status = 'active',
            last_synced_at = now(), last_refresh_at = now(), updated_at = now()
         RETURNING id, role",
    )
    .bind(&login.account_name)
    .bind(&login.student_id)
    .bind(dev_enc)
    .bind(az_enc)
    .bind(fb_enc)
    .bind(modules_json)
    .fetch_one(pool)
    .await?;
    Ok((id, role))
}

async fn set_session_status(pool: &PgPool, id: i64, status: &str, account_name: Option<&str>) -> anyhow::Result<()> {
    sqlx::query("UPDATE device_code_sessions SET status=$1, result_account_name=$2 WHERE id=$3")
        .bind(status)
        .bind(account_name)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn result_account(pool: &PgPool, id: i64) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar("SELECT result_account_name FROM device_code_sessions WHERE id=$1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .flatten())
}

async fn account_id_role(pool: &PgPool, account_name: &str) -> anyhow::Result<Option<(i64, String)>> {
    Ok(sqlx::query_as("SELECT id, role FROM accounts WHERE account_name=$1")
        .bind(account_name)
        .fetch_optional(pool)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = *b"0123456789abcdef0123456789abcdef";

    fn login(modules: Vec<&str>, device: &str) -> ResolvedLogin {
        ResolvedLogin {
            account_name: "niubi666".into(),
            student_id: "12345678".into(),
            device_uid: device.into(),
            azure_rt: "az".into(),
            firebase_rt: "fb".into(),
            modules: modules.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    #[sqlx::test]
    async fn upsert_creates_then_updates_preserving_device(pool: PgPool) {
        // 首次:有设备 + 课程
        let (id1, role) = upsert_account(&pool, &KEY, &login(vec!["COMP4082"], "12345678abcd")).await.unwrap();
        assert_eq!(role, "user");

        // 二次:同账号,但本次未取到设备/课程(空)→ 应保留旧值,且 id 不变
        let (id2, _) = upsert_account(&pool, &KEY, &login(vec![], "")).await.unwrap();
        assert_eq!(id1, id2, "同 account_name 应更新而非新建");

        let (dev_enc, mods): (Option<Vec<u8>>, serde_json::Value) =
            sqlx::query_as("SELECT device_uid, my_modules FROM accounts WHERE id=$1")
                .bind(id1).fetch_one(&pool).await.unwrap();
        let dev = crypto::decrypt_str(&KEY, &dev_enc.unwrap()).unwrap();
        assert_eq!(dev, "12345678abcd", "设备应保留");
        let mods: Vec<String> = serde_json::from_value(mods).unwrap();
        assert_eq!(mods, vec!["COMP4082".to_string()], "课程应保留");

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE account_name='niubi666'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);
    }
}

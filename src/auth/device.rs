//! Device Code 登录:= 登录 + 绑定 + 建账号(按 account_name upsert)。

use chrono::Utc;
use serde::Serialize;
use sqlx::PgPool;

use crate::crypto;
use crate::engine::instatt::{current_academic_year, DevicePoll, InstAttClient};

/// 新用户注册赠送余额(分)。按单价 200 分/次,¥4 = 2 次免费签到。
pub const REGISTER_BONUS_CENTS: i64 = 400;

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
    pub course: String,
    pub azure_rt: String,
    pub firebase_rt: String,
    pub modules: Vec<String>,
    /// { 课程代码: 课程名 }
    pub module_info: serde_json::Value,
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

/// 账号密码(ROPC)登录:账号密码 → Azure token → 跑完整链 → upsert。返回 (account_id, role)。
///
/// 相比 Device Code 的优点:用户只需在本站输入账号密码,后台静默完成授权,无需去微软页面贴码。
/// 代价:后台会短暂接触用户的学校密码(只用于换 token、用完即弃,不落库);
/// 且该流程不支持 MFA / 条件访问——账号若被强制 MFA 会失败,需回退 Device Code。
pub async fn password_login(
    pool: &PgPool,
    client: &InstAttClient,
    enc_key: &[u8; 32],
    username: &str,
    password: &str,
) -> anyhow::Result<(i64, String)> {
    let (access_token, refresh_token) = client.login_password(username, password).await?;
    let login = resolve_login(client, &access_token, &refresh_token).await?;
    upsert_account(pool, enc_key, &login).await
}

/// 已拿到 Azure access/refresh token 后完成登录(供辅助登录 worker 回收 token 后复用)。
/// 跑完整解析链 + upsert,返回 (account_id, role)。
pub async fn finalize_from_azure_tokens(
    pool: &PgPool,
    client: &InstAttClient,
    enc_key: &[u8; 32],
    azure_access: &str,
    azure_refresh: &str,
) -> anyhow::Result<(i64, String)> {
    let login = resolve_login(client, azure_access, azure_refresh).await?;
    upsert_account(pool, enc_key, &login).await
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
    let info = if !numeric_id.is_empty() {
        client.get_student_info(&id_token, &numeric_id).await?
    } else {
        None
    };
    let device_uid = info.as_ref().map(|s| s.device_uid.clone()).unwrap_or_default();
    let course = info.as_ref().map(|s| s.course.clone()).unwrap_or_default();
    let student_id = if !numeric_id.is_empty() {
        numeric_id
    } else {
        device_uid.chars().take(8).collect()
    };
    let module_list = if !student_id.is_empty() {
        client
            .get_student_modules(&id_token, &student_id, &current_academic_year())
            .await
            .unwrap_or_default()
    } else {
        vec![]
    };
    let modules: Vec<String> = module_list.iter().map(|m| m.module_id.clone()).collect();
    let module_info = crate::engine::instatt::modules_to_info_map(&module_list);

    Ok(ResolvedLogin {
        account_name,
        student_id,
        device_uid,
        course,
        azure_rt: azure_refresh.to_string(),
        firebase_rt,
        modules,
        module_info,
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
    let module_info_json: Option<serde_json::Value> = if login.modules.is_empty() {
        None
    } else {
        Some(login.module_info.clone())
    };
    let az_enc = crypto::encrypt_str(enc_key, &login.azure_rt)?;
    let fb_enc = crypto::encrypt_str(enc_key, &login.firebase_rt)?;

    let mut tx = pool.begin().await?;

    // `xmax = 0` 为真表示本次是 INSERT(全新注册);ON CONFLICT 触发的 UPDATE 其 xmax 非 0。
    let (id, role, inserted): (i64, String, bool) = sqlx::query_as(
        "INSERT INTO accounts
            (account_name, student_id, device_uid, azure_rt, firebase_rt, my_modules, enabled_modules, course, module_info, status, last_synced_at, last_refresh_at)
         VALUES ($1,$2,$3,$4,$5, COALESCE($6,'[]'::jsonb), COALESCE($6,'[]'::jsonb), $7, COALESCE($8,'{}'::jsonb), 'active', now(), now())
         ON CONFLICT (account_name) DO UPDATE SET
            student_id = CASE WHEN EXCLUDED.student_id <> '' THEN EXCLUDED.student_id ELSE accounts.student_id END,
            device_uid = COALESCE($3, accounts.device_uid),
            azure_rt = EXCLUDED.azure_rt,
            firebase_rt = EXCLUDED.firebase_rt,
            my_modules = COALESCE($6, accounts.my_modules),
            course = CASE WHEN EXCLUDED.course <> '' THEN EXCLUDED.course ELSE accounts.course END,
            module_info = COALESCE($8, accounts.module_info),
            status = 'active',
            last_synced_at = now(), last_refresh_at = now(), updated_at = now()
         RETURNING id, role, (xmax = 0) AS inserted",
    )
    .bind(&login.account_name)
    .bind(&login.student_id)
    .bind(dev_enc)
    .bind(az_enc)
    .bind(fb_enc)
    .bind(modules_json)
    .bind(&login.course)
    .bind(module_info_json)
    .fetch_one(&mut *tx)
    .await?;

    // 仅首次注册赠送余额,并记一条流水(同事务,保证余额与流水一致)。
    if inserted && REGISTER_BONUS_CENTS != 0 {
        let balance_after: i64 = sqlx::query_scalar(
            "UPDATE accounts SET balance_cents = balance_cents + $1, updated_at = now()
             WHERE id = $2 RETURNING balance_cents",
        )
        .bind(REGISTER_BONUS_CENTS)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO balance_transactions
                (account_id, amount_cents, type, balance_after, note, operator)
             VALUES ($1, $2, 'register_bonus', $3, '新用户注册赠送', 'system')",
        )
        .bind(id)
        .bind(REGISTER_BONUS_CENTS)
        .bind(balance_after)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
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
            course: "Computer Science".into(),
            azure_rt: "az".into(),
            firebase_rt: "fb".into(),
            modules: modules.iter().map(|s| s.to_string()).collect(),
            module_info: serde_json::Value::Object(
                modules
                    .iter()
                    .map(|s| (s.to_string(), serde_json::Value::String(format!("{s} 课程名"))))
                    .collect(),
            ),
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

    #[sqlx::test]
    async fn new_account_gets_register_bonus_once(pool: PgPool) {
        // 首次注册:应赠送 REGISTER_BONUS_CENTS 且恰好写一条 register_bonus 流水。
        let (id, _) = upsert_account(&pool, &KEY, &login(vec!["COMP4082"], "12345678abcd")).await.unwrap();
        let bal: i64 = sqlx::query_scalar("SELECT balance_cents FROM accounts WHERE id=$1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(bal, REGISTER_BONUS_CENTS, "新用户应获赠余额");
        let (cnt, sum, after): (i64, i64, i64) = sqlx::query_as(
            "SELECT count(*), COALESCE(SUM(amount_cents),0)::bigint, COALESCE(MAX(balance_after),0)::bigint
             FROM balance_transactions WHERE account_id=$1 AND type='register_bonus'",
        ).bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(cnt, 1, "恰好一条赠送流水");
        assert_eq!(sum, REGISTER_BONUS_CENTS);
        assert_eq!(after, REGISTER_BONUS_CENTS, "流水 balance_after 应为赠送后余额");

        // 再次登录(同账号 → UPDATE):不得重复赠送。
        upsert_account(&pool, &KEY, &login(vec!["COMP4082"], "12345678abcd")).await.unwrap();
        let bal2: i64 = sqlx::query_scalar("SELECT balance_cents FROM accounts WHERE id=$1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(bal2, REGISTER_BONUS_CENTS, "二次登录余额不变");
        let cnt2: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM balance_transactions WHERE account_id=$1 AND type='register_bonus'",
        ).bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(cnt2, 1, "二次登录不得重复赠送");
    }
}

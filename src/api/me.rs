//! 用户自身路由:资料、挂机开关、同步、记录、流水。

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use super::{paginate, ApiError, ApiResult};
use crate::auth::AuthCtx;
use crate::crypto;
use crate::models::{BalanceTx, SignRecord};
use crate::state::AppState;

/// 取当前登录账号 id;超管无个人账号时报错。
fn require_account(ctx: &AuthCtx) -> Result<i64, ApiError> {
    ctx.account_id
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "超管无个人签到账号"))
}

fn mask_student(sid: &str) -> String {
    let n = sid.chars().count();
    if n <= 4 {
        return "*".repeat(n);
    }
    let chars: Vec<char> = sid.chars().collect();
    let head: String = chars[..2].iter().collect();
    let tail: String = chars[n - 2..].iter().collect();
    format!("{head}{}{tail}", "*".repeat(n - 4))
}

pub async fn get_me(State(st): State<AppState>, ctx: AuthCtx) -> ApiResult {
    if ctx.account_id.is_none() {
        return Ok(Json(json!({"role": ctx.role, "account": null})));
    }
    let id = ctx.account_id.unwrap();
    let row: Option<(String, String, i64, bool, String, String, serde_json::Value, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            "SELECT account_name, student_id, balance_cents, auto_sign, role, status, my_modules, last_synced_at
             FROM accounts WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&st.pool)
        .await?;
    let (account_name, student_id, balance, auto_sign, role, status, modules, last_synced) =
        row.ok_or_else(|| ApiError::not_found("账号不存在"))?;
    Ok(Json(json!({
        "role": role,
        "account": {
            "id": id,
            "account_name": account_name,
            "student_id_masked": mask_student(&student_id),
            "balance_cents": balance,
            "auto_sign": auto_sign,
            "status": status,
            "modules": serde_json::from_value::<Vec<String>>(modules).unwrap_or_default(),
            "last_synced_at": last_synced,
        }
    })))
}

#[derive(Deserialize)]
pub struct AutoSignBody {
    pub auto_sign: bool,
}

pub async fn set_auto_sign(State(st): State<AppState>, ctx: AuthCtx, Json(b): Json<AutoSignBody>) -> ApiResult {
    let id = require_account(&ctx)?;
    sqlx::query("UPDATE accounts SET auto_sign=$1, updated_at=now() WHERE id=$2")
        .bind(b.auto_sign)
        .bind(id)
        .execute(&st.pool)
        .await?;
    Ok(Json(json!({"auto_sign": b.auto_sign})))
}

/// 手动同步:用有效 token 重新拉设备 UID 与课程,更新账号。
pub async fn sync(State(st): State<AppState>, ctx: AuthCtx) -> ApiResult {
    let id = require_account(&ctx)?;
    let student_id: String = sqlx::query_scalar("SELECT student_id FROM accounts WHERE id=$1")
        .bind(id)
        .fetch_optional(&st.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("账号不存在"))?;
    if student_id.is_empty() {
        return Err(ApiError::bad("缺少学号,请重新登录"));
    }
    let token = st
        .engine
        .tokens
        .get_valid_id_token(id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("token 失效,请重新登录: {e}")))?;

    // 设备 UID
    if let Ok(Some(info)) = st.client.get_student_info(&token, &student_id).await {
        if !info.device_uid.is_empty() {
            let enc = crypto::encrypt_str(&st.enc_key, &info.device_uid)?;
            sqlx::query("UPDATE accounts SET device_uid=$1 WHERE id=$2")
                .bind(enc)
                .bind(id)
                .execute(&st.pool)
                .await?;
        }
    }
    // 课程
    let modules: Vec<String> = st
        .client
        .get_student_modules(&token, &student_id, "25-26")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.module_id)
        .collect();
    if !modules.is_empty() {
        sqlx::query("UPDATE accounts SET my_modules=$1, last_synced_at=now(), updated_at=now() WHERE id=$2")
            .bind(serde_json::json!(modules))
            .bind(id)
            .execute(&st.pool)
            .await?;
    }
    Ok(Json(json!({"modules": modules})))
}

#[derive(Deserialize)]
pub struct PageQuery {
    pub page: Option<i64>,
}

pub async fn records(State(st): State<AppState>, ctx: AuthCtx, Query(q): Query<PageQuery>) -> ApiResult {
    let id = require_account(&ctx)?;
    let (limit, offset) = paginate(q.page);
    let rows: Vec<SignRecord> = sqlx::query_as(
        "SELECT * FROM sign_records WHERE account_id=$1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(json!({"records": rows, "page": q.page.unwrap_or(0)})))
}

pub async fn transactions(State(st): State<AppState>, ctx: AuthCtx, Query(q): Query<PageQuery>) -> ApiResult {
    let id = require_account(&ctx)?;
    let (limit, offset) = paginate(q.page);
    let rows: Vec<BalanceTx> = sqlx::query_as(
        "SELECT * FROM balance_transactions WHERE account_id=$1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(json!({"transactions": rows, "page": q.page.unwrap_or(0)})))
}

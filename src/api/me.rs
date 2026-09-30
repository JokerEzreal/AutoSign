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
use crate::engine::instatt;
use crate::models::{BalanceTx, SignRecord};
use crate::state::AppState;

/// 当前支持自动签到的教室(有 BSSID 的 + 免 WiFi 的)。任意登录用户可读。
pub async fn venues(_ctx: AuthCtx) -> ApiResult {
    let bssid: Vec<&str> = instatt::VENUE_BSSIDS.iter().map(|(v, _)| *v).collect();
    Ok(Json(json!({
        "bssid_venues": bssid,
        "ignore_wifi": instatt::IGNORE_WIFI_VENUES,
    })))
}

/// 取当前登录账号 id;超管无个人账号时报错。
fn require_account(ctx: &AuthCtx) -> Result<i64, ApiError> {
    ctx.account_id
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "超管无个人签到账号"))
}

pub async fn get_me(State(st): State<AppState>, ctx: AuthCtx) -> ApiResult {
    if ctx.account_id.is_none() {
        return Ok(Json(json!({"role": ctx.role, "account": null})));
    }
    let id = ctx.account_id.unwrap();
    let row: Option<(String, String, String, i64, bool, String, String, serde_json::Value, serde_json::Value, serde_json::Value, Option<chrono::DateTime<chrono::Utc>>)> =
        sqlx::query_as(
            "SELECT account_name, student_id, course, balance_cents, auto_sign, role, status, my_modules, enabled_modules, module_info, last_synced_at
             FROM accounts WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&st.pool)
        .await?;
    let (account_name, student_id, course, balance, auto_sign, role, status, modules, enabled, module_info, last_synced) =
        row.ok_or_else(|| ApiError::not_found("账号不存在"))?;
    Ok(Json(json!({
        "role": role,
        "account": {
            "id": id,
            "account_name": account_name,
            "student_id": student_id,
            "course": course,
            "balance_cents": balance,
            "auto_sign": auto_sign,
            "status": status,
            "modules": serde_json::from_value::<Vec<String>>(modules).unwrap_or_default(),
            "enabled_modules": serde_json::from_value::<Vec<String>>(enabled).unwrap_or_default(),
            "module_info": module_info,
            "last_synced_at": last_synced,
        }
    })))
}

#[derive(Deserialize)]
pub struct ModulesBody {
    pub enabled: Vec<String>,
}

/// 设置「已启用自动签」的课程(必须是 my_modules 子集)。
pub async fn set_modules(State(st): State<AppState>, ctx: AuthCtx, Json(b): Json<ModulesBody>) -> ApiResult {
    let id = require_account(&ctx)?;
    let my: serde_json::Value = sqlx::query_scalar("SELECT my_modules FROM accounts WHERE id=$1")
        .bind(id)
        .fetch_optional(&st.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("账号不存在"))?;
    let my: Vec<String> = serde_json::from_value(my).unwrap_or_default();
    let enabled: Vec<String> = b.enabled.into_iter().filter(|m| my.contains(m)).collect();
    sqlx::query("UPDATE accounts SET enabled_modules=$1, updated_at=now() WHERE id=$2")
        .bind(serde_json::json!(enabled))
        .bind(id)
        .execute(&st.pool)
        .await?;
    Ok(Json(json!({"enabled_modules": enabled})))
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

    // 设备 UID + 专业
    if let Ok(Some(info)) = st.client.get_student_info(&token, &student_id).await {
        if !info.device_uid.is_empty() {
            let enc = crypto::encrypt_str(&st.enc_key, &info.device_uid)?;
            sqlx::query("UPDATE accounts SET device_uid=$1 WHERE id=$2")
                .bind(enc)
                .bind(id)
                .execute(&st.pool)
                .await?;
        }
        if !info.course.is_empty() {
            sqlx::query("UPDATE accounts SET course=$1 WHERE id=$2")
                .bind(&info.course)
                .bind(id)
                .execute(&st.pool)
                .await?;
        }
    }
    // 课程(保留完整信息以构建 module_info)
    let module_list = st
        .client
        .get_student_modules(&token, &student_id, &instatt::current_academic_year())
        .await
        .unwrap_or_default();
    let modules: Vec<String> = module_list.iter().map(|m| m.module_id.clone()).collect();
    if !modules.is_empty() {
        let module_info = crate::engine::instatt::modules_to_info_map(&module_list);
        // 已有的启用选择 + 旧课表,用于计算新启用集
        let (old_my, old_enabled): (serde_json::Value, serde_json::Value) =
            sqlx::query_as("SELECT my_modules, enabled_modules FROM accounts WHERE id=$1")
                .bind(id)
                .fetch_one(&st.pool)
                .await?;
        let old_my: Vec<String> = serde_json::from_value(old_my).unwrap_or_default();
        let old_enabled: Vec<String> = serde_json::from_value(old_enabled).unwrap_or_default();
        // 仍在课表里的旧启用项 + 新增课程(默认开启)
        let enabled: Vec<String> = modules
            .iter()
            .filter(|m| old_enabled.contains(m) || !old_my.contains(m))
            .cloned()
            .collect();
        sqlx::query("UPDATE accounts SET my_modules=$1, enabled_modules=$2, module_info=$3, last_synced_at=now(), updated_at=now() WHERE id=$4")
            .bind(serde_json::json!(modules))
            .bind(serde_json::json!(enabled))
            .bind(module_info)
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

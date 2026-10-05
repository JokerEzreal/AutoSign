//! 管理员 / 超管路由。

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

use super::{paginate, ApiError, ApiResult};
use crate::auth::middleware::{AdminCtx, SuperAdminCtx};
use crate::auth::password;
use crate::models::SignRecord;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListQuery {
    pub search: Option<String>,
    pub page: Option<i64>,
}

pub async fn list_accounts(State(st): State<AppState>, _admin: AdminCtx, Query(q): Query<ListQuery>) -> ApiResult {
    let (limit, offset) = paginate(q.page);
    let pattern = format!("%{}%", q.search.clone().unwrap_or_default());
    let rows: Vec<(i64, String, String, i64, bool, String, String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as(
            "SELECT id, account_name, student_id, balance_cents, auto_sign, role, status, created_at
             FROM accounts WHERE account_name ILIKE $1
             ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(&pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(&st.pool)
        .await?;
    let accounts: Vec<_> = rows
        .into_iter()
        .map(|(id, name, sid, bal, auto, role, status, created)| {
            json!({
                "id": id, "account_name": name, "student_id": sid,
                "balance_cents": bal, "auto_sign": auto, "role": role,
                "status": status, "created_at": created,
            })
        })
        .collect();
    Ok(Json(json!({"accounts": accounts, "page": q.page.unwrap_or(0)})))
}

#[derive(Deserialize)]
pub struct TopupBody {
    pub amount_cents: i64,
    #[serde(default)]
    pub note: String,
}

/// 手动充值:事务内加余额并写流水。
pub async fn topup(State(st): State<AppState>, admin: AdminCtx, Path(id): Path<i64>, Json(b): Json<TopupBody>) -> ApiResult {
    if b.amount_cents == 0 {
        return Err(ApiError::bad("充值金额不能为 0"));
    }
    let mut tx = st.pool.begin().await?;
    let balance_after: Option<i64> = sqlx::query_scalar(
        "UPDATE accounts SET balance_cents = balance_cents + $1, updated_at=now()
         WHERE id=$2 RETURNING balance_cents",
    )
    .bind(b.amount_cents)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let balance_after = balance_after.ok_or_else(|| ApiError::not_found("账号不存在"))?;
    sqlx::query(
        "INSERT INTO balance_transactions (account_id, amount_cents, type, balance_after, note, operator)
         VALUES ($1,$2,'topup',$3,$4,$5)",
    )
    .bind(id)
    .bind(b.amount_cents)
    .bind(balance_after)
    .bind(&b.note)
    .bind(&admin.0.subject)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"balance_cents": balance_after})))
}

#[derive(Deserialize)]
pub struct StatusBody {
    pub status: String,
}

pub async fn set_status(State(st): State<AppState>, _admin: AdminCtx, Path(id): Path<i64>, Json(b): Json<StatusBody>) -> ApiResult {
    if !matches!(b.status.as_str(), "active" | "disabled") {
        return Err(ApiError::bad("status 仅支持 active/disabled"));
    }
    let n = sqlx::query("UPDATE accounts SET status=$1, updated_at=now() WHERE id=$2")
        .bind(&b.status)
        .bind(id)
        .execute(&st.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("账号不存在"));
    }
    Ok(Json(json!({"status": b.status})))
}

#[derive(Deserialize)]
pub struct PageQuery {
    pub page: Option<i64>,
}

pub async fn account_records(State(st): State<AppState>, _admin: AdminCtx, Path(id): Path<i64>, Query(q): Query<PageQuery>) -> ApiResult {
    let (limit, offset) = paginate(q.page);
    let rows: Vec<SignRecord> = sqlx::query_as(
        "SELECT * FROM sign_records WHERE account_id=$1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&st.pool)
    .await?;
    Ok(Json(json!({"records": rows})))
}

pub async fn stats(State(st): State<AppState>, _admin: AdminCtx) -> ApiResult {
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts").fetch_one(&st.pool).await?;
    let active: i64 = sqlx::query_scalar("SELECT count(*) FROM accounts WHERE status='active'").fetch_one(&st.pool).await?;
    let today_signs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sign_records WHERE result='ok_200' AND created_at::date = now()::date",
    )
    .fetch_one(&st.pool)
    .await?;
    let today_revenue: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(charged_cents),0)::bigint FROM sign_records WHERE created_at::date = now()::date",
    )
    .fetch_one(&st.pool)
    .await?;
    Ok(Json(json!({
        "total_accounts": total,
        "active_accounts": active,
        "today_signs": today_signs,
        "today_revenue_cents": today_revenue,
    })))
}

/// 管理员查看某账号详情:资料 + 课程清单(含课程名)+ 已启用自动签的课程。
pub async fn account_detail(State(st): State<AppState>, _admin: AdminCtx, Path(id): Path<i64>) -> ApiResult {
    let row: Option<(
        String,
        String,
        String,
        i64,
        bool,
        String,
        String,
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT account_name, student_id, course, balance_cents, auto_sign, role, status,
                my_modules, enabled_modules, module_info, last_synced_at
         FROM accounts WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(&st.pool)
    .await?;
    let (
        account_name,
        student_id,
        course,
        balance_cents,
        auto_sign,
        role,
        status,
        my_modules,
        enabled_modules,
        module_info,
        last_synced_at,
    ) = row.ok_or_else(|| ApiError::not_found("账号不存在"))?;
    Ok(Json(json!({
        "id": id,
        "account_name": account_name,
        "student_id": student_id,
        "course": course,
        "balance_cents": balance_cents,
        "auto_sign": auto_sign,
        "role": role,
        "status": status,
        "modules": serde_json::from_value::<Vec<String>>(my_modules).unwrap_or_default(),
        "enabled_modules": serde_json::from_value::<Vec<String>>(enabled_modules).unwrap_or_default(),
        "module_info": module_info,
        "last_synced_at": last_synced_at,
    })))
}

/// 管理员查看某账号某天课表(课程 + 对应教室 + 解锁/签到状态)。复用 /api/me/classes 的构建逻辑。
pub async fn account_classes(
    State(st): State<AppState>,
    _admin: AdminCtx,
    Path(id): Path<i64>,
    Query(q): Query<crate::api::classes::ClassesQuery>,
) -> ApiResult {
    Ok(Json(crate::api::classes::build_classes(&st, id, q.date).await?))
}

#[derive(Deserialize)]
pub struct ModulesBody {
    pub enabled: Vec<String>,
}

/// 管理员设置某账号「已启用自动签」的课程(必须是该账号 my_modules 的子集)。
pub async fn set_account_modules(
    State(st): State<AppState>,
    _admin: AdminCtx,
    Path(id): Path<i64>,
    Json(b): Json<ModulesBody>,
) -> ApiResult {
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

/// 管理员设置某账号的挂机总开关。
pub async fn set_account_auto_sign(
    State(st): State<AppState>,
    _admin: AdminCtx,
    Path(id): Path<i64>,
    Json(b): Json<AutoSignBody>,
) -> ApiResult {
    let n = sqlx::query("UPDATE accounts SET auto_sign=$1, updated_at=now() WHERE id=$2")
        .bind(b.auto_sign)
        .bind(id)
        .execute(&st.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("账号不存在"));
    }
    Ok(Json(json!({"auto_sign": b.auto_sign})))
}

// ---------- 仅超管 ----------

#[derive(Deserialize)]
pub struct RoleBody {
    pub role: String,
}

pub async fn set_role(State(st): State<AppState>, _su: SuperAdminCtx, Path(id): Path<i64>, Json(b): Json<RoleBody>) -> ApiResult {
    if !matches!(b.role.as_str(), "user" | "admin") {
        return Err(ApiError::bad("role 仅支持 user/admin"));
    }
    let n = sqlx::query("UPDATE accounts SET role=$1, updated_at=now() WHERE id=$2")
        .bind(&b.role)
        .bind(id)
        .execute(&st.pool)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(ApiError::not_found("账号不存在"));
    }
    Ok(Json(json!({"role": b.role})))
}

/// 读取全部 system_config(管理员可读)。
pub async fn get_config(State(st): State<AppState>, _admin: AdminCtx) -> ApiResult {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM system_config")
        .fetch_all(&st.pool)
        .await?;
    let config: HashMap<String, String> = rows.into_iter().collect();
    Ok(Json(json!({ "config": config })))
}

/// 更新 system_config(注:引擎参数在重启后生效)。
pub async fn set_config(State(st): State<AppState>, _su: SuperAdminCtx, Json(map): Json<HashMap<String, String>>) -> ApiResult {
    for (k, v) in &map {
        sqlx::query(
            "INSERT INTO system_config (key, value, updated_at) VALUES ($1,$2,now())
             ON CONFLICT (key) DO UPDATE SET value=EXCLUDED.value, updated_at=now()",
        )
        .bind(k)
        .bind(v)
        .execute(&st.pool)
        .await?;
    }
    Ok(Json(json!({"updated": map.len(), "note": "引擎参数(轮询/并发/单价)在服务重启后生效"})))
}

#[derive(Deserialize)]
pub struct PasswordBody {
    pub old: String,
    pub new: String,
}

pub async fn change_password(State(st): State<AppState>, su: SuperAdminCtx, Json(b): Json<PasswordBody>) -> ApiResult {
    let username = su.0.subject.strip_prefix("super:").unwrap_or(&su.0.subject).to_string();
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM super_admin WHERE username=$1")
        .bind(&username)
        .fetch_optional(&st.pool)
        .await?;
    let hash = hash.ok_or_else(|| ApiError::not_found("超管不存在"))?;
    if !password::verify(&b.old, &hash) {
        return Err(ApiError::bad("原密码错误"));
    }
    if b.new.len() < 6 {
        return Err(ApiError::bad("新密码至少 6 位"));
    }
    let new_hash = password::hash(&b.new)?;
    sqlx::query("UPDATE super_admin SET password_hash=$1 WHERE username=$2")
        .bind(&new_hash)
        .bind(&username)
        .execute(&st.pool)
        .await?;
    Ok(Json(json!({"ok": true})))
}

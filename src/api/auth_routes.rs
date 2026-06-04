//! 认证路由:Device Code 登录、超管密码登录、登出。

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use super::{clear_session_cookie, with_session_cookie, ApiError, ApiResult};
use crate::auth::{device, jwt, password};
use crate::state::AppState;

/// 发起 Device Code。
pub async fn device_start(State(st): State<AppState>) -> ApiResult {
    let r = device::start_device_login(&st.pool, &st.client)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("发起登录失败: {e}")))?;
    Ok(Json(json!({
        "session_id": r.session_id,
        "user_code": r.user_code,
        "verification_uri": r.verification_uri,
        "interval": r.interval,
        "expires_in": r.expires_in,
    })))
}

#[derive(Deserialize)]
pub struct PollQuery {
    pub id: i64,
}

/// 轮询 Device Code;成功则签发 session cookie。
pub async fn device_poll(State(st): State<AppState>, Query(q): Query<PollQuery>) -> Result<Response, ApiError> {
    let r = device::poll_device_login(&st.pool, &st.client, &st.enc_key, q.id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("轮询失败: {e}")))?;
    match r {
        device::PollResult::Pending => Ok(Json(json!({"status": "pending"})).into_response()),
        device::PollResult::Declined => Ok(Json(json!({"status": "declined"})).into_response()),
        device::PollResult::Expired => Ok(Json(json!({"status": "expired"})).into_response()),
        device::PollResult::Done { account_id, role } => {
            let token = jwt::issue(&st.jwt_secret, &account_id.to_string(), &role, Utc::now().timestamp())?;
            Ok(with_session_cookie(&token, st.cookie_secure, json!({"status": "done"})))
        }
    }
}

#[derive(Deserialize)]
pub struct AdminLogin {
    pub username: String,
    pub password: String,
}

/// 超管账号密码登录。
pub async fn admin_login(State(st): State<AppState>, Json(body): Json<AdminLogin>) -> Result<Response, ApiError> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT password_hash, role FROM super_admin WHERE username=$1")
            .bind(&body.username)
            .fetch_optional(&st.pool)
            .await?;
    let (hash, role) = row.ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "账号或密码错误"))?;
    if !password::verify(&body.password, &hash) {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "账号或密码错误"));
    }
    let sub = format!("super:{}", body.username);
    let token = jwt::issue(&st.jwt_secret, &sub, &role, Utc::now().timestamp())?;
    Ok(with_session_cookie(&token, st.cookie_secure, json!({"status": "ok", "role": role})))
}

pub async fn logout(State(st): State<AppState>) -> Response {
    clear_session_cookie(st.cookie_secure)
}

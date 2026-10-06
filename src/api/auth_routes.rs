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
use crate::auth::assisted::{self, AssistedState};
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
pub struct PasswordLogin {
    pub username: String,
    pub password: String,
}

/// 账号密码直登(ROPC):用户填学校账号 + 密码,后台静默换 token 并建/更新账号,直接下发 session。
/// 账号可只填前缀(如 `niubi666`),自动补 `@nottingham.edu.my`。
pub async fn password_login(State(st): State<AppState>, Json(body): Json<PasswordLogin>) -> Result<Response, ApiError> {
    let raw = body.username.trim();
    if raw.is_empty() || body.password.is_empty() {
        return Err(ApiError::bad("账号和密码不能为空"));
    }
    let username = if raw.contains('@') {
        raw.to_string()
    } else {
        format!("{raw}@nottingham.edu.my")
    };
    let (account_id, role) = device::password_login(&st.pool, &st.client, &st.enc_key, &username, &body.password)
        .await
        .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, format!("登录失败: {e}")))?;
    let token = jwt::issue(&st.jwt_secret, &account_id.to_string(), &role, Utc::now().timestamp())?;
    Ok(with_session_cookie(&token, st.cookie_secure, json!({"status": "done", "role": role})))
}

#[derive(Deserialize)]
pub struct AssistedStart {
    pub username: String,
    #[serde(default)]
    pub password: String,
}

/// 辅助登录(授权码 + MFA 数字匹配,后台无头浏览器代登):发起一次登录,返回 session_id。
/// 随后前端轮询 /api/auth/assisted/poll 拿 MFA 数字并完成登录。
pub async fn assisted_start(State(st): State<AppState>, Json(b): Json<AssistedStart>) -> ApiResult {
    let uname = b.username.trim();
    if uname.is_empty() {
        return Err(ApiError::bad("请输入账号"));
    }
    let sid = assisted::gen_session_id();
    assisted::start_worker(&sid, uname, &b.password)
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    Ok(Json(json!({ "session_id": sid })))
}

#[derive(Deserialize)]
pub struct AssistedPoll {
    pub id: String,
}

/// 轮询辅助登录状态。mfa:返回待输入的数字匹配号码;done:签发 session cookie;failed:返回原因。
pub async fn assisted_poll(State(st): State<AppState>, Query(q): Query<AssistedPoll>) -> Result<Response, ApiError> {
    match assisted::read_status(&st.enc_key, &q.id) {
        AssistedState::Pending | AssistedState::NotFound => {
            Ok(Json(json!({"status": "pending"})).into_response())
        }
        AssistedState::Mfa { number } => {
            Ok(Json(json!({"status": "mfa", "number": number})).into_response())
        }
        AssistedState::Failed { error } => {
            assisted::cleanup(&q.id);
            Ok(Json(json!({"status": "failed", "error": error})).into_response())
        }
        AssistedState::Success { azure_access, azure_refresh } => {
            let (account_id, role) =
                device::finalize_from_azure_tokens(&st.pool, &st.client, &st.enc_key, &azure_access, &azure_refresh)
                    .await
                    .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("完成登录失败: {e}")))?;
            assisted::cleanup(&q.id);
            let token = jwt::issue(&st.jwt_secret, &account_id.to_string(), &role, Utc::now().timestamp())?;
            Ok(with_session_cookie(&token, st.cookie_secure, json!({"status": "done", "role": role})))
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

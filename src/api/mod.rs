//! HTTP API:路由装配、统一错误、cookie 辅助。

pub mod admin;
pub mod auth_routes;
pub mod me;

use axum::{
    http::{header::SET_COOKIE, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde_json::{json, Value};

use crate::state::AppState;

const SESSION_COOKIE: &str = "session";
const SESSION_MAX_AGE: i64 = 30 * 24 * 3600;

/// 统一 API 错误 → JSON。
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn new(code: StatusCode, msg: impl Into<String>) -> Self {
        ApiError(code, msg.into())
    }
    pub fn bad(msg: impl Into<String>) -> Self {
        ApiError(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        ApiError(StatusCode::NOT_FOUND, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}"))
    }
}
impl From<crate::crypto::CryptoError> for ApiError {
    fn from(e: crate::crypto::CryptoError) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("crypto: {e}"))
    }
}

pub type ApiResult = Result<Json<Value>, ApiError>;

/// 构造带 session cookie 的响应。
pub fn with_session_cookie(token: &str, secure: bool, body: Value) -> Response {
    let cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_MAX_AGE}{}",
        if secure { "; Secure" } else { "" }
    );
    ([(SET_COOKIE, cookie)], Json(body)).into_response()
}

/// 清除 session cookie。
pub fn clear_session_cookie(secure: bool) -> Response {
    let cookie = format!(
        "{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    );
    ([(SET_COOKIE, cookie)], Json(json!({"ok": true}))).into_response()
}

/// 分页参数 → (limit, offset)。page 从 0 起,固定 20 条/页。
pub fn paginate(page: Option<i64>) -> (i64, i64) {
    let page = page.unwrap_or(0).max(0);
    (20, page * 20)
}

/// 全部路由。
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        // 认证
        .route("/api/auth/device/start", post(auth_routes::device_start))
        .route("/api/auth/device/poll", get(auth_routes::device_poll))
        .route("/api/auth/admin/login", post(auth_routes::admin_login))
        .route("/api/auth/logout", post(auth_routes::logout))
        // 用户自己
        .route("/api/me", get(me::get_me))
        .route("/api/me/auto-sign", patch(me::set_auto_sign))
        .route("/api/me/sync", post(me::sync))
        .route("/api/me/records", get(me::records))
        .route("/api/me/transactions", get(me::transactions))
        // 管理员
        .route("/api/admin/accounts", get(admin::list_accounts))
        .route("/api/admin/accounts/:id/topup", post(admin::topup))
        .route("/api/admin/accounts/:id/status", patch(admin::set_status))
        .route("/api/admin/accounts/:id/records", get(admin::account_records))
        .route("/api/admin/stats", get(admin::stats))
        // 仅超管
        .route("/api/admin/accounts/:id/role", patch(admin::set_role))
        .route("/api/admin/config", patch(admin::set_config))
        .route("/api/admin/superadmin/password", post(admin::change_password))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

//! 共享应用状态(注入到所有 handler)。

use std::sync::Arc;

use sqlx::PgPool;

use crate::engine::instatt::InstAttClient;
use crate::engine::worker::Engine;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub jwt_secret: String,
    pub enc_key: [u8; 32],
    pub client: InstAttClient,
    pub engine: Arc<Engine>,
    pub superadmin_username: String,
    /// 生产为 true(https),本地测试为 false。控制 cookie 的 Secure 属性。
    pub cookie_secure: bool,
}

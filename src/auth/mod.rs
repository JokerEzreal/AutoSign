//! 认证:JWT 会话、argon2 密码、Device Code 登录、角色中间件。

pub mod device;
pub mod jwt;
pub mod middleware;
pub mod password;

/// 已认证主体(从 JWT 解析)。
#[derive(Debug, Clone)]
pub struct AuthCtx {
    /// 账号主体:学校账号为其 account_id 字符串;超管为 "super:<username>"。
    pub subject: String,
    pub role: String,
    /// 学校账号 id(超管为 None)。
    pub account_id: Option<i64>,
}

impl AuthCtx {
    pub fn is_admin(&self) -> bool {
        self.role == "admin" || self.role == "superadmin"
    }
    pub fn is_superadmin(&self) -> bool {
        self.role == "superadmin"
    }
}

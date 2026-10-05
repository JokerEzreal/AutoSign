//! 从 session cookie 解析 AuthCtx 的 axum 提取器,及角色守卫包装。

use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};

use super::{jwt, AuthCtx};
use crate::state::AppState;

/// 从 Cookie 头取指定 cookie 值。
pub fn parse_cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|kv| {
        let kv = kv.trim();
        let (k, v) = kv.split_once('=')?;
        if k == name {
            Some(v)
        } else {
            None
        }
    })
}

/// 校验 token 并构建 AuthCtx。
pub fn ctx_from_token(secret: &str, token: &str) -> Option<AuthCtx> {
    let c = jwt::verify(secret, token).ok()?;
    let account_id = if c.sub.starts_with("super:") {
        None
    } else {
        c.sub.parse().ok()
    };
    Some(AuthCtx {
        subject: c.sub,
        role: c.role,
        account_id,
    })
}

/// 任意登录用户。
#[async_trait::async_trait]
impl FromRequestParts<AppState> for AuthCtx {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, StatusCode> {
        let cookie = parts
            .headers
            .get("cookie")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let token = parse_cookie(cookie, "session").ok_or(StatusCode::UNAUTHORIZED)?;
        let mut ctx = ctx_from_token(&state.jwt_secret, token).ok_or(StatusCode::UNAUTHORIZED)?;
        // 角色以数据库当前值为准:JWT 里的角色是登录那一刻写死的,登录后被改过就会过期。
        // 这样提权/降权立即生效,且与 /api/me 返回的角色一致(修复「提成 admin 后仍看不到账号」)。
        if let Some(aid) = ctx.account_id {
            if let Ok(Some(role)) =
                sqlx::query_scalar::<_, String>("SELECT role FROM accounts WHERE id=$1")
                    .bind(aid)
                    .fetch_optional(&state.pool)
                    .await
            {
                ctx.role = role;
            }
        }
        Ok(ctx)
    }
}

/// 仅管理员/超管。
pub struct AdminCtx(pub AuthCtx);

#[async_trait::async_trait]
impl FromRequestParts<AppState> for AdminCtx {
    type Rejection = StatusCode;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, StatusCode> {
        let ctx = AuthCtx::from_request_parts(parts, state).await?;
        if ctx.is_admin() {
            Ok(AdminCtx(ctx))
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }
}

/// 仅超管。
pub struct SuperAdminCtx(pub AuthCtx);

#[async_trait::async_trait]
impl FromRequestParts<AppState> for SuperAdminCtx {
    type Rejection = StatusCode;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, StatusCode> {
        let ctx = AuthCtx::from_request_parts(parts, state).await?;
        if ctx.is_superadmin() {
            Ok(SuperAdminCtx(ctx))
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::jwt;
    use chrono::Utc;

    #[test]
    fn parse_cookie_picks_right_value() {
        let h = "foo=bar; session=abc.def.ghi; other=1";
        assert_eq!(parse_cookie(h, "session"), Some("abc.def.ghi"));
        assert_eq!(parse_cookie(h, "missing"), None);
    }

    #[test]
    fn ctx_user_vs_super() {
        let secret = "s";
        let now = Utc::now().timestamp();
        let user_tok = jwt::issue(secret, "42", "user", now).unwrap();
        let ctx = ctx_from_token(secret, &user_tok).unwrap();
        assert_eq!(ctx.account_id, Some(42));
        assert!(!ctx.is_admin());

        let super_tok = jwt::issue(secret, "super:admin", "superadmin", now).unwrap();
        let sctx = ctx_from_token(secret, &super_tok).unwrap();
        assert_eq!(sctx.account_id, None);
        assert!(sctx.is_superadmin() && sctx.is_admin());
    }
}

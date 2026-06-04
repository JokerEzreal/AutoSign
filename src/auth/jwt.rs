//! JWT 会话令牌(HS256),默认 30 天有效。

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

const TTL_SECONDS: i64 = 30 * 24 * 3600; // 30 天

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub exp: i64,
}

/// 签发 token。`now_ts` 为当前 Unix 秒(便于测试注入)。
pub fn issue(secret: &str, sub: &str, role: &str, now_ts: i64) -> anyhow::Result<String> {
    let claims = Claims {
        sub: sub.to_string(),
        role: role.to_string(),
        exp: now_ts + TTL_SECONDS,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;
    Ok(token)
}

/// 校验并解析 token(过期会报错)。
pub fn verify(secret: &str, token: &str) -> anyhow::Result<Claims> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn round_trip() {
        let now = Utc::now().timestamp();
        let t = issue("secret", "42", "user", now).unwrap();
        let c = verify("secret", &t).unwrap();
        assert_eq!(c.sub, "42");
        assert_eq!(c.role, "user");
        assert_eq!(c.exp, now + TTL_SECONDS);
    }

    #[test]
    fn wrong_secret_fails() {
        let t = issue("secret", "42", "user", Utc::now().timestamp()).unwrap();
        assert!(verify("other", &t).is_err());
    }

    #[test]
    fn expired_fails() {
        // 颁发于很久以前(exp 已过去)→ 已过期
        let long_ago = Utc::now().timestamp() - (TTL_SECONDS + 10_000);
        let t = issue("secret", "42", "user", long_ago).unwrap();
        assert!(verify("secret", &t).is_err());
    }
}

//! 运行期配置:全部来自环境变量。缺失或非法即 panic 并给出明确提示。

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub jwt_secret: String,
    /// AES-256-GCM 密钥,必须正好 32 字节。
    pub encryption_key: [u8; 32],
    pub superadmin_username: String,
    pub superadmin_password: String,
    pub bind_addr: String,
}

impl Config {
    /// 从进程环境变量加载。任何必填项缺失/非法都会 panic(启动期 fail-fast)。
    pub fn from_env() -> Self {
        fn req(key: &str) -> String {
            std::env::var(key).unwrap_or_else(|_| panic!("缺少必填环境变量 {key}"))
        }

        let enc = req("ENCRYPTION_KEY");
        let enc_bytes = enc.as_bytes();
        if enc_bytes.len() != 32 {
            panic!(
                "ENCRYPTION_KEY 必须正好 32 字节,当前为 {} 字节",
                enc_bytes.len()
            );
        }
        let mut encryption_key = [0u8; 32];
        encryption_key.copy_from_slice(enc_bytes);

        Config {
            database_url: req("DATABASE_URL"),
            jwt_secret: req("JWT_SECRET"),
            encryption_key,
            superadmin_username: req("SUPERADMIN_USERNAME"),
            superadmin_password: req("SUPERADMIN_PASSWORD"),
            bind_addr: std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
        }
    }
}

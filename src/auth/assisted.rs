//! 辅助登录:调用 Python 无头浏览器 worker 走 Azure 授权码登录(账号密码 + MFA 数字匹配),
//! 用 App 自带的 https://localhost 回调即时截获授权码,绕开被租户封禁的设备码流程(7000218)。
//!
//! worker 把换到的 Azure access/refresh token 用服务器 ENCRYPTION_KEY 加密后写状态文件;
//! 本模块读取、解密,交回 device::finalize_from_azure_tokens 走原有建号链路。

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const PY: &str = "/opt/pwlogin/bin/python";
const WORKER: &str = "/opt/instatt_saas/tools/assisted_login.py";
const STATUS_DIR: &str = "/tmp/assisted";
const MAX_CONCURRENT: usize = 5;

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// 生成 32 位十六进制 session id。
pub fn gen_session_id() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn status_path(session_id: &str) -> Option<PathBuf> {
    if session_id.len() < 8
        || session_id.len() > 64
        || !session_id.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(PathBuf::from(STATUS_DIR).join(format!("{session_id}.json")))
}

/// 启动一个登录 worker。账号密码通过 stdin 传入(不进 argv/进程表)。
pub async fn start_worker(session_id: &str, username: &str, password: &str) -> anyhow::Result<()> {
    if ACTIVE.load(Ordering::SeqCst) >= MAX_CONCURRENT {
        anyhow::bail!("登录并发已满,请稍后再试");
    }
    let payload = serde_json::json!({
        "session_id": session_id, "username": username, "password": password
    })
    .to_string();
    let mut child = Command::new(PY)
        .arg(WORKER)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(payload.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        let _ = stdin.shutdown().await;
    }
    ACTIVE.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        let _ = child.wait().await;
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    });
    Ok(())
}

#[derive(Debug)]
pub enum AssistedState {
    Pending,
    Mfa { number: Option<String> },
    Success { azure_access: String, azure_refresh: String },
    Failed { error: String },
    NotFound,
}

/// 读取 worker 状态(成功时解密 azure token)。
pub fn read_status(enc_key: &[u8; 32], session_id: &str) -> AssistedState {
    let path = match status_path(session_id) {
        Some(p) => p,
        None => return AssistedState::NotFound,
    };
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return AssistedState::NotFound,
    };
    let v: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => return AssistedState::Pending, // 可能正写入中
    };
    match v["stage"].as_str().unwrap_or("") {
        "success" => {
            let dec = |k: &str| -> Option<String> {
                let bytes = hex_decode(v[k].as_str()?)?;
                crate::crypto::decrypt_str(enc_key, &bytes).ok()
            };
            match (dec("az_access_hex"), dec("az_refresh_hex")) {
                (Some(a), Some(r)) => AssistedState::Success {
                    azure_access: a,
                    azure_refresh: r,
                },
                _ => AssistedState::Failed {
                    error: "令牌解密失败".into(),
                },
            }
        }
        "mfa" => AssistedState::Mfa {
            number: v["mfa_number"].as_str().map(|s| s.to_string()),
        },
        "failed" => AssistedState::Failed {
            error: v["error"].as_str().unwrap_or("登录失败").to_string(),
        },
        _ => AssistedState::Pending,
    }
}

/// 删除状态文件。
pub fn cleanup(session_id: &str) {
    if let Some(p) = status_path(session_id) {
        let _ = std::fs::remove_file(p);
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let b = s.as_bytes();
    let hv = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < b.len() {
        out.push((hv(b[i])? << 4) | hv(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_roundtrip() {
        assert_eq!(hex_decode("00ff10"), Some(vec![0u8, 255, 16]));
        assert_eq!(hex_decode("0"), None);
        assert_eq!(hex_decode("zz"), None);
    }
    #[test]
    fn session_id_is_hex_32() {
        let s = gen_session_id();
        assert_eq!(s.len(), 32);
        assert!(status_path(&s).is_some());
        assert!(status_path("../etc/passwd").is_none());
        assert!(status_path("short").is_none());
    }
}

//! 字段级加密:AES-256-GCM。密文格式 = 12 字节随机 nonce ‖ ciphertext(含 tag)。

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;

#[derive(thiserror::Error, Debug)]
pub enum CryptoError {
    #[error("加密失败")]
    Encrypt,
    #[error("解密失败")]
    Decrypt,
    #[error("密文过短")]
    TooShort,
}

/// 用 32 字节密钥加密明文,返回 nonce(12B) ‖ 密文。
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::Encrypt)?;
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// 解密 encrypt() 产出的数据。
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if data.len() < 12 {
        return Err(CryptoError::TooShort);
    }
    let (nonce_bytes, ct) = data.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, ct).map_err(|_| CryptoError::Decrypt)
}

/// 加密字符串便捷封装(明文为空时返回空 Vec,表示「无」)。
pub fn encrypt_str(key: &[u8; 32], s: &str) -> Result<Vec<u8>, CryptoError> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    encrypt(key, s.as_bytes())
}

/// 解密为字符串;空输入返回空串。
pub fn decrypt_str(key: &[u8; 32], data: &[u8]) -> Result<String, CryptoError> {
    if data.is_empty() {
        return Ok(String::new());
    }
    let pt = decrypt(key, data)?;
    String::from_utf8(pt).map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        *b"0123456789abcdef0123456789abcdef"
    }

    #[test]
    fn round_trip() {
        let k = key();
        let secret = "1//refresh-token-éñ-数据";
        let enc = encrypt_str(&k, secret).unwrap();
        assert_ne!(enc.as_slice(), secret.as_bytes());
        let dec = decrypt_str(&k, &enc).unwrap();
        assert_eq!(dec, secret);
    }

    #[test]
    fn nonce_makes_ciphertext_unique() {
        let k = key();
        let a = encrypt_str(&k, "same").unwrap();
        let b = encrypt_str(&k, "same").unwrap();
        assert_ne!(a, b, "随机 nonce 应让同一明文每次密文不同");
    }

    #[test]
    fn wrong_key_fails() {
        let enc = encrypt_str(&key(), "secret").unwrap();
        let bad = *b"ffffffffffffffffffffffffffffffff";
        assert!(decrypt_str(&bad, &enc).is_err());
    }

    #[test]
    fn empty_is_empty() {
        assert!(encrypt_str(&key(), "").unwrap().is_empty());
        assert_eq!(decrypt_str(&key(), &[]).unwrap(), "");
    }
}

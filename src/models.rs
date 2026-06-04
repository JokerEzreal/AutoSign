//! 数据库行模型(sqlx::FromRow)。

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Account {
    pub id: i64,
    pub account_name: String,
    pub student_id: String,
    pub device_uid: Option<Vec<u8>>,
    pub azure_rt: Option<Vec<u8>>,
    pub firebase_rt: Option<Vec<u8>>,
    pub my_modules: serde_json::Value,
    pub balance_cents: i64,
    pub auto_sign: bool,
    pub role: String,
    pub status: String,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub last_refresh_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Account {
    /// 解析 my_modules JSON 为字符串数组。
    pub fn modules(&self) -> Vec<String> {
        serde_json::from_value(self.my_modules.clone()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SignRecord {
    pub id: i64,
    pub account_id: i64,
    pub module_key: String,
    pub module_name: String,
    pub venue: String,
    pub class_date: i64,
    pub start_time: i64,
    pub result: String,
    pub charged_cents: i64,
    pub attempt: i32,
    pub detail: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct BalanceTx {
    pub id: i64,
    pub account_id: i64,
    pub amount_cents: i64,
    pub r#type: String,
    pub ref_id: Option<i64>,
    pub balance_after: i64,
    pub note: String,
    pub operator: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SuperAdmin {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: String,
}

//! 签到引擎核心:Engine 结构 + 单 job 状态机。
//! 全局信号量限总并发,按账号串行(同账号多课逐个签),job 前随机抖动。

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use rand::Rng;
use sqlx::PgPool;
use tokio::sync::{Mutex, Semaphore};

use crate::crypto;
use crate::engine::billing::{self, ClassRef, Outcome};
use crate::engine::instatt::{resolve_bssid, InstAttClient, OngoingClass, SignParams};
use crate::engine::tokens::TokenManager;

/// 网络错误时的内联重试次数。
const MAX_NET_RETRIES: u32 = 3;

pub struct Engine {
    pub pool: PgPool,
    pub tokens: Arc<TokenManager>,
    pub client: InstAttClient,
    pub enc_key: [u8; 32],
    pub price_cents: i64,
    pub poll_interval: Duration,
    pub jitter_ms_max: u64,
    pub sem: Arc<Semaphore>,
    pub acct_locks: DashMap<i64, Arc<Mutex<()>>>,
    pub inflight: DashMap<String, ()>,
}

/// 候选课程对账号的唯一键(用于去重)。
pub fn job_key(account_id: i64, c: &OngoingClass) -> String {
    format!("{}|{}|{}|{}", account_id, c.module_key, c.class_date, c.start_time)
}

impl Engine {
    fn acct_lock(&self, account_id: i64) -> Arc<Mutex<()>> {
        self.acct_locks
            .entry(account_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// 处理一节课的签到。完成后从 inflight 移除该 key。
    pub async fn process_job(self: Arc<Self>, account_id: i64, class: OngoingClass) {
        let key = job_key(account_id, &class);
        // 完成即移除 inflight(无论成功失败)
        let _cleanup = InflightGuard { engine: &self, key: &key };

        let permit = match self.sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };
        let lock = self.acct_lock(account_id);
        let _guard = lock.lock().await;

        // 抖动:摊开瞬时爆发
        if self.jitter_ms_max > 0 {
            let ms = rand::thread_rng().gen_range(0..=self.jitter_ms_max);
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }

        if let Err(e) = self.try_sign(account_id, &class).await {
            tracing::debug!("[{account_id}] {} 签到处理异常: {e}", class.module_key);
        }
        drop(permit);
    }

    async fn try_sign(&self, account_id: i64, class: &OngoingClass) -> anyhow::Result<()> {
        // 1) 校验前置条件 + 取账号字段(优先判断余额是否足够)
        let row: Option<(String, i64, String, Option<Vec<u8>>)> = sqlx::query_as(
            "SELECT status, balance_cents, student_id, device_uid FROM accounts WHERE id=$1",
        )
        .bind(account_id)
        .fetch_optional(&self.pool)
        .await?;
        let (status, balance, student_id, device_uid_enc) = match row {
            Some(r) => r,
            None => return Ok(()),
        };
        if status != "active" || balance < self.price_cents {
            return Ok(()); // 余额不足或账号停用 → 跳过(不记录,余额回升后下轮可签)
        }

        let device_uid = crypto::decrypt_str(&self.enc_key, &device_uid_enc.unwrap_or_default())?;
        if device_uid.is_empty() {
            self.commit(account_id, class, Outcome::Failed("missing_device_uid".into())).await?;
            return Ok(());
        }
        let sign_student_id = if student_id.is_empty() {
            device_uid.chars().take(8).collect::<String>()
        } else {
            student_id
        };

        // 2) BSSID
        let bssid = match resolve_bssid(&class.venue) {
            Some(b) => b,
            None => {
                self.commit(account_id, class, Outcome::Failed(format!("missing_bssid:{}", class.venue))).await?;
                return Ok(());
            }
        };

        // 3) 取短 token(失败已在 TokenManager 内标记 needs_relogin)
        let id_token = self.tokens.get_valid_id_token(account_id).await?;

        // 4) 签到(网络错误内联重试)
        let (ct, cy) = class.course_type_year();
        let params = SignParams {
            module_code: class.module_code().to_string(),
            venue: class.venue.clone(),
            course_type: ct,
            course_year: cy,
            class_date: class.class_date,
            start_time: class.start_time,
            bssid,
            student_id: sign_student_id,
            device_uid,
        };

        let mut last_err = None;
        for attempt in 0..MAX_NET_RETRIES {
            match self.client.sign_attendance(&id_token, &params).await {
                Ok(outcome) => {
                    let mapped = match outcome.status_code {
                        200 => Outcome::Ok200,
                        202 => Outcome::Already202,
                        code => Outcome::Failed(format!("status_{code}:{}", outcome.body)),
                    };
                    let is_success = matches!(mapped, Outcome::Ok200);
                    let r = self.commit(account_id, class, mapped).await?;
                    if r.recorded && is_success {
                        tracing::info!(
                            "[{account_id}] ✓ 签到成功 {} @ {} 扣 {}分",
                            class.module_name, class.venue, r.charged_cents
                        );
                    }
                    return Ok(());
                }
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
                }
            }
        }
        // 网络持续失败:不写终态记录,留待下轮重试
        Err(anyhow::anyhow!("签到网络重试耗尽: {:?}", last_err))
    }

    async fn commit(&self, account_id: i64, class: &OngoingClass, outcome: Outcome) -> anyhow::Result<billing::CommitResult> {
        let cref = ClassRef {
            account_id,
            module_key: class.module_key.clone(),
            module_name: class.module_name.clone(),
            venue: class.venue.clone(),
            class_date: class.class_date,
            start_time: class.start_time,
            attempt: 0,
        };
        billing::commit_outcome(&self.pool, self.price_cents, &cref, outcome).await
    }
}

/// 离开作用域时把 key 从 inflight 移除。
struct InflightGuard<'a> {
    engine: &'a Engine,
    key: &'a str,
}
impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        self.engine.inflight.remove(self.key);
    }
}

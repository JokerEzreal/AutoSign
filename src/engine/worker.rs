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
    /// 实时监听发现新解锁时唤醒轮询器,立即跑一轮而不等下个周期。
    pub wake: tokio::sync::Notify,
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

        // 抖动:摊开瞬时爆发。放在拿并发许可之前,睡觉时不占用名额
        if self.jitter_ms_max > 0 {
            let ms = rand::thread_rng().gen_range(0..=self.jitter_ms_max);
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }

        let permit = match self.sem.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => return,
        };
        let lock = self.acct_lock(account_id);
        let _guard = lock.lock().await;

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

        // 4) 签到(网络错误内联重试);deviceUID 不匹配时同步/注册后重试一次
        let (ct, cy) = class.course_type_year();
        let mut params = SignParams {
            module_code: class.module_code().to_string(),
            venue: class.venue.clone(),
            course_type: ct,
            course_year: cy,
            class_date: class.class_date,
            start_time: class.start_time,
            bssid,
            student_id: sign_student_id.clone(),
            device_uid,
        };

        let mut outcome = self.sign_with_net_retry(&id_token, &params).await?;
        // 上游 409「Student deviceUID does not match」:先同步 Firestore 的 deviceUID;
        // 同步为空则调用 registerDevice 注册一个新设备,然后重签一次。
        if let Outcome::Failed(ref d) = outcome {
            if d.contains("status_409") && d.contains("deviceUID") {
                tracing::warn!("[{account_id}] 签到遇 deviceUID 不匹配,尝试同步/注册后重签");
                if let Some(new_uid) = self
                    .recover_device_uid(account_id, &sign_student_id, &id_token, &params.device_uid)
                    .await
                    .unwrap_or(None)
                {
                    params.device_uid = new_uid;
                    outcome = self.sign_with_net_retry(&id_token, &params).await?;
                }
            }
        }

        let is_success = matches!(outcome, Outcome::Ok200);
        let r = self.commit(account_id, class, outcome).await?;
        if r.recorded && is_success {
            tracing::info!(
                "[{account_id}] ✓ 签到成功 {} @ {} 扣 {}分",
                class.module_name, class.venue, r.charged_cents
            );
        }
        Ok(())
    }

    /// 执行签到,网络错误内联重试;返回映射后的结果(不落库)。
    async fn sign_with_net_retry(&self, id_token: &str, params: &SignParams) -> anyhow::Result<Outcome> {
        let mut last_err = None;
        for attempt in 0..MAX_NET_RETRIES {
            match self.client.sign_attendance(id_token, params).await {
                Ok(o) => {
                    return Ok(match o.status_code {
                        200 => Outcome::Ok200,
                        202 => Outcome::Already202,
                        code => Outcome::Failed(format!("status_{code}:{}", o.body)),
                    });
                }
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
                }
            }
        }
        Err(anyhow::anyhow!("签到网络重试耗尽: {:?}", last_err))
    }

    /// 处理 deviceUID 不匹配:重新同步 Firestore 的 deviceUID;若为空则调 registerDevice 注册。
    /// 返回新的可用 device_uid(已落库);None 表示无法恢复。
    async fn recover_device_uid(
        &self,
        account_id: i64,
        student_id: &str,
        id_token: &str,
        current_uid: &str,
    ) -> anyhow::Result<Option<String>> {
        // 一层:重新同步 Firestore 学生文档里的 deviceUID
        let fresh = self
            .client
            .get_student_info(id_token, student_id)
            .await?
            .map(|i| i.device_uid)
            .unwrap_or_default();
        if !fresh.is_empty() {
            if fresh != current_uid {
                self.persist_device_uid(account_id, &fresh).await?;
                tracing::info!("[{account_id}] 已同步到 Firestore 最新 deviceUID,准备重签");
                return Ok(Some(fresh));
            }
            // 同步值与当前相同仍不匹配 → 无能为力
            return Ok(None);
        }
        // 二层:Firestore 为空(无已注册设备)→ 调 registerDevice 注册一个
        match self.client.register_device(id_token, student_id).await? {
            Some(temp) => {
                let stamp = format!("{student_id}{}", chrono::Utc::now().timestamp());
                if self
                    .client
                    .register_device_success(id_token, student_id, &temp, &stamp)
                    .await?
                {
                    self.persist_device_uid(account_id, &temp).await?;
                    tracing::info!("[{account_id}] 已为空设备注册新 deviceUID,准备重签");
                    Ok(Some(temp))
                } else {
                    tracing::warn!("[{account_id}] registerDeviceSuccess 未返回成功");
                    Ok(None)
                }
            }
            None => {
                tracing::warn!("[{account_id}] registerDevice 返回 304(24h 内已注册),暂无法注册设备");
                Ok(None)
            }
        }
    }

    /// 加密并落库新的 device_uid。
    async fn persist_device_uid(&self, account_id: i64, uid: &str) -> anyhow::Result<()> {
        let enc = crypto::encrypt_str(&self.enc_key, uid)?;
        sqlx::query("UPDATE accounts SET device_uid=$1, last_synced_at=now(), updated_at=now() WHERE id=$2")
            .bind(enc)
            .bind(account_id)
            .execute(&self.pool)
            .await?;
        Ok(())
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

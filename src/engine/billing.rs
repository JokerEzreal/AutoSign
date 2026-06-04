//! 计费:幂等写入签到结果,对成功签到(200)原子扣费。
//!
//! 幂等核心:sign_records 对 (account_id, module_key, class_date, start_time) 有唯一约束。
//! 先插入记录(ON CONFLICT DO NOTHING),冲突即视为已处理 → 绝不重复扣费。
//! 扣费用单语句条件更新,天然防并发超扣。

use sqlx::PgPool;

/// 签到结果类别。
#[derive(Debug, Clone)]
pub enum Outcome {
    /// 上游 200:首次成功,需计费。
    Ok200,
    /// 上游 202:之前已签,不计费。
    Already202,
    /// 失败(含原因),不计费。
    Failed(String),
}

/// 一节课对某账号的引用(写记录所需字段)。
#[derive(Debug, Clone)]
pub struct ClassRef {
    pub account_id: i64,
    pub module_key: String,
    pub module_name: String,
    pub venue: String,
    pub class_date: i64,
    pub start_time: i64,
    pub attempt: i32,
}

#[derive(Debug, Clone)]
pub struct CommitResult {
    /// 本次是否新写入记录(false = 已存在,幂等跳过)。
    pub recorded: bool,
    /// 实际扣费(分)。
    pub charged_cents: i64,
}

/// 写入签到结果。对 Ok200 在同一事务内原子条件扣费。
/// 幂等:若该课已有记录,直接返回 recorded=false,不做任何扣费。
pub async fn commit_outcome(
    pool: &PgPool,
    price_cents: i64,
    class: &ClassRef,
    outcome: Outcome,
) -> anyhow::Result<CommitResult> {
    let (result_str, detail) = match &outcome {
        Outcome::Ok200 => ("ok_200", String::new()),
        Outcome::Already202 => ("already_202", String::new()),
        Outcome::Failed(reason) => ("failed", reason.clone()),
    };

    let mut tx = pool.begin().await?;

    // 先插入记录(charged 先记 0);唯一约束冲突 → 返回 None = 已处理。
    let record_id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO sign_records
            (account_id, module_key, module_name, venue, class_date, start_time, result, charged_cents, attempt, detail)
         VALUES ($1,$2,$3,$4,$5,$6,$7,0,$8,$9)
         ON CONFLICT (account_id, module_key, class_date, start_time) DO NOTHING
         RETURNING id",
    )
    .bind(class.account_id)
    .bind(&class.module_key)
    .bind(&class.module_name)
    .bind(&class.venue)
    .bind(class.class_date)
    .bind(class.start_time)
    .bind(result_str)
    .bind(class.attempt)
    .bind(&detail)
    .fetch_optional(&mut *tx)
    .await?;

    let record_id = match record_id {
        Some(id) => id,
        None => {
            // 已处理过,幂等跳过。
            tx.commit().await?;
            return Ok(CommitResult { recorded: false, charged_cents: 0 });
        }
    };

    let mut charged = 0i64;
    if matches!(outcome, Outcome::Ok200) {
        // 原子条件扣费:余额足才扣。
        let balance_after: Option<i64> = sqlx::query_scalar(
            "UPDATE accounts SET balance_cents = balance_cents - $1, updated_at = now()
             WHERE id = $2 AND balance_cents >= $1
             RETURNING balance_cents",
        )
        .bind(price_cents)
        .bind(class.account_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(after) = balance_after {
            charged = price_cents;
            sqlx::query("UPDATE sign_records SET charged_cents = $1 WHERE id = $2")
                .bind(charged)
                .bind(record_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO balance_transactions
                    (account_id, amount_cents, type, ref_id, balance_after, note, operator)
                 VALUES ($1, $2, 'sign_charge', $3, $4, $5, 'engine')",
            )
            .bind(class.account_id)
            .bind(-price_cents)
            .bind(record_id)
            .bind(after)
            .bind(format!("{} @ {}", class.module_key, class.venue))
            .execute(&mut *tx)
            .await?;
        } else {
            // 余额不足(pre-check 之后的竞态):已签但不扣,标注。
            sqlx::query("UPDATE sign_records SET detail = 'signed_but_insufficient_balance' WHERE id = $1")
                .bind(record_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    tx.commit().await?;
    Ok(CommitResult { recorded: true, charged_cents: charged })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn mk_account(pool: &PgPool, balance: i64) -> i64 {
        sqlx::query_scalar("INSERT INTO accounts (account_name, balance_cents) VALUES ($1,$2) RETURNING id")
            .bind(format!("u{balance}"))
            .bind(balance)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    fn class(account_id: i64, n: i64) -> ClassRef {
        ClassRef {
            account_id,
            module_key: format!("COMP{n}_FML_25-26"),
            module_name: "x".into(),
            venue: "F3C04".into(),
            class_date: 20251126,
            start_time: 900 + n,
            attempt: 0,
        }
    }

    #[sqlx::test]
    async fn ok200_charges_once_and_writes_ledger(pool: PgPool) {
        let id = mk_account(&pool, 500).await;
        let r = commit_outcome(&pool, 100, &class(id, 1), Outcome::Ok200).await.unwrap();
        assert!(r.recorded);
        assert_eq!(r.charged_cents, 100);

        let bal: i64 = sqlx::query_scalar("SELECT balance_cents FROM accounts WHERE id=$1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(bal, 400);

        let tx_count: i64 = sqlx::query_scalar("SELECT count(*) FROM balance_transactions WHERE account_id=$1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(tx_count, 1);
    }

    #[sqlx::test]
    async fn already202_does_not_charge(pool: PgPool) {
        let id = mk_account(&pool, 500).await;
        let r = commit_outcome(&pool, 100, &class(id, 1), Outcome::Already202).await.unwrap();
        assert!(r.recorded);
        assert_eq!(r.charged_cents, 0);
        let bal: i64 = sqlx::query_scalar("SELECT balance_cents FROM accounts WHERE id=$1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(bal, 500);
    }

    #[sqlx::test]
    async fn duplicate_same_class_charges_only_once(pool: PgPool) {
        // 同一节课并发 commit 10 次,只应扣一次。
        let id = mk_account(&pool, 1000).await;
        let mut handles = vec![];
        for _ in 0..10 {
            let pool = pool.clone();
            let c = class(id, 7);
            handles.push(tokio::spawn(async move {
                commit_outcome(&pool, 100, &c, Outcome::Ok200).await.unwrap()
            }));
        }
        let mut recorded = 0;
        for h in handles {
            if h.await.unwrap().recorded { recorded += 1; }
        }
        assert_eq!(recorded, 1, "幂等:只有一次真正写入");
        let bal: i64 = sqlx::query_scalar("SELECT balance_cents FROM accounts WHERE id=$1")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(bal, 900, "只扣一次");
    }

    #[sqlx::test]
    async fn concurrent_distinct_classes_never_overdraft(pool: PgPool) {
        // 余额 300,单价 100。并发 10 节不同课 → 最多扣 3 次,余额不为负。
        let id = Arc::new(mk_account(&pool, 300).await);
        let mut handles = vec![];
        for n in 0..10 {
            let pool = pool.clone();
            let c = class(*id, n);
            handles.push(tokio::spawn(async move {
                commit_outcome(&pool, 100, &c, Outcome::Ok200).await.unwrap().charged_cents
            }));
        }
        let mut total = 0i64;
        for h in handles { total += h.await.unwrap(); }
        assert_eq!(total, 300, "总扣费 = 余额上限");
        let bal: i64 = sqlx::query_scalar("SELECT balance_cents FROM accounts WHERE id=$1")
            .bind(*id).fetch_one(&pool).await.unwrap();
        assert_eq!(bal, 0, "余额恰好为 0,绝不为负");
    }
}

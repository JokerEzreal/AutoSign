//! 课程表:某天的课程 + 解锁 / 官方考勤 / 本系统自动签状态。
//! 数据来源:上游 students/{id}/classes(课表与考勤)、ongoingClasses(实时解锁)、本地 sign_records。

use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{ApiError, ApiResult};
use crate::auth::AuthCtx;
use crate::engine::instatt::{self, OngoingClass, StudentClass};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ClassesQuery {
    /// YYYYMMDD,缺省为校区今天。
    pub date: Option<i64>,
}

/// 本系统对该节课的签到结果(来自 sign_records)。
#[derive(Serialize, Clone)]
pub struct SignInfo {
    pub result: String,
    pub detail: String,
    pub charged_cents: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 一节课的综合视图。
#[derive(Serialize)]
pub struct ClassView {
    pub module_key: String,
    pub module_code: String,
    pub module_name: String,
    pub venue: String,
    /// 教室是否在本系统支持列表内(有 BSSID 或免 WiFi)。
    pub venue_supported: bool,
    pub class_type: i64,
    pub class_type_label: &'static str,
    pub start_time: i64,
    pub end_time: i64,
    pub class_status: i64,
    pub attended: bool,
    pub attendance_time: i64,
    /// 此刻是否在 ongoingClasses 中(已解锁,可签)。
    pub unlocked: bool,
    pub unlock_user_name: String,
    pub students_attended: i64,
    /// 该课程是否已勾选自动签到。
    pub enabled: bool,
    pub sign: Option<SignInfo>,
    /// 综合状态,见 derive_status。
    pub status: &'static str,
}

/// 综合状态判定(纯函数)。`now_hhmm` 为校区当前时刻;查看过去的日期传 2400(全部已结束),
/// 未来的日期传 0(全部未开始)。
///
/// - cancelled:上游已取消
/// - attended:官方考勤已签
/// - unlocked:此刻已解锁、尚未签
/// - absent:已上课(conducted)但未签
/// - upcoming / locked / not_unlocked:待上(YTBC)且未解锁,分别为未开始 / 上课时段内 / 已过时段
pub fn derive_status(c: &StudentClass, unlocked: bool, now_hhmm: i64) -> &'static str {
    if c.class_status == 0 {
        return "cancelled";
    }
    if c.attended {
        return "attended";
    }
    if unlocked {
        return "unlocked";
    }
    if c.class_status == 1 {
        return "absent";
    }
    if now_hhmm < c.start_time {
        "upcoming"
    } else if now_hhmm < c.end_time {
        "locked"
    } else {
        "not_unlocked"
    }
}

/// GET /api/me/classes?date=YYYYMMDD(当前登录用户看自己的课表)
pub async fn list_classes(
    State(st): State<AppState>,
    ctx: AuthCtx,
    Query(q): Query<ClassesQuery>,
) -> ApiResult {
    let id = ctx
        .account_id
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "超管无个人签到账号"))?;
    Ok(Json(build_classes(&st, id, q.date).await?))
}

/// 构建某账号某天的课程表视图(供用户自己与管理员查看复用)。
pub async fn build_classes(
    st: &AppState,
    account_id: i64,
    date_opt: Option<i64>,
) -> Result<serde_json::Value, ApiError> {
    let (today, now_hhmm) = instatt::date_hhmm(&instatt::campus_now());
    let date = date_opt.unwrap_or(today);
    if !(19000101..=21001231).contains(&date) {
        return Err(ApiError::bad("date 须为 YYYYMMDD"));
    }

    let row: Option<(String, serde_json::Value)> =
        sqlx::query_as("SELECT student_id, enabled_modules FROM accounts WHERE id=$1")
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await?;
    let (student_id, enabled) = row.ok_or_else(|| ApiError::not_found("账号不存在"))?;
    if student_id.is_empty() {
        return Err(ApiError::bad("该账号缺少学号(未成功登录过)"));
    }
    let enabled: Vec<String> = serde_json::from_value(enabled).unwrap_or_default();

    let token = st
        .engine
        .tokens
        .get_valid_id_token(account_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("token 失效,需重新登录: {e}")))?;
    let classes = st
        .client
        .get_student_classes(&token, &student_id, date)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("拉取课表失败: {e}")))?;

    // 实时解锁列表只对今天有意义,其他日期不请求
    let ongoing: HashMap<(String, i64, i64), OngoingClass> = if date == today {
        st.client
            .get_ongoing_classes()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|c| ((c.module_key.clone(), c.class_date, c.start_time), c))
            .collect()
    } else {
        HashMap::new()
    };

    // 本系统当天的签到记录
    let recs: Vec<(String, i64, String, String, i64, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT module_key, start_time, result, detail, charged_cents, created_at
         FROM sign_records WHERE account_id=$1 AND class_date=$2",
    )
    .bind(account_id)
    .bind(date)
    .fetch_all(&st.pool)
    .await?;
    let signs: HashMap<(String, i64), SignInfo> = recs
        .into_iter()
        .map(|(module_key, start_time, result, detail, charged_cents, created_at)| {
            ((module_key, start_time), SignInfo { result, detail, charged_cents, created_at })
        })
        .collect();

    // 过去的日期全部视为已结束,未来的全部视为未开始
    let effective_now = if date < today {
        2400
    } else if date > today {
        0
    } else {
        now_hhmm
    };

    let views: Vec<ClassView> = classes
        .iter()
        .map(|c| {
            let og = ongoing.get(&(c.module_key.clone(), c.class_date, c.start_time));
            let unlocked = og.is_some();
            let code = c.module_code().to_uppercase();
            ClassView {
                module_key: c.module_key.clone(),
                module_code: code.clone(),
                module_name: c.module_name.clone(),
                venue: c.venue.clone(),
                venue_supported: instatt::resolve_bssid(&c.venue).is_some(),
                class_type: c.class_type,
                class_type_label: instatt::class_type_label(c.class_type),
                start_time: c.start_time,
                end_time: c.end_time,
                class_status: c.class_status,
                attended: c.attended,
                attendance_time: c.attendance_time,
                unlocked,
                unlock_user_name: og.map(|o| o.unlock_user_name.clone()).unwrap_or_default(),
                students_attended: og.map(|o| o.students_attended).unwrap_or(0),
                // 与 poller 的匹配规则一致:课程代码包含匹配
                enabled: enabled.iter().any(|m| m.to_uppercase().contains(&code)),
                sign: signs.get(&(c.module_key.clone(), c.start_time)).cloned(),
                status: derive_status(c, unlocked, effective_now),
            }
        })
        .collect();

    Ok(json!({
        "date": date,
        "today": today,
        "now_hhmm": now_hhmm,
        "classes": views,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(class_status: i64, attended: bool) -> StudentClass {
        StudentClass {
            module_key: "COMP4082_AUM_26-27".into(),
            module_name: "Autonomous Robotic Systems".into(),
            venue: "BB80".into(),
            class_date: 20260930,
            start_time: 1100,
            end_time: 1300,
            class_type: 5,
            class_status,
            attended,
            attendance_time: 0,
        }
    }

    #[test]
    fn status_matrix() {
        // 取消优先于一切
        assert_eq!(derive_status(&class(0, false), true, 1200), "cancelled");
        // 官方已签优先于解锁
        assert_eq!(derive_status(&class(2, true), true, 1200), "attended");
        assert_eq!(derive_status(&class(1, true), false, 2400), "attended");
        // 解锁中
        assert_eq!(derive_status(&class(2, false), true, 1200), "unlocked");
        // 已上课未签
        assert_eq!(derive_status(&class(1, false), false, 2400), "absent");
        // 待上且未解锁:按时刻分三态
        assert_eq!(derive_status(&class(2, false), false, 1059), "upcoming");
        assert_eq!(derive_status(&class(2, false), false, 1100), "locked");
        assert_eq!(derive_status(&class(2, false), false, 1259), "locked");
        assert_eq!(derive_status(&class(2, false), false, 1300), "not_unlocked");
        // 过去 / 未来日期的约定值
        assert_eq!(derive_status(&class(2, false), false, 2400), "not_unlocked");
        assert_eq!(derive_status(&class(2, false), false, 0), "upcoming");
    }
}

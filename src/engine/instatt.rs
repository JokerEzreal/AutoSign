//! InstAtt 上游客户端:移植自 offline_sign.py / auto_sign_daemon.py。
//! 所有 base URL 可注入,便于用 wiremock 测试。

use serde_json::{json, Value};
use std::collections::HashMap;

// ==================== 常量 ====================

pub const FIREBASE_API_KEY: &str = "AIzaSyDfppFm4OpP33u6vfw7VBf-AvGJUPvJ-mo";
pub const FIREBASE_PROJECT_ID: &str = "instatt-3c80d";
pub const AZURE_TENANT_ID: &str = "274313da-18e1-40ab-97e0-adc6eb1ec699";
pub const AZURE_CLIENT_ID: &str = "e9ed2cb6-da5d-48dc-b8be-28b0d6016e53";

/// 已收集 BSSID 的教室(教室代码, BSSID)。单一数据源。
pub const VENUE_BSSIDS: &[(&str, &str)] = &[
    ("F3C04", "a0:0f:37:e0:3c:2c"),
    ("DA08", "14:84:73:40:3d:ec"),
    ("F4C10", "34:b8:83:56:3e:2c"),
    ("F1A13", "9c:d5:7d:a5:e2:e3"),
    ("F3A04", "a0:0f:37:e1:b0:2c"),
    ("F3A08", "a0:0f:37:e1:b0:2c"),
    ("BB80", "8c:1e:80:22:e9:2c"),
    ("F1A02", "a0:0f:37:e0:57:4c"),
    ("F1A24", "a0:0f:37:e0:57:4c"),
    ("TCR1", "9c:d5:7d:a5:e4:4a"),
    ("TCR2", "9c:d5:7d:a5:e4:4a"), // 与 TCR1 同一 BSSID
    ("F3A12", "9c:d5:7d:a5:a8:0c"),
    ("F3B06", "34:b8:83:5e:03:eb"),
    ("F4B10", "34:b8:83:56:98:0b"),
];

/// 不检查 WiFi 的教室(ignoreWifi=true),任意 BSSID 可签。
pub const IGNORE_WIFI_VENUES: &[&str] = &["DA05", "DA07", "NB03", "NOLOC", "ONLINE"];

/// 新学年起始月份(9 月秋季学期开学)。
const ACADEMIC_YEAR_START_MONTH: u32 = 9;

/// 某日期所属学年,格式与上游 courseYear 一致("YY-YY"):
/// 2026-09-30 → "26-27",2026-06-05 → "25-26"。
pub fn academic_year_for(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let start = if date.month() >= ACADEMIC_YEAR_START_MONTH {
        date.year()
    } else {
        date.year() - 1
    };
    format!("{:02}-{:02}", start % 100, (start + 1) % 100)
}

/// 当前学年(按校区日期)。
pub fn current_academic_year() -> String {
    academic_year_for(campus_now().date_naive())
}

/// 校区时区(马来西亚,UTC+8)。上游 classDate / startTime 均按此时区。
pub const CAMPUS_UTC_OFFSET_HOURS: i32 = 8;

/// 校区当前时间。
pub fn campus_now() -> chrono::DateTime<chrono::FixedOffset> {
    let off = chrono::FixedOffset::east_opt(CAMPUS_UTC_OFFSET_HOURS * 3600).expect("固定时区偏移");
    chrono::Utc::now().with_timezone(&off)
}

/// 时间点 → 上游格式 (classDate=YYYYMMDD, HHMM)。
pub fn date_hhmm<Tz: chrono::TimeZone>(t: &chrono::DateTime<Tz>) -> (i64, i64) {
    use chrono::{Datelike, Timelike};
    let d = t.year() as i64 * 10000 + t.month() as i64 * 100 + t.day() as i64;
    let h = t.hour() as i64 * 100 + t.minute() as i64;
    (d, h)
}

/// 上游 global/classType 的课型编码。
pub const CLASS_TYPES: &[(i64, &str)] = &[
    (0, "lecture"),
    (1, "tutorial"),
    (2, "seminar"),
    (3, "workshop"),
    (4, "practical"),
    (5, "computing"),
    (6, "field trip"),
    (7, "lab"),
    (8, "screening"),
    (9, "assessment"),
    (10, "drop-in"),
    (11, "presentation"),
    (12, "placement"),
];

/// 课型编码 → 名称(未知编码返回空串)。
pub fn class_type_label(t: i64) -> &'static str {
    CLASS_TYPES
        .iter()
        .find(|(k, _)| *k == t)
        .map(|(_, v)| *v)
        .unwrap_or("")
}

/// 教室 BSSID 查找(传入大写教室代码)。
pub fn venue_bssid(venue_upper: &str) -> Option<&'static str> {
    VENUE_BSSIDS
        .iter()
        .find(|(v, _)| *v == venue_upper)
        .map(|(_, b)| *b)
}

/// 是否为免 WiFi 教室。
pub fn is_ignore_wifi(venue_upper: &str) -> bool {
    IGNORE_WIFI_VENUES.contains(&venue_upper)
}

/// 当前支持自动签到的全部教室代码(有 BSSID 的 + 免 WiFi 的)。
pub fn supported_venues() -> Vec<&'static str> {
    VENUE_BSSIDS
        .iter()
        .map(|(v, _)| *v)
        .chain(IGNORE_WIFI_VENUES.iter().copied())
        .collect()
}

/// 解析教室应使用的 BSSID。None 表示既非 ignoreWifi 又缺 BSSID,无法签到。
pub fn resolve_bssid(venue: &str) -> Option<String> {
    let up = venue.to_uppercase();
    if is_ignore_wifi(&up) {
        Some("00:00:00:00:00:00".to_string())
    } else {
        venue_bssid(&up).map(|s| s.to_string())
    }
}

// ==================== 数据结构 ====================

#[derive(Debug, Clone)]
pub struct DeviceCodeStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: i64,
    pub expires_in: i64,
}

#[derive(Debug, Clone)]
pub enum DevicePoll {
    Pending,
    Declined,
    Expired,
    /// 认证成功,拿到 Azure access/refresh token。
    Success { access_token: String, refresh_token: String },
}

#[derive(Debug, Clone, Default)]
pub struct UserInfo {
    pub display_name: String,
    pub mail: String,
    pub employee_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct StudentInfo {
    pub device_uid: String,
    pub account_name: String,
    pub email: String,
    pub course: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct Module {
    pub module_id: String,
    pub module_name: String,
}

/// 把课程列表构建为 { 课程代码: 课程名 } 的 JSON 映射。
pub fn modules_to_info_map(mods: &[Module]) -> serde_json::Value {
    let map: serde_json::Map<String, Value> = mods
        .iter()
        .map(|m| (m.module_id.clone(), Value::String(m.module_name.clone())))
        .collect();
    Value::Object(map)
}

#[derive(Debug, Clone)]
pub struct OngoingClass {
    pub module_key: String,
    pub module_name: String,
    pub venue: String,
    pub class_date: i64,
    pub start_time: i64,
    pub end_time: i64,
    /// 已签到人数(上游 studentsAttended)。
    pub students_attended: i64,
    /// 解锁老师姓名(上游 unlockUserName)。
    pub unlock_user_name: String,
}

/// students/{id}/classes 中的一节课:学生视角的课表 + 官方考勤。
#[derive(Debug, Clone)]
pub struct StudentClass {
    pub module_key: String,
    pub module_name: String,
    pub venue: String,
    pub class_date: i64,
    pub start_time: i64,
    pub end_time: i64,
    /// 课型编码,见 CLASS_TYPES。
    pub class_type: i64,
    /// 0 cancelled / 1 conducted / 2 待上(YTBC)。
    pub class_status: i64,
    /// 官方考勤是否已签。
    pub attended: bool,
    /// 官方签到时刻 YYYYMMDDHHMM,未签为 0。
    pub attendance_time: i64,
}

impl StudentClass {
    /// 课程代码部分(module_key 第一段,如 COMP4082)。
    pub fn module_code(&self) -> &str {
        self.module_key.split('_').next().unwrap_or(&self.module_key)
    }
}

impl OngoingClass {
    /// 课程代码部分(module_key 第一段,如 COMP4082)。
    pub fn module_code(&self) -> &str {
        self.module_key.split('_').next().unwrap_or(&self.module_key)
    }
    /// course_type(第二段)与 course_year(第三段),缺省分别为 AUM 与当前学年。
    pub fn course_type_year(&self) -> (String, String) {
        let parts: Vec<&str> = self.module_key.split('_').collect();
        let ct = parts.get(1).copied().unwrap_or("AUM").to_string();
        let cy = parts
            .get(2)
            .map(|s| s.to_string())
            .unwrap_or_else(current_academic_year);
        (ct, cy)
    }
}

#[derive(Debug, Clone)]
pub struct SignParams {
    pub module_code: String,
    pub venue: String,
    pub course_type: String,
    pub course_year: String,
    pub class_date: i64,
    pub start_time: i64,
    pub bssid: String,
    pub student_id: String,
    pub device_uid: String,
}

#[derive(Debug, Clone)]
pub struct SignOutcome {
    pub status_code: i64,
    pub body: String,
}

// ==================== 客户端 ====================

#[derive(Clone)]
pub struct InstAttClient {
    http: reqwest::Client,
    pub login_base: String,           // https://login.microsoftonline.com
    pub securetoken_base: String,     // https://securetoken.googleapis.com
    pub identitytoolkit_base: String, // https://identitytoolkit.googleapis.com
    pub functions_base: String,       // https://us-central1-instatt-3c80d.cloudfunctions.net
    pub graph_base: String,           // https://graph.microsoft.com
    pub firestore_base: String,       // https://firestore.googleapis.com
}

impl Default for InstAttClient {
    fn default() -> Self {
        Self::new()
    }
}

impl InstAttClient {
    /// 指向真实上游的客户端。
    pub fn new() -> Self {
        Self::with_bases(
            "https://login.microsoftonline.com",
            "https://securetoken.googleapis.com",
            "https://identitytoolkit.googleapis.com",
            "https://us-central1-instatt-3c80d.cloudfunctions.net",
            "https://graph.microsoft.com",
            "https://firestore.googleapis.com",
        )
    }

    /// 自定义全部 base URL(测试用,通常都指向同一个 mock server)。
    pub fn with_bases(
        login: &str,
        securetoken: &str,
        identitytoolkit: &str,
        functions: &str,
        graph: &str,
        firestore: &str,
    ) -> Self {
        InstAttClient {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("构建 reqwest 客户端失败"),
            login_base: login.trim_end_matches('/').to_string(),
            securetoken_base: securetoken.trim_end_matches('/').to_string(),
            identitytoolkit_base: identitytoolkit.trim_end_matches('/').to_string(),
            functions_base: functions.trim_end_matches('/').to_string(),
            graph_base: graph.trim_end_matches('/').to_string(),
            firestore_base: firestore.trim_end_matches('/').to_string(),
        }
    }

    fn firestore_docs(&self) -> String {
        format!(
            "{}/v1/projects/{}/databases/(default)/documents",
            self.firestore_base, FIREBASE_PROJECT_ID
        )
    }

    // ---------- Azure Device Code ----------

    pub async fn device_code_start(&self) -> anyhow::Result<DeviceCodeStart> {
        let url = format!("{}/{}/oauth2/v2.0/devicecode", self.login_base, AZURE_TENANT_ID);
        let res: Value = self
            .http
            .post(&url)
            .form(&[
                ("client_id", AZURE_CLIENT_ID),
                ("scope", "https://graph.microsoft.com/.default offline_access"),
            ])
            .send()
            .await?
            .json()
            .await?;
        let user_code = res["user_code"].as_str()
            .ok_or_else(|| anyhow::anyhow!("devicecode 响应缺少 user_code: {res}"))?;
        Ok(DeviceCodeStart {
            device_code: res["device_code"].as_str().unwrap_or_default().to_string(),
            user_code: user_code.to_string(),
            verification_uri: res["verification_uri"].as_str().unwrap_or_default().to_string(),
            interval: res["interval"].as_i64().unwrap_or(5),
            expires_in: res["expires_in"].as_i64().unwrap_or(900),
        })
    }

    pub async fn device_code_poll(&self, device_code: &str) -> anyhow::Result<DevicePoll> {
        let url = format!("{}/{}/oauth2/v2.0/token", self.login_base, AZURE_TENANT_ID);
        let res: Value = self
            .http
            .post(&url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", AZURE_CLIENT_ID),
                ("device_code", device_code),
            ])
            .send()
            .await?
            .json()
            .await?;
        if let Some(access) = res["access_token"].as_str() {
            return Ok(DevicePoll::Success {
                access_token: access.to_string(),
                refresh_token: res["refresh_token"].as_str().unwrap_or_default().to_string(),
            });
        }
        Ok(match res["error"].as_str().unwrap_or("") {
            "authorization_pending" => DevicePoll::Pending,
            "authorization_declined" => DevicePoll::Declined,
            "expired_token" => DevicePoll::Expired,
            _ => DevicePoll::Pending,
        })
    }

    pub async fn refresh_azure(&self, refresh_token: &str) -> anyhow::Result<String> {
        let url = format!("{}/{}/oauth2/v2.0/token", self.login_base, AZURE_TENANT_ID);
        let res: Value = self
            .http
            .post(&url)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", AZURE_CLIENT_ID),
                ("refresh_token", refresh_token),
                ("scope", "https://graph.microsoft.com/.default offline_access"),
            ])
            .send()
            .await?
            .json()
            .await?;
        res["access_token"].as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("azure 刷新无 access_token: {res}"))
    }

    // ---------- Firebase ----------

    pub async fn firebase_custom_token(&self, azure_access: &str) -> anyhow::Result<String> {
        let url = format!("{}/userLogin", self.functions_base);
        let res: Value = self
            .http
            .post(&url)
            .json(&json!({"data": {"token": azure_access}}))
            .send()
            .await?
            .json()
            .await?;
        if res["result"]["statusCode"].as_i64() == Some(200) {
            if let Some(t) = res["result"]["token"].as_str() {
                return Ok(t.to_string());
            }
        }
        Err(anyhow::anyhow!("获取 firebase custom token 失败: {res}"))
    }

    /// 返回 (id_token, firebase_refresh_token)。
    pub async fn firebase_id_token(&self, custom_token: &str) -> anyhow::Result<(String, String)> {
        let url = format!(
            "{}/v1/accounts:signInWithCustomToken?key={}",
            self.identitytoolkit_base, FIREBASE_API_KEY
        );
        let res: Value = self
            .http
            .post(&url)
            .json(&json!({"token": custom_token, "returnSecureToken": true}))
            .send()
            .await?
            .json()
            .await?;
        let id = res["idToken"].as_str()
            .ok_or_else(|| anyhow::anyhow!("无 idToken: {res}"))?;
        Ok((id.to_string(), res["refreshToken"].as_str().unwrap_or_default().to_string()))
    }

    pub async fn refresh_firebase(&self, firebase_rt: &str) -> anyhow::Result<String> {
        let url = format!("{}/v1/token?key={}", self.securetoken_base, FIREBASE_API_KEY);
        let res: Value = self
            .http
            .post(&url)
            .form(&[("grant_type", "refresh_token"), ("refresh_token", firebase_rt)])
            .send()
            .await?
            .json()
            .await?;
        res["id_token"].as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("firebase 刷新无 id_token: {res}"))
    }

    // ---------- Graph / Firestore 查询 ----------

    pub async fn get_user_info(&self, azure_access: &str) -> anyhow::Result<UserInfo> {
        let url = format!(
            "{}/v1.0/me?$select=id,displayName,mail,userPrincipalName,employeeId,department",
            self.graph_base
        );
        let res: Value = self
            .http
            .get(&url)
            .bearer_auth(azure_access)
            .send()
            .await?
            .json()
            .await?;
        Ok(UserInfo {
            display_name: res["displayName"].as_str().unwrap_or_default().to_string(),
            mail: res["mail"].as_str().unwrap_or_default().to_string(),
            employee_id: res["employeeId"].as_str().unwrap_or_default().to_string(),
        })
    }

    pub async fn get_student_info(&self, id_token: &str, student_id: &str) -> anyhow::Result<Option<StudentInfo>> {
        let url = format!("{}/students/{}", self.firestore_docs(), student_id);
        let resp = self.http.get(&url).bearer_auth(id_token).send().await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let res: Value = resp.json().await?;
        let f = &res["fields"];
        if f.is_null() {
            return Ok(None);
        }
        Ok(Some(StudentInfo {
            device_uid: sv(f, "deviceUID"),
            account_name: sv(f, "accountName"),
            email: sv(f, "email"),
            course: sv(f, "courseDescription"),
            status: sv(f, "status"),
        }))
    }

    pub async fn get_student_modules(
        &self,
        id_token: &str,
        student_id: &str,
        current_year: &str,
    ) -> anyhow::Result<Vec<Module>> {
        let url = format!("{}/students/{}/modules", self.firestore_docs(), student_id);
        let resp = self.http.get(&url).bearer_auth(id_token).send().await?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let res: Value = resp.json().await?;
        // 上游返回历年全部课程,先按 (courseYear, Module) 收齐
        let mut all: Vec<(String, Module)> = vec![];
        if let Some(docs) = res["documents"].as_array() {
            for doc in docs {
                let f = &doc["fields"];
                let id = sv(f, "moduleID");
                if id.is_empty() {
                    continue;
                }
                all.push((
                    sv(f, "courseYear"),
                    Module {
                        module_id: id,
                        module_name: sv(f, "moduleName"),
                    },
                ));
            }
        }
        // 优先当前学年;上游尚未录入当前学年(如开学初)时退回最新学年,避免列表为空
        let year = if all.iter().any(|(y, _)| y == current_year) {
            current_year.to_string()
        } else {
            all.iter().map(|(y, _)| y.clone()).max().unwrap_or_default()
        };
        Ok(all
            .into_iter()
            .filter(|(y, _)| *y == year)
            .map(|(_, m)| m)
            .collect())
    }

    pub async fn get_ongoing_classes(&self) -> anyhow::Result<Vec<OngoingClass>> {
        let url = format!("{}/ongoingClasses", self.firestore_docs());
        let res: Value = self.http.get(&url).send().await?.json().await?;
        let mut out = vec![];
        if let Some(docs) = res["documents"].as_array() {
            for doc in docs {
                let f = &doc["fields"];
                out.push(OngoingClass {
                    module_key: sv(f, "moduleKey"),
                    module_name: sv(f, "moduleName"),
                    venue: sv(f, "venue"),
                    class_date: iv(f, "classDate"),
                    start_time: iv(f, "startTime"),
                    end_time: iv(f, "endTime"),
                    students_attended: iv(f, "studentsAttended"),
                    unlock_user_name: sv(f, "unlockUserName"),
                });
            }
        }
        Ok(out)
    }

    /// 学生某天的课表:runQuery students/{id}/classes where classDate == date,按开始时间排序。
    pub async fn get_student_classes(
        &self,
        id_token: &str,
        student_id: &str,
        class_date: i64,
    ) -> anyhow::Result<Vec<StudentClass>> {
        let url = format!("{}/students/{}:runQuery", self.firestore_docs(), student_id);
        let body = json!({"structuredQuery": {
            "from": [{"collectionId": "classes"}],
            "where": {"fieldFilter": {
                "field": {"fieldPath": "classDate"},
                "op": "EQUAL",
                "value": {"integerValue": class_date.to_string()}
            }},
            "limit": 50
        }});
        let res: Value = self
            .http
            .post(&url)
            .bearer_auth(id_token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        if let Some(e) = res.get("error") {
            return Err(anyhow::anyhow!("runQuery 失败: {e}"));
        }
        let mut out = vec![];
        if let Some(items) = res.as_array() {
            for item in items {
                let f = &item["document"]["fields"];
                if f.is_null() {
                    continue; // 末尾只有 readTime 的空条目
                }
                out.push(StudentClass {
                    module_key: sv(f, "moduleKey"),
                    module_name: sv(f, "moduleName"),
                    venue: sv(f, "venue"),
                    class_date: iv(f, "classDate"),
                    start_time: iv(f, "startTime"),
                    end_time: iv(f, "endTime"),
                    class_type: iv(f, "classType"),
                    class_status: iv(f, "classStatus"),
                    attended: iv(f, "attended") == 1,
                    attendance_time: iv(f, "attendanceDateTime"),
                });
            }
        }
        out.sort_by(|a, b| (a.start_time, &a.module_key).cmp(&(b.start_time, &b.module_key)));
        Ok(out)
    }

    // ---------- 签到 ----------

    pub async fn sign_attendance(&self, id_token: &str, p: &SignParams) -> anyhow::Result<SignOutcome> {
        let url = format!("{}/signAttendance", self.functions_base);
        let body = json!({"data": {
            "moduleID": p.module_code,
            "venue": p.venue,
            "courseType": p.course_type,
            "courseYear": p.course_year,
            "classDate": p.class_date,
            "startTime": p.start_time,
            "MACaddress": p.bssid,
            "studentID": p.student_id,
            "deviceUID": p.device_uid,
        }});
        let res: Value = self
            .http
            .post(&url)
            .bearer_auth(id_token)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        let result = &res["result"];
        if result.is_null() {
            return Err(anyhow::anyhow!("签到响应异常: {res}"));
        }
        Ok(SignOutcome {
            status_code: result["statusCode"].as_i64().unwrap_or(0),
            body: match &result["body"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
        })
    }
}

/// 取 Firestore 字段的 stringValue。
fn sv(fields: &Value, key: &str) -> String {
    fields[key]["stringValue"].as_str().unwrap_or_default().to_string()
}

/// 取 Firestore 字段的 integerValue(字符串形式数字)。
fn iv(fields: &Value, key: &str) -> i64 {
    match &fields[key]["integerValue"] {
        Value::String(s) => s.parse().unwrap_or(0),
        Value::Number(n) => n.as_i64().unwrap_or(0),
        _ => 0,
    }
}

/// 占位以消除未使用告警(HashMap 仅文档示意)。
#[allow(dead_code)]
fn _unused() -> HashMap<String, String> {
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client_for(server: &MockServer) -> InstAttClient {
        let base = server.uri();
        InstAttClient::with_bases(&base, &base, &base, &base, &base, &base)
    }

    #[test]
    fn bssid_resolution() {
        assert_eq!(resolve_bssid("F3C04").as_deref(), Some("a0:0f:37:e0:3c:2c"));
        assert_eq!(resolve_bssid("online").as_deref(), Some("00:00:00:00:00:00"));
        assert_eq!(resolve_bssid("UNKNOWN"), None);
    }

    #[test]
    fn academic_year_boundaries() {
        use chrono::NaiveDate;
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(academic_year_for(d(2026, 9, 30)), "26-27");
        assert_eq!(academic_year_for(d(2026, 9, 1)), "26-27");
        assert_eq!(academic_year_for(d(2026, 8, 31)), "25-26");
        assert_eq!(academic_year_for(d(2026, 6, 5)), "25-26");
        assert_eq!(academic_year_for(d(2027, 1, 15)), "26-27");
        assert_eq!(academic_year_for(d(2099, 10, 1)), "99-00");
    }

    #[test]
    fn module_key_parsing() {
        let c = OngoingClass {
            module_key: "COMP4082_FML_25-26".into(),
            module_name: "x".into(),
            venue: "F3C04".into(),
            class_date: 0,
            start_time: 0,
            end_time: 0,
            students_attended: 0,
            unlock_user_name: String::new(),
        };
        assert_eq!(c.module_code(), "COMP4082");
        assert_eq!(c.course_type_year(), ("FML".to_string(), "25-26".to_string()));
    }

    #[test]
    fn campus_date_hhmm_and_class_types() {
        use chrono::TimeZone;
        let off = chrono::FixedOffset::east_opt(CAMPUS_UTC_OFFSET_HOURS * 3600).unwrap();
        // UTC 04:56 = 校区 12:56
        let t = chrono::Utc.with_ymd_and_hms(2026, 9, 30, 4, 56, 0).unwrap().with_timezone(&off);
        assert_eq!(date_hhmm(&t), (20260930, 1256));
        // UTC 23:30 → 校区已是次日 07:30
        let t = chrono::Utc.with_ymd_and_hms(2026, 9, 30, 23, 30, 0).unwrap().with_timezone(&off);
        assert_eq!(date_hhmm(&t), (20261001, 730));
        assert_eq!(class_type_label(5), "computing");
        assert_eq!(class_type_label(0), "lecture");
        assert_eq!(class_type_label(99), "");
    }

    #[tokio::test]
    async fn student_classes_run_query_parsing() {
        let server = MockServer::start().await;
        // 上游 runQuery 响应:文档条目 + 末尾一个只含 readTime 的空条目;故意乱序
        let body = json!([
            {"document": {"name": "projects/x/databases/(default)/documents/students/12345678/classes/COMP4082_AUM_26-27_20260930_1400_F1A24",
              "fields": {"moduleKey": {"stringValue": "COMP4082_AUM_26-27"}, "moduleName": {"stringValue": "Autonomous Robotic Systems"},
                         "venue": {"stringValue": "F1A24"}, "classDate": {"integerValue": "20260930"}, "startTime": {"integerValue": "1400"},
                         "endTime": {"integerValue": "1600"}, "classType": {"integerValue": "0"}, "classStatus": {"integerValue": "2"},
                         "attended": {"integerValue": "0"}}}},
            {"document": {"name": "projects/x/databases/(default)/documents/students/12345678/classes/COMP3041_AUM_26-27_20260930_0900_F1A13",
              "fields": {"moduleKey": {"stringValue": "COMP3041_AUM_26-27"}, "moduleName": {"stringValue": "Ethics"},
                         "venue": {"stringValue": "F1A13"}, "classDate": {"integerValue": "20260930"}, "startTime": {"integerValue": "900"},
                         "endTime": {"integerValue": "1100"}, "classType": {"integerValue": "0"}, "classStatus": {"integerValue": "1"},
                         "attended": {"integerValue": "1"}, "attendanceDateTime": {"integerValue": "202609300912"}}}},
            {"readTime": "2026-09-30T04:56:00Z"}
        ]);
        Mock::given(method("POST"))
            .and(path(format!(
                "/v1/projects/{}/databases/(default)/documents/students/12345678:runQuery",
                FIREBASE_PROJECT_ID
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let classes = client_for(&server)
            .get_student_classes("idtok", "12345678", 20260930)
            .await
            .unwrap();
        assert_eq!(classes.len(), 2);
        assert_eq!(classes[0].module_code(), "COMP3041", "应按开始时间排序");
        assert!(classes[0].attended);
        assert_eq!(classes[0].attendance_time, 202609300912);
        assert_eq!(classes[1].start_time, 1400);
        assert!(!classes[1].attended);
        assert_eq!(classes[1].class_status, 2);
    }

    #[tokio::test]
    async fn student_classes_run_query_error_is_err() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403).set_body_json(
                json!({"error": {"code": 403, "message": "Missing or insufficient permissions.", "status": "PERMISSION_DENIED"}}),
            ))
            .mount(&server)
            .await;
        let r = client_for(&server).get_student_classes("idtok", "12345678", 20260930).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn parse_ongoing_classes() {
        let server = MockServer::start().await;
        let body = json!({"documents": [
            {"fields": {
                "moduleKey": {"stringValue": "BUSI2152_AUM_25-26"},
                "moduleName": {"stringValue": "Management Accounting"},
                "venue": {"stringValue": "F3A08"},
                "classDate": {"integerValue": "20251126"},
                "startTime": {"integerValue": "930"},
                "endTime": {"integerValue": "1100"}
            }}
        ]});
        Mock::given(method("GET"))
            .and(path(format!(
                "/v1/projects/{}/databases/(default)/documents/ongoingClasses",
                FIREBASE_PROJECT_ID
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let classes = client_for(&server).get_ongoing_classes().await.unwrap();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].module_code(), "BUSI2152");
        assert_eq!(classes[0].start_time, 930);
        assert_eq!(classes[0].class_date, 20251126);
    }

    #[tokio::test]
    async fn sign_attendance_status_codes() {
        for (code, expect) in [(200i64, 200i64), (202, 202), (403, 403)] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/signAttendance"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"result": {"statusCode": code, "body": "msg"}}),
                ))
                .mount(&server)
                .await;
            let p = SignParams {
                module_code: "COMP1001".into(),
                venue: "F3C04".into(),
                course_type: "FML".into(),
                course_year: "25-26".into(),
                class_date: 20251126,
                start_time: 1400,
                bssid: "a0:0f:37:e0:3c:2c".into(),
                student_id: "12345678".into(),
                device_uid: "12345678abcd".into(),
            };
            let out = client_for(&server).sign_attendance("idtok", &p).await.unwrap();
            assert_eq!(out.status_code, expect);
            assert_eq!(out.body, "msg");
        }
    }

    #[tokio::test]
    async fn firebase_id_token_parsing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/accounts:signInWithCustomToken"))
            .and(query_param("key", FIREBASE_API_KEY))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"idToken": "ID123", "refreshToken": "RT456"}),
            ))
            .mount(&server)
            .await;
        let (id, rt) = client_for(&server).firebase_id_token("custom").await.unwrap();
        assert_eq!(id, "ID123");
        assert_eq!(rt, "RT456");
    }

    #[tokio::test]
    async fn student_modules_filters_by_year() {
        let server = MockServer::start().await;
        let body = json!({"documents": [
            {"fields": {"courseYear": {"stringValue": "25-26"}, "moduleID": {"stringValue": "COMP4082"}, "moduleName": {"stringValue": "AI"}}},
            {"fields": {"courseYear": {"stringValue": "24-25"}, "moduleID": {"stringValue": "OLD1001"}, "moduleName": {"stringValue": "Old"}}}
        ]});
        Mock::given(method("GET"))
            .and(path(format!(
                "/v1/projects/{}/databases/(default)/documents/students/12345678/modules",
                FIREBASE_PROJECT_ID
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let mods = client_for(&server)
            .get_student_modules("idtok", "12345678", "25-26")
            .await
            .unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].module_id, "COMP4082");
    }

    #[tokio::test]
    async fn student_modules_prefers_current_year_else_newest() {
        let server = MockServer::start().await;
        // 上游历年课程:24-25 ×1、25-26 ×2、26-27 ×1(同 niubi777 的真实数据形态)
        let body = json!({"documents": [
            {"fields": {"courseYear": {"stringValue": "25-26"}, "moduleID": {"stringValue": "COMP2019"}, "moduleName": {"stringValue": "SE Group Project"}}},
            {"fields": {"courseYear": {"stringValue": "24-25"}, "moduleID": {"stringValue": "COMP1017"}, "moduleName": {"stringValue": "Maths 1"}}},
            {"fields": {"courseYear": {"stringValue": "26-27"}, "moduleID": {"stringValue": "COMP4082"}, "moduleName": {"stringValue": "Autonomous Robotic Systems"}}},
            {"fields": {"courseYear": {"stringValue": "25-26"}, "moduleID": {"stringValue": "COMP2025"}, "moduleName": {"stringValue": "HCI"}}}
        ]});
        Mock::given(method("GET"))
            .and(path(format!(
                "/v1/projects/{}/databases/(default)/documents/students/12345678/modules",
                FIREBASE_PROJECT_ID
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = client_for(&server);

        // 请求 26-27:只取 26-27 那一门,25-26 的不再混入
        let mods = client.get_student_modules("idtok", "12345678", "26-27").await.unwrap();
        assert_eq!(mods.iter().map(|m| m.module_id.as_str()).collect::<Vec<_>>(), vec!["COMP4082"]);

        // 请求一个上游还没有的学年(27-28):退回最新学年 26-27,而不是返回空
        let mods = client.get_student_modules("idtok", "12345678", "27-28").await.unwrap();
        assert_eq!(mods.iter().map(|m| m.module_id.as_str()).collect::<Vec<_>>(), vec!["COMP4082"]);
    }

    #[tokio::test]
    async fn device_poll_pending_then_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/{}/oauth2/v2.0/token", AZURE_TENANT_ID)))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"access_token": "AZ_ACC", "refresh_token": "AZ_RT"}),
            ))
            .mount(&server)
            .await;
        let poll = client_for(&server).device_code_poll("dc").await.unwrap();
        match poll {
            DevicePoll::Success { access_token, refresh_token } => {
                assert_eq!(access_token, "AZ_ACC");
                assert_eq!(refresh_token, "AZ_RT");
            }
            other => panic!("期望 Success,得到 {other:?}"),
        }
    }
}

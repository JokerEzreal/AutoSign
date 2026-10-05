# InstAtt Firebase Database 公开信息

> 语言:[English](InstAtt_Database_Info.md) · **简体中文**

> 数据获取时间: 2025-11-26 12:40 (服务器时间)
>
> Firebase Project ID: `instatt-3c80d`
>
> Firestore REST API: `https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/`

---

## 访问权限总结

| 集合 | 权限 | 说明 |
|------|------|------|
| `global/*` | **公开读取** | 系统配置信息 |
| `rooms/*` | **公开读取** | 教室配置信息 |
| `ongoingClasses` | **公开读取** | **实时解锁课程列表** |
| `students/*` | 需要认证 | 学生信息 |
| `modules/*` | 需要认证 | 课程模块信息 |
| `lecturers/*` | 需要认证 | 讲师信息 |

---

## 1. 全局配置 (global)

### 1.1 服务器时间 (global/time)
```
curl "https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/global/time"
```

| 字段 | 值 | 说明 |
|------|------|------|
| serverDate | 20251126 | 服务器日期 (YYYYMMDD) |
| serverTime | 1240 | 服务器时间 (HHMM) |
| unixTime | 1764132001 | Unix时间戳 |

### 1.2 SSID白名单 (global/ssidFilters)
允许签到的WiFi SSID:
- `eduroam` ✅
- `unmc-guest` ✅
- `unm-guest` ✅

### 1.3 课程类型 (global/classType)

| 代码 | 类型 |
|------|------|
| 0 | lecture |
| 1 | tutorial |
| 2 | seminar |
| 3 | workshop |
| 4 | practical |
| 5 | computing |
| 6 | field Trip |
| 7 | lab |
| 8 | screening |
| 9 | assessment |
| 10 | drop-in |
| 11 | presentation |
| 12 | placement |

### 1.4 课程状态 (global/classStatus)

| 代码 | 状态 |
|------|------|
| 0 | cancelled |
| 1 | conducted |
| 2 | YTBC (Yet To Be Conducted) |

### 1.5 锁定状态 (global/lockStatus)

| 代码 | 状态 |
|------|------|
| 0 | locked |
| 1 | unlocked |
| ignoreWifi | false (全局默认) |

### 1.6 用户权限类型 (global/privilegeType)

| 代码 | 类型 |
|------|------|
| 0 | student |
| 1 | lecturer |
| 1.5 | visa |
| 2 | assistant |
| 3 | admin |

### 1.7 考勤警告阈值 (global/warningThreshold)

| 字段 | 值 |
|------|------|
| absentThreshold | 80% |
| firstLevelWarningAverageAttendanceRateThreshold | 81% |
| secondLevelWarningAverageAttendanceRateThreshold | 79% |
| ceilingCourseAttendanceRateThreshold | 79% |
| floorCourseAttendanceRateThreshold | 76% |
| secondLevelWarningCourseAttendanceRateThreshold | 74% |

### 1.8 警告状态类型 (global/warningStatus)

| 代码 | 类型 |
|------|------|
| 0 | Extenuating Circumstances |
| 1 | Absence for Consecutive Days |
| 2 | Low Attendance |

### 1.9 App版本信息 (global/latestAppVersion)

| 平台 | 版本 |
|------|------|
| Android | 1.42 |
| iOS | 1.2.2 |
| iOS Store URL | itms-apps://itunes.apple.com/my/app/instatt/id1432497825?mt=8 |

### 1.10 当前学年 (global/courseType)
- courseYear: `25-26`

### 1.11 支持邮箱 (global/supportEmail)
- email: `instatt.attendance@nottingham.edu.my`

---

## 2. 实时解锁课程 (ongoingClasses)

**这是最关键的公开数据！可以实时监控哪些课程已解锁。**

```bash
curl "https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/ongoingClasses"
```

### 当前解锁课程列表 (2025-11-26 12:40)

| 课程代码 | 课程名称 | 教室 | 时间 | 解锁老师 | 已签到 |
|----------|----------|------|------|----------|--------|
| BUSI2152_AUM_25-26 | Management Accounting | F3A08 | 09:30-11:00 | HUNG WOAN TING | 72人 |
| BUSI4641_AUM_25-26 | Employability and Global Human Resource Management | F3B03 | 11:00-13:00 | VANITHA PONNUSAMY | 4人 |
| CIVE2047_FML_25-26 | Portfolio of Civil Engineering Studies 2 | B1C26 | 09:00-13:00 | JING YING WONG | 0人 |
| EDUC2028_AUM_25-26 | Teaching Language across the Curriculum | BA64 | 12:00-13:00 | SHARIMILA AMBROSE | 0人 |
| FNDSF017_AUM_25-26 | Programming | TCR1 | 11:00-13:00 | REGINAMARY MATTHEWS | 44人 |
| MMME1035_FML_25-26 | Engineering Design and Design Project | F1A02 | 11:00-13:00 | CHIN SEONG LIM | 0人 |

### ongoingClasses 文档字段说明

| 字段 | 类型 | 说明 |
|------|------|------|
| moduleKey | string | 课程唯一标识 (格式: COURSECODE_TYPE_YEAR) |
| moduleName | string | 课程名称 |
| venue | string | 教室代码 |
| classDate | int | 日期 (YYYYMMDD) |
| startTime | int | 开始时间 (HHMM, 如 930 = 09:30) |
| endTime | int | 结束时间 |
| classType | int | 课程类型 (见上表) |
| classStatus | int | 课程状态 |
| lockStatus | int | 锁定状态 (1=已解锁) |
| attendanceType | int | 签到类型 |
| studentsAttended | int | 已签到学生数 |
| unlockUserName | string | 解锁的老师姓名 |
| unlockDateTimeStamp | int | 解锁时间戳 (YYYYMMDDHHMM) |

---

## 3. 教室信息 (rooms)

### 3.1 特殊教室 (ignoreWifi = true)

以下教室**无需WiFi验证**即可签到:

| 教室 | filterStrength | 说明 |
|------|----------------|------|
| DA05 | 3 | 无需WiFi |
| DA07 | 3 | 无需WiFi |
| NB03 | 3 | 无需WiFi |
| NOLOC | 1 | 无固定地点 |
| ONLINE | 1 | 在线课程 |

### 3.2 所有教室列表

```
查询: curl "https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/rooms"
```

#### A楼 (Building A)
| 教室 | filterStrength |
|------|----------------|
| AMPTHEA | 3 |

#### B楼 (Building B)
| 教室 | filterStrength |
|------|----------------|
| B1A24 | 3 |
| B1A30 | 3 |
| B1B26 | 3 |
| B1C24 | 3 |
| B1C24A | 3 |
| B1C26 | 3 |
| B1C29 | 3 |
| BA05 | 3 |
| BA06 | 3 |
| BA07 | 3 |
| BA10 | 3 |
| BA18 | 3 |
| BA21 | 3 |
| BA42 | 3 |
| BA64 | 3 |
| BB01 | 3 |
| BB80 | 3 |

#### C楼 (Building C)
| 教室 | filterStrength |
|------|----------------|
| C1A02 | 3 |
| C1A09 | 3 |
| C1B01 | 3 |
| C1B07 | 3 |
| CA42 | 3 |
| CA43A | 3 |
| CA43B | 3 |
| CA45 | 3 |
| CA48A | 3 |
| CA48B | 3 |
| CA48C | 3 |
| CB27 | 3 |
| CB28 | 3 |
| CB29 | 3 |
| CB31 | 3 |
| CB35 | 3 |

#### D楼 (Building D)
| 教室 | filterStrength | ignoreWifi |
|------|----------------|------------|
| DA05 | 3 | **true** |
| DA06 | 3 | - |
| DA07 | 3 | **true** |
| DA08 | 3 | - |
| DA18 | 3 | - |
| DA19 | 3 | - |
| DA20-21 | 3 | - |
| DA22 | 3 | - |
| DB38 | 3 | - |
| DLG01 | 3 | - |
| DLG02 | 3 | - |
| DLG03 | 3 | - |
| DLG04 | 3 | - |
| DLG05 | 3 | - |

#### E楼 (Building E)
| 教室 | filterStrength |
|------|----------------|
| EA21 | 3 |
| EA22 | 3 |
| EA23 | 3 |
| EA28 | 3 |
| EA29 | 3 |
| EA51 | 3 |
| EA53 | 3 |

#### F1楼
| 教室 | filterStrength |
|------|----------------|
| F1A02 | 3 |
| F1A03 | 3 |
| F1A09 | 3 |
| F1A10 | 3 |
| F1A11 | 3 |
| F1A13 | 3 |
| F1A15 | 3 |
| F1A22 | 3 |
| F1A23 | 3 |
| F1A24 | 3 |

#### F3楼
| 教室 | filterStrength |
|------|----------------|
| F3A03 | 3 |
| F3A04 | 3 |
| F3A08 | 3 |
| F3A12 | 3 |
| F3B03 | 3 |
| F3B04 | 3 |
| F3B06 | 3 |
| F3B08 | 4 |
| F3B09 | 4 |
| F3C03 | 3 |
| F3C04 | 3 |
| F3C06 | 3 |
| F3C07 | 3 |
| F3C09 | 3 |

#### F4楼
| 教室 | filterStrength |
|------|----------------|
| F4A07 | 3 |
| F4A10 | 3 |
| F4A11 | 3 |
| F4B05 | 3 |
| F4B06 | 3 |
| F4B09A | 3 |
| F4B09B | 3 |
| F4B10 | 3 |
| F4C06 | 3 |
| F4C07 | 3 |
| F4C10 | 3 |
| F4C11 | 3 |
| F4LG06 | 3 |
| F4LG07 | 3 |
| F4LG10 | 3 |

#### TCR (Teaching Computer Room)
| 教室 | filterStrength |
|------|----------------|
| TCR1 | 3 |
| TCR2 | 3 |
| TCR3 | 3 |
| TCR4 | 3 |

#### 其他
| 教室 | filterStrength | ignoreWifi |
|------|----------------|------------|
| GD14 | 3 | - |
| GREAT-HALL | 3 | - |
| HB05 | 3 | - |
| JH2B02 | 3 | - |
| JH2B02A | 3 | - |
| JH2B02C | 3 | - |
| KL07-16 | 3 | - |
| NA16 | 3 | - |
| NA16B | 3 | - |
| NB03 | 3 | **true** |
| NB10 | 3 | - |
| NB12 | 3 | - |
| NLG02 | 3 | - |
| NOLOC | 1 | **true** |
| ONLINE | 1 | **true** |
| PSY-LAB | 3 | - |

---

## 4. API使用示例

### 4.1 获取服务器时间
```bash
curl -s "https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/global/time"
```

### 4.2 获取当前所有解锁课程
```bash
curl -s "https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/ongoingClasses"
```

### 4.3 获取特定教室信息
```bash
curl -s "https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/rooms/F3C04"
```

### 4.4 获取SSID白名单
```bash
curl -s "https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/global/ssidFilters"
```

---

## 5. 监控脚本示例 (Python)

```python
import requests
import time
import json

def get_ongoing_classes():
    url = "https://firestore.googleapis.com/v1/projects/instatt-3c80d/databases/(default)/documents/ongoingClasses"
    response = requests.get(url)
    data = response.json()

    classes = []
    if 'documents' in data:
        for doc in data['documents']:
            fields = doc['fields']
            classes.append({
                'module': fields.get('moduleKey', {}).get('stringValue', ''),
                'name': fields.get('moduleName', {}).get('stringValue', ''),
                'venue': fields.get('venue', {}).get('stringValue', ''),
                'startTime': fields.get('startTime', {}).get('integerValue', ''),
                'endTime': fields.get('endTime', {}).get('integerValue', ''),
                'lockStatus': fields.get('lockStatus', {}).get('integerValue', ''),
                'attended': fields.get('studentsAttended', {}).get('integerValue', ''),
                'teacher': fields.get('unlockUserName', {}).get('stringValue', '')
            })
    return classes

def monitor_course(target_module):
    """监控特定课程是否解锁"""
    print(f"Monitoring for: {target_module}")
    while True:
        classes = get_ongoing_classes()
        for c in classes:
            if target_module in c['module']:
                print(f"UNLOCKED! {c['name']} at {c['venue']}")
                print(f"Time: {c['startTime']} - {c['endTime']}")
                print(f"Teacher: {c['teacher']}")
                return c
        time.sleep(10)  # 每10秒检查一次

# 使用示例
# monitor_course("COMP")  # 监控所有COMP开头的课程
```

---

## 6. 签到请求格式 (参考)

虽然无法直接离线签到（需要认证），但签到请求格式如下:

**URL**: `https://us-central1-instatt-3c80d.cloudfunctions.net/signAttendance`

**Headers**:
```
Authorization: Bearer {Firebase ID Token}
Firebase-Instance-ID-Token: {FCM Token}
Content-Type: application/json
```

**Body**:
```json
{
    "data": {
        "module": "COMP1001_FML_25-26",
        "venue": "F3C04",
        "courseType": "FML",
        "courseYear": "25-26",
        "classDate": 20251126,
        "startTime": 1400,
        "macAdd": "a0:0f:37:e0:3c:2c",
        "studentId": "学生ID",
        "deviceUID": "设备唯一ID"
    }
}
```

**响应代码**:
- 200: 签到成功
- 403: 设备不在教室范围内 (BSSID验证失败)
- 409: 设备未注册

---

## 7. 安全漏洞总结

1. **`ongoingClasses`集合完全公开** - 任何人都可以实时查看所有解锁的课程
2. **`global`配置公开** - 暴露系统配置、SSID白名单等
3. **`rooms`配置公开** - 暴露所有教室信息，包括哪些教室可以绕过WiFi验证

---

*文档生成时间: 2025-11-26*

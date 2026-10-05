# AutoSign · InstAtt 自动签到服务

> 语言:[English](README.md) · **简体中文**

面向诺丁汉大学马来西亚校区考勤系统 **InstAtt** 的自动签到服务。学生用学校账号登录本站、勾选要挂机的课程;服务端持续监听上游「已解锁课程」列表,老师一解锁就替其调用上游签到接口,按**成功签到次数**扣费。

- 后端:Rust(Axum + Tokio + sqlx)+ PostgreSQL,单二进制,自带 SPA 静态托管
- 前端:React 18 + Vite + TypeScript 单页应用
- 配套:Windows 教室 BSSID 采集工具、无头浏览器辅助登录 worker、上游 App 逆向资料

> 本仓库 crate 名为 `instatt_saas`,线上面板标题为「Instatt 自动签到」。

> **⚠️ 声明:本项目仅供安全研究与学习交流。** 文中涉及的上游接口、配置常量与数据,均来自对公开发行的 InstAtt APK 的逆向分析(见 [10. 逆向来源、安全分析与加固建议](#10-逆向来源安全分析与加固建议))。请勿用于实际代签或任何违反校规、校方服务条款与当地法律的用途;由此产生的一切后果由使用者自负。

---

## 目录

1. [功能一览](#1-功能一览)
2. [工作原理](#2-工作原理)
3. [仓库结构](#3-仓库结构)
4. [本地开发](#4-本地开发)
5. [配置项](#5-配置项)
6. [部署](#6-部署)
7. [HTTP API](#7-http-api)
8. [数据模型](#8-数据模型)
9. [附属工具与目录](#9-附属工具与目录)
10. [逆向来源、安全分析与加固建议](#10-逆向来源安全分析与加固建议)
11. [已知限制与注意事项](#11-已知限制与注意事项)

---

## 1. 功能一览

### 用户端(学校账号登录)

- **登录即注册**:首次登录自动建账号并赠送 400 分(¥4.00)余额,写一条 `register_bonus` 流水;再次登录只更新凭据,不重复赠送。
- **仪表盘**:余额、账号状态、按课程勾选自动签(每门课独立开关)、「立即同步」重新拉取课表与设备信息、当前支持的教室列表。
- **课程表**:按日查看课程,综合显示上游课表、官方考勤、实时解锁状态和本系统的签到结果。
- **签到记录 / 余额流水**:分页展示,每页 20 条。

### 管理端(admin / superadmin)

- **统计看板**:总账号、活跃账号、今日成功签到数、今日营收。
- **账号管理**:搜索、手动充值(写流水)、启用/停用、查看详情(资料、当日课表、签到流水)、代为修改课程勾选与挂机开关。
- **仅超管**:任命/撤销 admin、修改引擎参数(单价、轮询间隔、并发、抖动)、修改超管密码。

### 签到引擎

- **共享轮询器**:每 N 秒拉一次公开的 `ongoingClasses`,一次请求服务全员,与用户数无关。
- **Firestore gRPC 实时监听**:新解锁即唤醒轮询器,延迟压到亚秒级;断线自动退避重连,期间轮询照常兜底。
- **有界并发 worker 池**:全局信号量限流 + 按账号串行 + 随机抖动,摊开瞬时爆发。
- **幂等计费**:唯一约束防重签,单语句条件扣费防超扣,只对上游首次成功(200)收费。
- **两层 token 续期**:签到前按需刷新短 token(缓存 10 分钟、single-flight 去重),每 24 小时跑完整链保活长 token。

---

## 2. 工作原理

### 2.1 认证链路

上游认证是 Azure AD → Firebase 两段式,登录时跑完整链并建号(`src/auth/device.rs`):

```
学校账号(Azure AD)
  → Azure access / refresh token
  → Graph /me                       取邮箱前缀作 account_name、employeeId 作学号
  → 云函数 userLogin                 Azure access → Firebase custom token
  → signInWithCustomToken           → Firebase ID token + Firebase refresh token
  → Firestore students/{学号}        deviceUID、专业
  → Firestore students/{学号}/modules 当前学年课程;上游尚无当前学年时退回最新学年
  → upsert accounts                 按 account_name;本次没取到设备/课程时保留旧值
```

后端提供三种拿到 Azure token 的方式:

| 方式 | 接口 | 说明 |
|---|---|---|
| **辅助登录**(当前登录页使用) | `POST /api/auth/assisted/start` → `GET /api/auth/assisted/poll` | 服务端拉起 Playwright 无头浏览器 worker(`tools/assisted_login.py`)替用户走授权码登录,把 MFA「数字匹配」的数字回传给页面;用 App 自带的 `https://localhost` 回调截获授权码。密码只经 stdin 传给 worker,不落盘、不进 argv |
| **账号密码直登**(ROPC) | `POST /api/auth/password/login` | 服务端直接用账号密码换 token。账号可只填前缀,自动补 `@nottingham.edu.my`。**开了 MFA 的账号会失败** |
| **设备码**(Device Code) | `POST /api/auth/device/start` → `GET /api/auth/device/poll` | 标准微软设备码流程,会话落 `device_code_sessions`。代码注释记录该流程已被租户封禁(AADSTS 7000218),登录页因此改用辅助登录 |

登录成功后签发 HS256 JWT,放在 `session` cookie 里(HttpOnly、SameSite=Strict、30 天;`COOKIE_SECURE=true` 时加 Secure)。每次请求都从数据库重读角色,提权/降权立即生效。

超管使用独立的 `super_admin` 表(argon2 哈希),从 `/manage` 页面登录;超管没有个人签到账号,登录后直接进管理看板。

### 2.2 自动签到流程

```
                 ┌────────────────────────────────────────┐
                 │  Firestore(上游,公开可读)              │
                 │  ongoingClasses = 当前已解锁的课程        │
                 └──────────┬──────────────┬──────────────┘
          gRPC Listen(实时) │              │ REST(每 poll_interval_sec)
                            ▼              ▼
                    ┌──────────────┐  ┌──────────────┐
                    │  listener    │─▶│   poller     │  wake.notify → 立即跑一轮
                    └──────────────┘  └──────┬───────┘
                                             │ 对每个 (解锁课程 × 账号) 判断:
                                             │  • 账号 active 且余额 ≥ 单价
                                             │  • enabled_modules 含该课程代码
                                             │  • 24h 内无 sign_records、不在 inflight
                                             ▼
                                    ┌──────────────────┐
                                    │  worker job      │  抖动 → 全局信号量 → 按账号互斥
                                    │  1 解密 deviceUID │  缺失 → failed(missing_device_uid)
                                    │  2 教室 → BSSID  │  查不到 → failed(missing_bssid)
                                    │  3 取短 token     │  TokenManager
                                    │  4 signAttendance │  网络错误内联重试 3 次
                                    └────────┬─────────┘
                                             ▼
                                    ┌──────────────────┐
                                    │  billing.commit  │  同一事务内:
                                    │  200 → ok_200    │   INSERT sign_records ON CONFLICT DO NOTHING
                                    │        + 扣费     │   UPDATE balance WHERE balance ≥ price
                                    │  202 → already   │   INSERT balance_transactions(sign_charge)
                                    │  其他 → failed   │
                                    └──────────────────┘
```

签到请求发给上游云函数 `signAttendance`,字段为 `moduleID / venue / courseType / courseYear / classDate / startTime / MACaddress / studentID / deviceUID`。`MACaddress` 填教室对应的 Wi-Fi BSSID;`courseType` / `courseYear` 从 `moduleKey`(如 `COMP4082_AUM_26-27`)拆出。

签到结果与计费:

| `result` | 含义 | 扣费 |
|---|---|---|
| `ok_200` | 上游首次成功 | 扣 `price_per_sign_cents` |
| `already_202` | 上游提示之前已签 | 不扣 |
| `failed` | 见 `detail`:`missing_bssid:<教室>`(教室不支持)、`missing_device_uid`(设备未注册)、`status_<code>:<body>`(上游拒绝) | 不扣 |

余额不足或账号非 active 的账号在轮询阶段就被过滤,不产生记录;余额回升后下一轮自然可签。极端竞态下「已签但扣费失败」会把 `detail` 标为 `signed_but_insufficient_balance`。网络重试耗尽时不写记录,留待下一轮。

### 2.3 Token 续期

| Token | 寿命 | 用途 |
|---|---|---|
| Firebase ID token(短) | 约 1 小时 | 查课表、签到 |
| Firebase refresh token(长) | 很长 | 便宜地再生成短 token |
| Azure refresh token(长) | 约 90 天,每用一次轮换 | Firebase RT 失效时兜底,重走完整链 |

- **按需刷新**(`src/engine/tokens.rs`):短 token 进程内缓存 10 分钟;未命中时按账号 single-flight 刷新,先试 Firebase RT,失败再走 Azure 完整链并把新的 Firebase RT 加密回写。
- **每日刷新**(`src/engine/refresher.rs`):每 24 小时对全部 active 账号跑一次完整链,保活并轮换长 token。
- 两层都失败 → 账号置 `needs_relogin`,面板顶部红条提示,引擎跳过该账号直到用户重新登录。

### 2.4 课程表与状态判定

`GET /api/me/classes?date=YYYYMMDD` 用 Firestore `runQuery` 拉 `students/{学号}/classes` 中该日的课,叠加当天的 `ongoingClasses` 与本地 `sign_records`,给每节课一个综合状态:

| 状态 | 含义 |
|---|---|
| `cancelled` | 上游已取消 |
| `attended` | 官方考勤已签 |
| `unlocked` | 此刻已解锁、尚未签 |
| `absent` | 已上课(conducted)但未签 |
| `upcoming` / `locked` / `not_unlocked` | 待上且未解锁,分别为未开始 / 上课时段内 / 已过时段 |

时间一律按校区时区(UTC+8)计算;学年按 9 月切换,格式与上游 `courseYear` 一致(2026-09-30 → `26-27`)。

### 2.5 教室与 BSSID

上游**服务端**按教室的合法 BSSID 名单校验签到请求里提交的 `MACaddress`:名单存在 Firestore、客户端读不到,匹配才算到场(云函数返回 200),否则 403。因此本系统**只能签已采集 BSSID 的教室**(`src/engine/instatt.rs` 中的 `VENUE_BSSIDS`,目前 22 条,部分相邻教室共用同一 AP),以及上游标记 `ignoreWifi=true` 的 5 个教室(`DA05 / DA07 / NB03 / NOLOC / ONLINE`,任意 BSSID 可签)。其他教室会记一条 `failed / missing_bssid`,不扣费。

新教室用 `tools/wifi-bssid` 采集后追加到 `VENUE_BSSIDS` 即可;面板「支持的教室」与 `GET /api/venues` 都从这张表读。

### 2.6 信息来源:逆向 InstAtt APK

上面用到的全部上游细节 —— Firebase 项目与 API key、Azure 租户/客户端 ID、各云函数名(`signAttendance`、`userLogin`、`unlock` …)、`signAttendance` 的请求字段、以及客户端如何上报所连 BSSID(`MACaddress`)与 `ignoreWifi` 处理 —— 都来自对公开发行的 InstAtt 安卓 APK(`instatt.instatt`,v1.43)的反编译,留存在本仓库 `app/` 目录。随后用从 APK 取得的配置,直接查询上游公开可读的 Firestore 集合,整理成 [`InstAtt_Database_Info.md`](InstAtt_Database_Info.zh-CN.md)。

注意:教室 BSSID **不在** APK 里;校验用的合法 BSSID 名单在服务端(存于 Firestore,但学生 token 读不到、也不在公开的 `rooms` 文档里 —— `rooms` 只有 `filterStrength` 与 `ignoreWifi`),所以只能到各教室现场用 `tools/wifi-bssid` 实测采集。该 APK 为发布版但未做任何混淆或加固,逆向门槛极低,详见 [10. 逆向来源、安全分析与加固建议](#10-逆向来源安全分析与加固建议)。

---

## 3. 仓库结构

```
.
├─ Cargo.toml / src/                 Rust 后端(crate: instatt_saas)
│  ├─ main.rs                        启动:读 .env → 连库迁移 → seed → 起引擎 → 起 HTTP
│  ├─ config.rs                      环境变量(缺失即 panic)
│  ├─ state.rs / models.rs           共享状态、sqlx 行模型
│  ├─ crypto.rs                      AES-256-GCM 字段加密(12 字节 nonce ‖ 密文)
│  ├─ db/                            连接池 + 嵌入式迁移;seed 超管与默认配置
│  ├─ auth/                          device(设备码 / ROPC / 建号链)、assisted(辅助登录)、
│  │                                 jwt、password(argon2)、middleware(角色提取器)
│  ├─ api/                           路由装配、auth_routes、me、classes、admin
│  └─ engine/                        instatt(上游客户端)、listener、poller、worker、
│                                    billing、tokens、refresher
├─ migrations/                       sqlx 迁移:0001 初始 schema,0002 每课开关 + 专业,0003 课程名映射
├─ web/                              React + Vite 前端;构建产物 web/dist 由后端托管
│  └─ src/pages/                     Login、AdminLogin、Dashboard、Classes、Records、Transactions、admin/*
├─ deploy/                           systemd unit、nginx 配置、服务器部署说明
├─ scripts/                          sync.py(打包上传源码到服务器)、rexec.py(远程执行命令)
├─ tools/
│  ├─ assisted_login.py              辅助登录 worker(Playwright,服务器上运行)
│  └─ wifi-bssid/                    教室 BSSID 采集工具(Rust,Windows;附 Python 版)
├─ docs/superpowers/                 2026-06-05 的设计文档与实现计划
├─ InstAtt_Database_Info.md          上游 Firestore 公开数据整理(集合权限、字段含义、签到请求格式)
├─ app/ + build.gradle + settings.gradle
│                                    上游 InstAtt Android App(v1.43)反编译工程,仅供查阅
├─ MyXposed/                         Xposed / LSPosed 模块(独立仓库 JokerEzreal/InstAttPlugin),在手机端 hook 上游 App
├─ docker-compose.dev.yml            本地开发用 PostgreSQL 16
└─ .env.example                      环境变量模板
```

---

## 4. 本地开发

### 前置

- Rust stable(edition 2021)、Node.js 18+、Docker(或任意可用的 PostgreSQL 16)
- 上游均为公网服务,开发机需能访问 `login.microsoftonline.com`、`graph.microsoft.com`、`*.googleapis.com`、`*.cloudfunctions.net`

### 步骤

```bash
# 1. 数据库
docker compose -f docker-compose.dev.yml up -d
#    → postgres:16,连接串 postgres://instatt:instatt@localhost:5432/instatt

# 2. 配置
cp .env.example .env
#    按需改 JWT_SECRET;ENCRYPTION_KEY 必须正好 32 字节

# 3. 后端(启动时自动跑迁移,并 seed 超管与默认 system_config)
cargo run
#    listening on 0.0.0.0:8080;GET /api/health → {"status":"ok"}
#    注意:引擎随进程启动,会立刻开始轮询真实上游的 ongoingClasses

# 4. 前端开发服务器(/api 代理到 :8080)
cd web && npm install && npm run dev

# 生产构建:npm run build → web/dist;后端 fallback 直接托管,不需要单独的静态服务器
```

### 测试

```bash
# 需要 DATABASE_URL 指向可用的 PostgreSQL:
# #[sqlx::test] 会为每个测试建临时库并跑迁移;上游全部用 wiremock 模拟
cargo test
```

测试覆盖的点:加密往返、JWT 签发/过期、argon2、cookie 解析、课程状态矩阵、学年与时区换算、上游响应解析、token single-flight 与降级链、幂等计费与并发不超扣、50 个账号同一节课并发端到端、注册赠送只发一次、seed 幂等。

---

## 5. 配置项

### 环境变量

| 变量 | 必填 | 默认 | 说明 |
|---|---|---|---|
| `DATABASE_URL` | 是 | | PostgreSQL 连接串 |
| `JWT_SECRET` | 是 | | 会话签名密钥 |
| `ENCRYPTION_KEY` | 是 | | AES-256-GCM 密钥,**必须正好 32 字节**;换了就解不开库里已有的 token 与设备号 |
| `SUPERADMIN_USERNAME` / `SUPERADMIN_PASSWORD` | 是 | | 首次启动 seed 超管;用户名已存在则不覆盖,之后在面板改密 |
| `BIND_ADDR` | 否 | `0.0.0.0:8080` | 监听地址;走 nginx 时建议 `127.0.0.1:8080` |
| `COOKIE_SECURE` | 否 | `false` | HTTPS 下设 `true`,cookie 加 Secure |
| `WEB_DIR` | 否 | `web/dist` | 前端构建产物目录 |
| `RUST_LOG` | 否 | `instatt_saas=info` | 日志过滤 |

开发期从当前目录的 `.env` 读取(内置极简解析,不覆盖已有环境变量);生产由 systemd `EnvironmentFile` 注入。

### system_config(运行期参数,存库)

| key | seed 默认 | 说明 |
|---|---|---|
| `price_per_sign_cents` | `100` | 每次成功签到扣费(分) |
| `poll_interval_sec` | `5` | 轮询 `ongoingClasses` 的间隔 |
| `max_concurrency` | `50` | 同时打到上游的签到请求上限 |
| `jitter_ms_max` | `1500` | 每个签到 job 前的随机延迟上限 |
| `realtime_listen` | 不 seed,缺省按 `1` | 置 `0` 关闭 Firestore 实时监听,只剩定时轮询 |

超管在「系统配置」页修改,或 `PATCH /api/admin/config`。**引擎只在启动时读取这些值,改完需要重启服务。**

---

## 6. 部署

生产形态:Ubuntu 24.04,源码在 `/opt/instatt_saas`,单个 release 二进制 + 本机 PostgreSQL,systemd 守护,nginx 做 80/443 入口反代到 `127.0.0.1:8080`。完整步骤(环境变量、systemd、nginx、certbot、每日 pg_dump)见 [`deploy/README.md`](deploy/README.md)。

```bash
# 本地:把源码打包上传到服务器
#   排除 .git / target / node_modules / dist,以及 .env / .deploy.env / instatt_tokens.json
python scripts/sync.py

# 本地:在服务器上执行命令(自动 cd 到 REMOTE_DIR)
python scripts/rexec.py "cargo build --release && cd web && npm run build && cd .. && systemctl restart instatt"
```

两个脚本依赖 `paramiko`,连接信息读自仓库根目录的 `.deploy.env`(`HOST / USER / PASS / REMOTE_DIR`,已 gitignore)。

### 辅助登录的服务器依赖

辅助登录在服务端拉起无头浏览器,路径写死在 `src/auth/assisted.rs` 与 `tools/assisted_login.py`:

- Python 虚拟环境 `/opt/pwlogin`,需安装 `playwright`(含 chromium)与 `cryptography`
- worker 脚本 `/opt/instatt_saas/tools/assisted_login.py`,从 `/opt/instatt_saas/.env` 读 `ENCRYPTION_KEY`
- 状态目录 `/tmp/assisted/`,每次登录一个 `<session_id>.json`,成功或失败后由后端删除
- 并发上限 5 个 worker;等待用户在 Authenticator 批准最长 180 秒,前端轮询最长 200 秒

没有这套环境时 `/api/auth/assisted/start` 返回 503,设备码与 ROPC 两条接口不受影响。

---

## 7. HTTP API

所有接口返回 JSON,错误统一为 `{"error": "..."}`。分页参数 `page` 从 0 起,每页 20 条。权限列中「用户」指有个人签到账号的登录用户,「登录」含超管。

| 方法 路径 | 权限 | 说明 |
|---|---|---|
| `GET /api/health` | 公开 | 存活检查 |
| `POST /api/auth/assisted/start` | 公开 | `{username, password}` → `{session_id}` |
| `GET /api/auth/assisted/poll?id=` | 公开 | `pending` / `mfa{number}` / `done`(下发 cookie)/ `failed{error}` |
| `POST /api/auth/password/login` | 公开 | `{username, password}`,ROPC 直登 |
| `POST /api/auth/device/start` | 公开 | → `{session_id, user_code, verification_uri, interval, expires_in}` |
| `GET /api/auth/device/poll?id=` | 公开 | `pending` / `declined` / `expired` / `done` |
| `POST /api/auth/admin/login` | 公开 | 超管账号密码 |
| `POST /api/auth/logout` | 公开 | 清 cookie |
| `GET /api/me` | 登录 | 角色 + 账号资料;超管的 `account` 为 `null` |
| `PATCH /api/me/auto-sign` | 用户 | `{auto_sign}` 挂机总开关 |
| `PATCH /api/me/modules` | 用户 | `{enabled: [...]}`,必须是自己课表的子集 |
| `POST /api/me/sync` | 用户 | 重新拉设备号、专业、课表;新增课程默认开启自动签 |
| `GET /api/me/classes?date=` | 用户 | 某日课表综合视图,缺省为校区今天 |
| `GET /api/me/records?page=` | 用户 | 签到记录 |
| `GET /api/me/transactions?page=` | 用户 | 余额流水 |
| `GET /api/venues` | 登录 | 支持的教室列表 |
| `GET /api/admin/accounts?search=&page=` | admin | 账号列表,按 `account_name` 模糊搜索 |
| `POST /api/admin/accounts/:id/topup` | admin | `{amount_cents, note}`,写 `topup` 流水 |
| `PATCH /api/admin/accounts/:id/status` | admin | `active` / `disabled` |
| `GET /api/admin/accounts/:id/detail` | admin | 账号详情 |
| `GET /api/admin/accounts/:id/classes?date=` | admin | 该账号某日课表 |
| `GET /api/admin/accounts/:id/records?page=` | admin | 该账号签到记录 |
| `PATCH /api/admin/accounts/:id/modules` | admin | 代改课程勾选 |
| `PATCH /api/admin/accounts/:id/auto-sign` | admin | 代改挂机开关 |
| `GET /api/admin/stats` | admin | 总账号 / 活跃 / 今日成功签到 / 今日营收 |
| `GET /api/admin/config` | admin | 读 system_config |
| `PATCH /api/admin/config` | superadmin | 写 system_config(重启生效) |
| `PATCH /api/admin/accounts/:id/role` | superadmin | `user` / `admin` |
| `POST /api/admin/superadmin/password` | superadmin | `{old, new}`,新密码至少 6 位 |

非 `/api` 路径一律回退到 `WEB_DIR/index.html`,以支持前端路由。

---

## 8. 数据模型

金额一律为整数「分」(`BIGINT`);`device_uid / azure_rt / firebase_rt` 以 AES-256-GCM 密文(`BYTEA`)存储。

| 表 | 作用 | 关键字段 |
|---|---|---|
| `accounts` | 学校账号即用户,1:1 | `account_name` 唯一;`student_id`;`my_modules`(课表)、`enabled_modules`(已勾选自动签,子集)、`module_info`(代码 → 课程名);`balance_cents`;`auto_sign`;`role`(`user` / `admin`);`status`(`active` / `needs_relogin` / `disabled`);`course`(专业) |
| `super_admin` | 内置超管 | `username`、argon2 `password_hash`、`role='superadmin'` |
| `device_code_sessions` | 设备码登录会话 | `device_code`、`user_code`、`status`(`pending` / `done` / `expired` / `error`)、`result_account_name` |
| `sign_records` | 签到结果,也是账单凭证 | `result`、`charged_cents`、`detail`;**唯一约束 `(account_id, module_key, class_date, start_time)`** 是幂等的最终保证 |
| `balance_transactions` | 余额流水,只增不改 | `amount_cents`(充值正、扣费负)、`type`(`topup` / `sign_charge` / `register_bonus` 等)、`ref_id`(关联 sign_record)、`balance_after`、`operator` |
| `system_config` | 运行期参数 | `key` / `value` |

迁移文件在 `migrations/`,启动时由 `sqlx::migrate!` 自动执行。

---

## 9. 附属工具与目录

### tools/wifi-bssid

Windows 下记录当前所连 Wi-Fi BSSID 的小工具,零依赖(只调 `netsh wlan`),用于让同学在教室里采集 AP 地址。输出 `bssid_records.csv`,并打印可直接粘贴进 `VENUE_BSSIDS` 的一行。附 Python 版应对 Windows 11 24H2 的定位权限问题。用法与常见问题见 [`tools/wifi-bssid/README.md`](tools/wifi-bssid/README.md)。

### tools/assisted_login.py

辅助登录 worker,见 [6. 部署](#6-部署)。stdin 读一行 JSON,输出状态文件;拿到的 Azure token 用服务器 `ENCRYPTION_KEY` 加密后再写盘。

### app/(含根目录的 build.gradle / settings.gradle)

上游 InstAtt Android App(包名 `instatt.instatt`,版本 1.43)的反编译工程,约 9 千个文件,用于确认上游接口字段、Firebase 配置与 Wi-Fi 校验逻辑。不参与本服务构建。

### MyXposed/

独立的 Xposed / LSPosed 模块,位于独立仓库 [JokerEzreal/InstAttPlugin](https://github.com/JokerEzreal/InstAttPlugin),运行在已 root 的手机上 hook 上游 App:检测课程解锁后自动点签到、伪造 BSSID、强制 `ignoreWifi`。与服务端方案互不依赖,是另一条「端侧」实现路线。文档见 `MyXposed/README.md`、`FEATURES.md`、`USAGE_GUIDE.md`。

### InstAtt_Database_Info.md

上游 Firestore 公开数据整理:哪些集合无需认证可读(`global/*`、`rooms/*`、`ongoingClasses`)、各字段含义、课型与状态编码、`signAttendance` 请求格式与返回码。`src/engine/instatt.rs` 的常量与解析逻辑以此为依据。

### docs/superpowers/

2026-06-05 的 SaaS 化设计文档与分阶段实现计划。现状与文档有出入:登录方式已从单一设备码扩展为三种;新增了 Firestore 实时监听、每课独立开关、课程表页和注册赠送;文档中提到的 CSRF token 未实现,目前靠 `SameSite=Strict` cookie。

### 本地保留、不入库的文件

`.gitignore` 排除了 `.env`、`.deploy.env`(服务器口令)、`instatt_tokens.json`(真实用户 refresh token),以及服务端前身的独立 Python 脚本 `auto_sign_daemon*.py` / `offline_sign*.py`。`src/engine/instatt.rs` 就是从这两个脚本移植的。

---

## 10. 逆向来源、安全分析与加固建议

> 本节均为对**公开发行、可自由下载安装**的 InstAtt APK 的静态分析结论,目的是交代本项目数据的来历,并从防御角度给出改进建议。未涉及任何对上游服务器的入侵或越权操作。

### 10.1 逆向过程

1. 反编译 InstAtt 安卓 APK(`instatt.instatt`,versionName 1.43),得到可读 Java 源码,留存于 `app/`(`instatt` 包下 109 个业务类)。
2. 从中提取客户端配置与协议:Firebase 项目/密钥、Azure 租户与客户端 ID、云函数名清单、`signAttendance` 等请求的字段结构,以及客户端上报所连 BSSID(`MACaddress`)与 `ignoreWifi` 的处理。
3. 用这些配置直接请求上游**公开可读**的 Firestore 集合(`global/*`、`rooms/*`、`ongoingClasses`),把字段含义、编码表与签到请求格式整理成 [`InstAtt_Database_Info.md`](InstAtt_Database_Info.zh-CN.md)。
4. 把上述逻辑用 Rust 重写为本服务的上游客户端(`src/engine/instatt.rs`)。

### 10.2 发现:APK 未混淆、未加密、未加壳

发布版(`BuildConfig.BUILD_TYPE = "release"`、`DEBUG = false`)直接反编译即得到带原始包名/类名/字段名的 Java,几乎无逆向门槛:

| 观察 | 证据(本仓库 `app/`) |
|---|---|
| 类名、方法名、字段名全部保留,无 `a/b/c` 混淆 | `instatt` 包下 109 个有意义命名的类,如 `LecturerHomeFragment`、`WifiConnectionReceiver`、`FirebaseFunctionName` |
| 云函数名明文硬编码 | `FirebaseFunctionName.java`:`signAttendance` / `userLogin` / `unlock` / `lock` / `createClass` / `modifyAttendanceAdmin` … |
| Firebase 密钥明文 | `res/values/strings.xml` 的 `google_api_key`、`project_id`、`firebase_database_url` |
| Azure 身份常量明文 | `AzureParameters.java`(租户 ID、客户端 ID) |
| 客户端 Wi-Fi 采集与预检逻辑可读、可改 | `GlobalStatic`、`CustomWifi`、`WifiConnectionReceiver` 中读取所连 BSSID 与 `ignoreWifi` 开关(最终校验在服务端) |

「未加密 / 未加壳」是据此推断:DEX 能被直接反编译为带原始标识符的源码、且密钥与端点以明文出现,说明既无字符串加密也无加壳保护(未另跑专门的脱壳检测)。

### 10.3 由此暴露的风险面

- **服务端校验的是客户端自报的 BSSID,可伪造、可重放**:位置校验确实在服务端 —— 云函数 `signAttendance` 把请求里的 `MACaddress` 与 Firestore 中该教室的合法 BSSID 名单比对,匹配才成功(200),否则返回 403「BSSID验证失败」。但它比对的是**客户端自己填进请求的那个值**,服务端无从确认设备真的连着那个 AP。于是只要现场采集到某教室的一个合法 BSSID(名单客户端读不到,但到场连一次网即可得),之后便能在任意网络、任意位置把它填进请求重放,稳定通过校验 —— 本项目与 `MyXposed/` 模块都建立在这一点上。
- **凭据与端点全暴露**:Firebase API key、项目 ID、Azure 租户/客户端 ID 明文可取,配合登录流程即可在校外完成完整认证链。
- **公开可读的 Firestore**:`ongoingClasses` 实时暴露全校哪些课已解锁、各教室 `ignoreWifi` 配置等,无需认证即可抓取(见 [`InstAtt_Database_Info.md`](InstAtt_Database_Info.zh-CN.md) 第 7 节「安全漏洞总结」)。
- **端侧无完整性保护**:无混淆、无 root/hook 检测、无证书绑定,Xposed 一类运行时插桩可随意改写 `ignoreWifi`、伪造 BSSID、自动点击签到。

### 10.4 给上游的加固建议

1. **不要把客户端自报的 BSSID 当作到场证明**:校验虽在服务端,但它信的是请求里由客户端填写的 `MACaddress`。应改用服务端能独立观测、客户端无法伪造的信号(如老师端一次性解锁随机数、服务端侧观测到的接入网络、与课节绑定的限时 nonce),而不是让学生设备自报连了哪个 AP。
2. **收紧 Firestore 安全规则**:按最小权限暴露,`ongoingClasses` / `rooms` / `global` 不应对未认证客户端整体开放。
3. **启用代码混淆与字符串加密**:R8/ProGuard(或商用加固)+ 资源/字符串加密,显著抬高逆向成本。
4. **加运行时完整性与反注入**:root / 模拟器 / 调试器 / Xposed 检测、证书绑定(certificate pinning)、防抓包与重打包签名校验。
5. **接入 Firebase App Check**,并对 API key 做来源/应用限制,阻断脱离正规 App 的直接调用。
6. **服务端风控**:对同一 `deviceUID` 复用、异常签到频率、地理/网络异常做检测与限流。

> 这些建议针对的是上游「**信任客户端上报**」这一根因;一旦落地,本服务的签到路径即失效。

### 10.5 仅供学习与免责

本项目(含服务端、`MyXposed/` 模块与各逆向资料)**仅用于安全研究、协议分析与学习交流**,用以展示「服务端轻信客户端自报数据(如所连 BSSID)」这一经典问题。请勿用于真实代签,或任何违反校规、校方服务条款及当地法律的场景。将其用于生产即是替学生向校方考勤系统提交虚假出勤,风险与责任由运营者和使用者自行承担,作者与本仓库不对由此产生的任何后果负责。

**侵权与下架联系**:若本仓库内容涉及侵权或其他权益问题,请邮件联系 [fs840594947@gmail.com](mailto:fs840594947@gmail.com),我会尽快处理(删除或下架相关内容)。

---

## 11. 已知限制与注意事项

- **教室覆盖有限**:只有 `VENUE_BSSIDS` 与免 Wi-Fi 教室可签,其余教室记失败、不扣费;需要持续用采集工具补表。
- **引擎参数重启生效**:`system_config` 只在进程启动时读取。
- **每日刷新首轮在启动 24 小时后**:刷新器先 sleep 再跑,重启后前 24 小时只依赖按需刷新。
- **辅助登录绑定服务器路径,仅 Linux**:见 [6. 部署](#6-部署);本地开发机上该接口不可用。
- **ROPC 不支持 MFA**:账号开了 MFA 或条件访问会失败,应走辅助登录。
- **登录页目前只暴露辅助登录**:设备码与 ROPC 接口仍在后端,但 `web/src/pages/Login.tsx` 没有入口。
- **前端单价文案写死**:仪表盘提示「成功一次扣 ¥2.00」,而 seed 默认单价是 100 分;改单价时记得同步 `Dashboard.tsx` 的文案。
- **换 `ENCRYPTION_KEY` 等于清空凭据**:库里所有 refresh token 与设备号都将无法解密,全员需重新登录。
- **安全**:超管初始密码务必在面板尽快修改;`JWT_SECRET`、`ENCRYPTION_KEY`、`.deploy.env` 不要进仓库;生产务必开 HTTPS 并把 `COOKIE_SECURE` 设为 `true`。
- **合规与免责**:本系统替学生提交签到、以实测 BSSID 通过上游位置校验,与校规直接冲突,相关账号可能被处理。本项目仅供学习研究,完整说明见 [10.5 仅供学习与免责](#105-仅供学习与免责)。

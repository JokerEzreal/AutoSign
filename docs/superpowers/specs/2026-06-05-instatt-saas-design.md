# InstAtt 自动签到 SaaS 化设计文档

> 日期: 2026-06-05
> 状态: 已批准,进入实现
> 技术栈: Rust (Axum) + PostgreSQL + React SPA

## 1. 背景与目标

现有 `offline_sign.py` / `auto_sign_daemon.py` 是本地多用户 InstAtt(诺丁汉大学马来西亚校区考勤系统)自动代签工具,数据存本地 `instatt_tokens.json`。

目标:将其改造为跑在自有 Linux 服务器上的完整服务,具备:

- **账号系统**:用户用学校账号(Azure Device Code)登录即用,不注册、不能注册,严格 1:1。
- **余额系统**:按成功签到次数(仅上游返回 200 首次成功)扣费,管理员手动充值。
- **高并发签到引擎**:能扛 ~500 用户同时签到,单机可达上千。

### 现有可复用逻辑

- 认证链路:Azure AD(Device Code)→ Firebase Custom Token → Firebase ID Token,带 refresh token 续期。
- 签到逻辑:轮询公开 `ongoingClasses` → 匹配用户已选课程 → 用 `deviceUID` + 教室 BSSID 调 `signAttendance` 云函数。
- 常量:`FIREBASE_API_KEY`、`FIREBASE_PROJECT_ID`、`AZURE_TENANT_ID`、`AZURE_CLIENT_ID`、`VENUE_BSSID_MAP`、`IGNORE_WIFI_VENUES`。

## 2. 架构总览(方案 A:单体 + 共享轮询 + 有界并发池)

```
[Nginx + TLS]  反代 + 托管 SPA build
      │
      ▼
[Rust 单二进制 (Axum)]  systemd 守护,启动自动跑 DB 迁移
   ├─ HTTP API           服务 SPA:登录/同步/记录/充值/配置
   ├─ 共享轮询器 (1任务)  每 N 秒拉 1 次 ongoingClasses (O(1),与用户数无关)
   ├─ 有界并发 Worker 池  Semaphore 限总并发,按账号串行
   └─ 每日刷新器          保活长 token + 提前发现失效账号
      │
      ▼
[PostgreSQL]  pg_dump 定时备份
```

**为什么扛得住 500+**:轮询是 O(1)(一次请求服务全员);签到爆发是有界并发的异步 I/O(50 并发闸 + 随机抖动),几秒清空;I/O 密集,CPU 几乎空闲。引擎模块边界清晰,将来可拆成独立 binary(方案 B)。

## 3. 数据模型(PostgreSQL)

钱一律用整数「分」(`BIGINT`)。敏感字段(`device_uid`/`azure_rt`/`firebase_rt`)AES-GCM 加密入库,密钥走环境变量 `ENCRYPTION_KEY`(32 字节)。

```
accounts                   学校账号即身份(1:1)
  id              bigserial PK
  account_name    text unique     -- 如 niubi666
  student_id      text            -- device_uid 前 8 位
  device_uid      bytea (enc)
  azure_rt        bytea (enc)
  firebase_rt     bytea (enc)
  my_modules      jsonb           -- ["COMP4082", ...]
  balance_cents   bigint default 0
  auto_sign       bool   default true
  role            text   default 'user'    -- 'user' | 'admin'
  status          text   default 'active'  -- 'active' | 'needs_relogin' | 'disabled'
  last_synced_at / last_refresh_at / created_at / updated_at

super_admin                内置超管(非学校账号,seed)
  id, username unique, password_hash (argon2), role='superadmin', created_at

device_code_sessions       登录/授权临时会话
  id, device_code, user_code, verification_uri, interval_sec,
  expires_at, status('pending'|'done'|'expired'|'error'),
  result_account_name (完成后写入), created_at

sign_records               签到结果 / 账单凭证
  id, account_id
  module_key, module_name, venue, class_date, start_time
  result          text   -- 'ok_200' | 'already_202' | 'failed'
  charged_cents   bigint default 0
  attempt         int     default 0
  detail          text    -- 失败原因 / 状态码
  created_at
  unique(account_id, module_key, class_date, start_time)   -- 幂等防重签

balance_transactions       余额流水(只增不改)
  id, account_id
  amount_cents    bigint  -- 充值正,扣费负
  type            text    -- 'topup' | 'sign_charge' | 'refund' | 'adjust'
  ref_id          bigint  -- sign_record id / 操作者
  balance_after   bigint
  note, operator   text   -- 操作管理员标识
  created_at

system_config              运行期参数(键值)
  key text PK, value text, updated_at
  -- price_per_sign_cents, poll_interval_sec, max_concurrency, jitter_ms_max ...
```

## 4. 签到引擎

### 4.1 共享轮询器(单任务)

每 `poll_interval_sec` 拉一次公开 `ongoingClasses`。内存维护 `HashMap<课程代码, Vec<account_id>>` 索引,从 active 账号的 `my_modules` 构建,账号变更时增量更新。命中且 `sign_records` 无记录的 →（账号未禁用、`auto_sign=true`、余额≥单价)投递签到 job。

### 4.2 有界并发 Worker 池

- **全局信号量**(默认 50):控制同时打到 Firebase 的总请求数。
- **按账号串行**:同一 `account_id` 的多门课排队逐个签;不同账号并发。同时解决扣费竞态与单账号瞬时请求过多/风控。
- 每 job 前 **0–1.5s 随机抖动**,摊开爆发。
- job panic 不影响池(spawn + 捕获)。

### 4.3 单 job 状态机

```
取 job
 ├─ 校验:status=active? auto_sign? 余额 ≥ price?   否 → 记录跳过
 ├─ 解析教室 BSSID(VENUE_BSSID_MAP / IGNORE_WIFI_VENUES)  缺失 → 记 failed
 ├─ 取有效短 token(缓存命中直接用,否则 single-flight 刷新)
 ├─ POST signAttendance(module_code/venue/courseType/courseYear/classDate/startTime/MACaddress/studentID/deviceUID)
 └─ 按返回码:
     200  → 事务{ 原子条件扣费 + sign_record(ok_200,charged) + balance_transaction(sign_charge) }
     202  → sign_record(already_202, charged=0)         不扣费
     其他 → attempt+1,有限次跨轮询退避重试;超限 → sign_record(failed) 不扣费
```

### 4.4 扣费原子性(防并发超扣)

单语句条件扣减,在按账号串行之上的双保险:

```sql
UPDATE accounts SET balance_cents = balance_cents - $price
 WHERE id = $id AND balance_cents >= $price
 RETURNING balance_cents;
```

返回 0 行 = 余额不足,本次不扣(且不应已签 —— pre-check 已挡)。扣费成功后在**同一事务**内写 `sign_records` 与 `balance_transactions`。

### 4.5 幂等防重签

`sign_records` 对 `(account_id, module_key, class_date, start_time)` 唯一约束。内存去重为快路径,唯一约束为跨重启的最终保证(插入冲突即视为已处理)。

## 5. Token 模型与续期

| 类别 | Token | 寿命 | 用途 |
|------|-------|------|------|
| 工作 token(短) | Firebase ID Token | ~1h | 查课程/deviceUID **和**签到(同一个) |
| 续命 token(长) | Firebase Refresh Token | 很长 | 再生产短 token(便宜路径) |
| 续命 token(长) | Azure Refresh Token | ~90 天(用一次轮换重置) | Firebase RT 失效时兜底,Azure→Firebase 重走 |

### 两层续期(才能"永不重新授权")

1. **签到时按需刷新短 token**(`DashMap<account_id, (id_token, expire_at)>` 缓存 10 分钟,single-flight 去重):Firebase ID Token ~1h 过期,必须临签前刷新。只刷有课要签的账号,最省。
2. **每日刷新器**:对所有 active 账号跑一次完整链(Azure RT → Firebase RT),保活+轮换长 token(每天用一次即重置 Azure RT 的 90 天),并提前把失效账号标记 `needs_relogin`。

刷新彻底失败 → 账号置 `needs_relogin`,面板红条提示重新登录。

## 6. 账号 / 权限模型

- **无注册**:Device Code 登录学校账号 = 登录 + 绑定 + 建账号(按 `account_name` upsert)。1 学校账号 = 1 用户 = 1 份余额。
- **登录会话**:Device Code 完成 → 签发 JWT(httponly+Secure+SameSite=Strict cookie),有效期 30 天。每日刷新器保活,正常情况下无需重新授权。
- **三级权限**:
  - `superadmin`:内置固定账号 `admin` / 初始密码 `changeme-strong-pass`(argon2 哈希 seed,不明文硬编码,支持改),账号密码登录。可手动充值、任命/撤销 admin、改系统配置、改超管密码。
  - `admin`:被超管任命的学校账号,Device Code 登录后可见管理视图(手动充值、管理账号状态)。
  - `user`:普通学校账号。

## 7. API 接口

JWT 存 httponly cookie,改动型接口加 CSRF token,`/api/admin/*` 走角色中间件。

**认证**
```
POST /api/auth/device/start      发起 Device Code → user_code + 验证网址 + 轮询句柄
GET  /api/auth/device/poll?id=   轮询;done 时签发 JWT cookie
POST /api/auth/admin/login       超管/管理员账号密码登录
POST /api/auth/logout
```

**用户(登录态)**
```
GET   /api/me                    余额/状态/auto_sign/课程/学号(脱敏)
PATCH /api/me/auto-sign          开关挂机
POST  /api/me/sync               手动同步课程 & deviceUID
GET   /api/me/records?page=      自己的签到记录
GET   /api/me/transactions?page= 自己的余额流水
```

**管理员(admin/superadmin)**
```
GET   /api/admin/accounts?search=&page=   账号列表
POST  /api/admin/accounts/:id/topup       手动充值 {amount_cents, note}
PATCH /api/admin/accounts/:id/status      启用/禁用
GET   /api/admin/accounts/:id/records
GET   /api/admin/stats                    仪表盘统计
```

**仅超管**
```
PATCH /api/admin/accounts/:id/role        设/撤 admin
PATCH /api/admin/config                   改单价/轮询间隔/并发上限
POST  /api/admin/superadmin/password      改超管密码
```

## 8. 前端(React + Vite + TS,独立 SPA)

**用户端**
- 登录页:「用学校账号登录」→ 展示 user_code + 微软网址 + 自动轮询;角落「管理员登录」。
- 仪表盘:余额、账号状态(`needs_relogin` 顶部红条)、挂机开关、已绑课程、学号脱敏、上次同步、「立即同步」。
- 签到记录:表格(课程/教室/时间/结果/扣费),按结果筛选。
- 余额流水:充值/扣费账本。

**管理端(角色显示)**
- 账号管理:可搜索表格 + 充值弹窗 + 启停 +(超管)设角色。
- 系统配置(超管):单价 / 轮询间隔 / 并发上限 / 改超管密码。
- 统计看板:总账号 / 活跃 / 今日签到数 / 今日营收。

## 9. 工程结构

```
src/
  main.rs              启动:DB迁移 + spawn引擎 + 起HTTP
  config.rs            环境变量加载
  db/                  连接池 + queries
  auth/                device code / jwt / argon2 / 中间件
  crypto.rs            AES-GCM 字段加解密
  api/                 路由 handler
  engine/              签到引擎(可独立拆分)
    poller.rs          共享轮询器
    worker.rs          有界并发池 + job 状态机
    tokens.rs          4-token 两层续期 + single-flight
    billing.rs         原子扣费 + 流水
    instatt.rs         Firebase/Azure/签到逻辑(移植自 Python)
  models.rs
migrations/            sqlx 迁移
web/                   React SPA
```

**关键 crate**:`axum`/`tokio`/`tower-http` · `sqlx`(postgres,编译期校验)· `reqwest`(rustls)· `argon2` · `jsonwebtoken` · `aes-gcm` · `dashmap` · `tracing`/`tracing-subscriber` · `serde`/`serde_json` · `thiserror` · `time`/`chrono`。

## 10. 错误处理与韧性

- 类型化错误(`thiserror`)→ 映射 HTTP 状态。
- 引擎:轮询失败记录日志后继续下一轮;job 失败有限退避重试;panic 隔离不影响池。
- 上游失效账号 → `needs_relogin`,用户可见。
- 优雅退出:收到信号后停止投递新 job,等待在途 job 完成。

## 11. 部署

- 单 Rust 二进制(Axum)+ Nginx(TLS via Let's Encrypt)反代并托管 SPA build。
- systemd 守护,启动自动跑 DB 迁移并 seed 超管账号 + 默认 `system_config`。
- 环境变量:`DATABASE_URL` · `JWT_SECRET` · `ENCRYPTION_KEY` · `SUPERADMIN_USERNAME`/`SUPERADMIN_PASSWORD`(seed)· 监听地址。
- PostgreSQL,`pg_dump` 定时备份。

## 12. 测试策略

- 单元:并发扣费正确性(多任务打同一账号,断言不超扣/只扣一次)· 幂等唯一约束防重签 · token single-flight · 模块匹配。
- 集成:`wiremock` mock 上游 signAttendance/Firebase,测 200/202/failed 状态机。
- 压测:模拟 500 账号同时解锁,断言并发闸生效 + 每笔只扣一次。

## 13. 安全注意

- 超管初始密码生产环境务必尽快修改。
- 所有 refresh token / device_uid 加密入库;`ENCRYPTION_KEY`、`JWT_SECRET` 不入代码库。
- cookie `Secure+HttpOnly+SameSite=Strict` + CSRF token。
- 代签行为本身的合规风险由运营方承担,本设计仅作技术实现。

## 14. 范围外(YAGNI,后续)

- 在线支付网关、卡密充值(当前仅管理员手动充值)。
- Redis / 多 Worker 进程横向扩展(方案 B,引擎模块已为此预留边界)。
- 代理 IP 池(若上游对单 IP 风控加剧再引入)。

# InstAtt 自动签到 SaaS 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把本地 InstAtt 自动签到脚本改造为跑在 Linux 服务器、带学校账号登录与余额计费、能扛 ~500 并发签到的 Rust + React SaaS。

**Architecture:** 方案 A 单体——Axum HTTP API + 进程内签到引擎(共享轮询器 + 有界并发 Worker 池 + 每日刷新器)+ PostgreSQL。引擎做成边界清晰模块,将来可拆独立 binary。

**Tech Stack:** Rust(axum/tokio/sqlx/reqwest/argon2/jsonwebtoken/aes-gcm/dashmap/tracing)· PostgreSQL · React+Vite+TS · Docker(dev DB)· Nginx+systemd(部署)。

> 配套 spec: `docs/superpowers/specs/2026-06-05-instatt-saas-design.md`

---

## 阶段总览(按依赖顺序,每阶段产出可测试软件)

- **Phase 0** 工程脚手架 + 开发基建(Cargo 项目、Docker Postgres、配置、日志、health)
- **Phase 1** 数据层(迁移、模型、AES-GCM 加密、seed 超管/配置)
- **Phase 2** InstAtt 上游客户端(移植 Python:Azure/Firebase/签到/查询)
- **Phase 3** Token 管理器(两层续期 + single-flight)
- **Phase 4** 签到引擎(原子扣费 + Worker 状态机 + 共享轮询器 + 每日刷新)
- **Phase 5** 认证(Device Code 会话、JWT cookie、超管密码登录、角色中间件)
- **Phase 6** HTTP API(用户 / 管理员 / 超管 路由)
- **Phase 7** 前端 SPA(登录/仪表盘/记录/流水/管理)
- **Phase 8** 部署(Dockerfile、嵌入 SPA、systemd、nginx、备份)

> 约定:每阶段开始前若 micro-step 未展开,执行时按 TDD(写失败测试→跑红→最小实现→跑绿→提交)细化。后端用 `cargo test`,前端用 `vitest`/`playwright`。所有金额单位为「分」。

---

## Phase 0:工程脚手架 + 开发基建

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `src/config.rs`, `docker-compose.dev.yml`, `.env.example`, `.gitignore`, `rust-toolchain.toml`

- [ ] **Step 1: 初始化 git 仓库与 Rust 项目**

```bash
cd "C:/Users/Administrator/Desktop/bakcup/AutoSign"
git init
cargo init --name instatt_saas
```

- [ ] **Step 2: 写 `.gitignore`**

```
/target
.env
*.log
node_modules
web/dist
.sqlx
```

- [ ] **Step 3: 写 `Cargo.toml` 依赖**

```toml
[package]
name = "instatt_saas"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = { version = "0.7", features = ["macros"] }
tokio = { version = "1", features = ["full"] }
tower-http = { version = "0.5", features = ["fs", "cors", "trace"] }
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "postgres", "macros", "chrono", "json", "migrate"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
argon2 = "0.5"
jsonwebtoken = "9"
aes-gcm = "0.10"
dashmap = "6"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "2"
chrono = { version = "0.4", features = ["serde"] }
rand = "0.8"
anyhow = "1"

[dev-dependencies]
wiremock = "0.6"
```

- [ ] **Step 4: 写 `docker-compose.dev.yml`(本地 Postgres)**

```yaml
services:
  db:
    image: postgres:16
    environment:
      POSTGRES_USER: instatt
      POSTGRES_PASSWORD: instatt
      POSTGRES_DB: instatt
    ports: ["5432:5432"]
    volumes: ["pgdata:/var/lib/postgresql/data"]
volumes:
  pgdata:
```

- [ ] **Step 5: 写 `.env.example`**

```
DATABASE_URL=postgres://instatt:instatt@localhost:5432/instatt
JWT_SECRET=change-me-32-bytes-minimum-secret
ENCRYPTION_KEY=0123456789abcdef0123456789abcdef
SUPERADMIN_USERNAME=admin
SUPERADMIN_PASSWORD=changeme-strong-pass
BIND_ADDR=0.0.0.0:8080
RUST_LOG=instatt_saas=debug,info
```

- [ ] **Step 6: 写 `src/config.rs`** — 从环境变量加载 `Config { database_url, jwt_secret, encryption_key:[u8;32], superadmin_username, superadmin_password, bind_addr }`,缺失则 panic 并提示。`encryption_key` 校验为 32 字节。

- [ ] **Step 7: 写 `src/main.rs` 最小可跑** — 初始化 tracing、加载 config、起 Axum、`GET /api/health` 返回 `{"status":"ok"}`。

- [ ] **Step 8: 验证**

```bash
docker compose -f docker-compose.dev.yml up -d
cargo run
# 另一终端: curl localhost:8080/api/health  → {"status":"ok"}
```

- [ ] **Step 9: 提交** `chore: project scaffold + dev infra`

---

## Phase 1:数据层

**Files:**
- Create: `migrations/0001_init.sql`, `src/db/mod.rs`, `src/crypto.rs`, `src/models.rs`, `src/db/seed.rs`

- [ ] **Step 1: 写 `migrations/0001_init.sql`** — 按 spec §3 建表:`accounts`、`super_admin`、`device_code_sessions`、`sign_records`(含唯一约束)、`balance_transactions`、`system_config`。加索引:`accounts(account_name)`、`sign_records(account_id, created_at)`、`balance_transactions(account_id, created_at)`。

- [ ] **Step 2: `src/crypto.rs` + 测试(TDD)** — AES-256-GCM `encrypt(key,&[u8])->Vec<u8>`(prepend 12B nonce)/`decrypt`。先写 round-trip 失败测试 → 实现 → 跑绿。

- [ ] **Step 3: `src/db/mod.rs`** — `PgPool` 创建 + `sqlx::migrate!()` 启动时执行。

- [ ] **Step 4: `src/db/seed.rs` + 测试** — 启动时若 `super_admin` 无记录则用 `argon2` 哈希 `SUPERADMIN_PASSWORD` 写入;seed 默认 `system_config`(`price_per_sign_cents=100`,`poll_interval_sec=5`,`max_concurrency=50`,`jitter_ms_max=1500`)。测试:重复 seed 幂等。

- [ ] **Step 5: `src/models.rs`** — `Account`、`SignRecord`、`BalanceTx`、`SystemConfig` 等 `sqlx::FromRow` 结构。

- [ ] **Step 6: 验证 + 提交** `cargo test` 全绿;`feat: data layer (migrations, crypto, seed)`。

---

## Phase 2:InstAtt 上游客户端(移植 Python)

**Files:**
- Create: `src/engine/mod.rs`, `src/engine/instatt.rs`, `tests/instatt_test.rs`

移植自 `offline_sign.py` / `auto_sign_daemon.py`,常量(`FIREBASE_API_KEY` 等、`VENUE_BSSID_MAP`、`IGNORE_WIFI_VENUES`)搬入 `instatt.rs`。

函数(均 async,`reqwest::Client` 注入以便测试指向 wiremock):
- `device_code_start()` / `device_code_poll(device_code)`
- `refresh_azure(rt) -> access_token`
- `firebase_custom_token(azure_access) -> custom`
- `firebase_id_token(custom) -> (id_token, firebase_rt)`
- `refresh_firebase(firebase_rt) -> id_token`
- `get_user_info(azure_access)`(displayName/mail/employeeId)
- `get_student_info(id_token, student_id)`(deviceUID 等)
- `get_student_modules(id_token, student_id, year)`
- `get_ongoing_classes()`
- `sign_attendance(id_token, params) -> SignOutcome{ status_code, body }`

- [ ] 每个函数 TDD:用 `wiremock` 起本地 mock,断言请求体/路径正确、解析响应正确(覆盖 200/202/403/409)。base URL 可配置以便测试。
- [ ] 提交 `feat: instatt upstream client`

---

## Phase 3:Token 管理器

**Files:**
- Create: `src/engine/tokens.rs`

- [ ] `TokenManager{ cache: DashMap<i64,(String,Instant)>, inflight: DashMap<i64,Arc<Mutex<()>>> }`
- [ ] `get_valid_id_token(account_id) -> Result<String>`:命中缓存(<10min)直接返回;否则按 `account_id` 取 inflight 锁(single-flight),先试 `refresh_firebase`,失败再走 `refresh_azure→custom→id`(并持久化新 firebase_rt、轮换 azure_rt),仍失败 → 标记账号 `needs_relogin` 并返回 Err。
- [ ] `daily_refresh_all(pool)`:遍历 active 账号跑完整链保活,失败标记 `needs_relogin`。
- [ ] TDD:single-flight 并发只刷一次(用计数 mock);firebase 失败回退 azure 链路;彻底失败置 needs_relogin。
- [ ] 提交 `feat: token manager with two-layer refresh`

---

## Phase 4:签到引擎

**Files:**
- Create: `src/engine/billing.rs`, `src/engine/worker.rs`, `src/engine/poller.rs`, `src/engine/refresher.rs`

- [ ] **billing.rs + 测试**:`charge_for_sign(pool, account_id, price, record_fields)`——单事务:`UPDATE accounts SET balance_cents=balance_cents-$p WHERE id=$id AND balance_cents>=$p RETURNING balance_cents`;成功则 insert `sign_records(ok_200,charged)` + `balance_transactions(sign_charge, balance_after)`。**并发测试**:N 个 tokio 任务同时对同一账号扣费,断言总扣 = min(N, balance/price) 且 balance 不为负。
- [ ] **worker.rs + 测试**:job 状态机(spec §4.3)。全局 `Semaphore(max_concurrency)` + 按 `account_id` 串行(`DashMap<i64, Arc<Mutex>>`)+ job 前 `0..jitter_ms_max` 随机抖动。返回码处理:200→billing 扣费;202→记 already 不扣;其他→attempt+1 退避重试,超限记 failed。panic 隔离。用 wiremock 测三类返回码各自落库正确。
- [ ] **poller.rs + 测试**:每 `poll_interval` 拉 `ongoing_classes`,维护 `HashMap<module_code, Vec<account_id>>` 索引(查 active+auto_sign+balance≥price 账号的 my_modules),命中且 `sign_records` 无记录 → 投递 job。幂等:已签课程不重复投递。
- [ ] **refresher.rs**:每 24h 调 `daily_refresh_all`。
- [ ] **engine spawn**:`engine::run(pool, config)` 启动 poller/worker/refresher;`main.rs` 调用。
- [ ] **压测**:模拟 500 账号 + 一批课程解锁,断言并发不超 `max_concurrency`、每笔只扣一次。
- [ ] 提交 `feat: signing engine (billing, worker, poller, refresher)`

---

## Phase 5:认证

**Files:**
- Create: `src/auth/mod.rs`, `src/auth/jwt.rs`, `src/auth/device.rs`, `src/auth/middleware.rs`

- [ ] **jwt.rs**:签发/校验 JWT(claims: `sub=account_id|superadmin`, `role`, `exp=30d`),httponly+Secure+SameSite=Strict cookie。
- [ ] **device.rs**:`start` 建 `device_code_sessions` 调上游;`poll` 轮询上游,done 时跑完整登录链(拿 user_info→employeeId、firebase_rt、student_info.deviceUID、modules)→ 按 `account_name` upsert `accounts`(加密存 token)→ 签发 JWT。
- [ ] **admin login**:校验 `super_admin`(argon2)或 `accounts.role in (admin)` 的密码路径(仅超管有密码;admin 用 Device Code),签发 JWT。
- [ ] **middleware.rs**:`require_auth`(解析 cookie→AccountCtx)、`require_admin`、`require_superadmin`;CSRF token 校验改动型请求。
- [ ] TDD:JWT round-trip/过期;device poll 完成 upsert;角色中间件放行/拒绝。
- [ ] 提交 `feat: auth (device code, jwt, roles)`

---

## Phase 6:HTTP API

**Files:**
- Create: `src/api/mod.rs`, `src/api/me.rs`, `src/api/admin.rs`, `src/api/auth_routes.rs`

按 spec §7 实现全部路由。每个 handler:TDD(用 `axum::Router` + 测试 DB + 测试 token 起 in-process 请求,断言状态码/响应/落库)。
- [ ] auth_routes:device start/poll、admin login、logout
- [ ] me:GET me / PATCH auto-sign / POST sync / GET records / GET transactions(分页)
- [ ] admin:GET accounts(搜索分页)/ POST topup(写流水,事务)/ PATCH status / GET accounts/:id/records / GET stats
- [ ] superadmin:PATCH role / PATCH config / POST superadmin/password
- [ ] 提交 `feat: http api`

---

## Phase 7:前端 SPA

**Files:**
- Create: `web/`(Vite+React+TS),`web/src/pages/*`、`web/src/api.ts`、路由与鉴权守卫

- [ ] `npm create vite@latest web -- --template react-ts`;装 axios、react-router、一个轻量 UI 库(如 Mantine)。
- [ ] `api.ts`:封装后端接口(withCredentials)。
- [ ] 登录页:学校账号登录(展示 user_code+网址+轮询)/ 管理员登录。
- [ ] 仪表盘:余额、状态红条(needs_relogin)、挂机开关、课程、学号脱敏、立即同步。
- [ ] 签到记录 / 余额流水:分页表格。
- [ ] 管理端:账号管理(搜索+充值弹窗+启停+设角色)、系统配置(超管)、统计看板。
- [ ] 路由守卫按角色;`vitest` 关键组件测试 + 1 条 playwright 冒烟(登录→看板)。
- [ ] 提交 `feat: react spa`

---

## Phase 8:部署

**Files:**
- Create: `Dockerfile`, `deploy/instatt.service`, `deploy/nginx.conf`, `deploy/README.md`

- [ ] 前端 `npm run build` → `web/dist`;后端用 `tower-http::ServeDir` 托管(或 nginx 托管静态)。
- [ ] 多阶段 `Dockerfile`(构建 Rust + 构建前端 + 运行镜像)。
- [ ] `instatt.service`(systemd):env 文件、自动重启、优雅退出。
- [ ] `nginx.conf`:TLS(Let's Encrypt)、反代 `/api`、托管 SPA、`try_files` 回退 index.html。
- [ ] 启动跑迁移 + seed;`deploy/README.md` 写部署步骤 + `pg_dump` 备份 cron。
- [ ] 提交 `chore: deployment (docker, systemd, nginx)`

---

## Self-Review 摘要

- **Spec 覆盖**:§3 数据→P1;§4 引擎→P4;§5 token→P3;§6 权限→P5;§7 API→P6;§8 前端→P7;§11 部署→P8;§2 架构贯穿 P0/P4。无遗漏。
- **范围外**(spec §14):在线支付/卡密/Redis/代理池——本计划不含,符合预期。
- **类型一致**:`account_id:i64`、金额 `_cents:i64`、`SignOutcome{status_code,body}`、`charge_for_sign` 全程一致命名。

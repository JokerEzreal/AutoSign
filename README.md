# AutoSign · InstAtt Auto Sign-in Service

> Language: **English** · [简体中文](README.zh-CN.md)

An automatic attendance sign-in service for **InstAtt**, the attendance system of the University of Nottingham Malaysia campus. A student signs in to this site with their school account and ticks the courses to auto-attend; the server continuously watches the upstream "currently unlocked classes" feed, and the moment a lecturer unlocks a class it calls the upstream sign-in endpoint on the student's behalf. Billing is per **successful** sign-in.

- Backend: Rust (Axum + Tokio + sqlx) + PostgreSQL, a single binary that also serves the SPA
- Frontend: React 18 + Vite + TypeScript single-page app
- Bundled: a Windows classroom-BSSID collector, a headless-browser assisted-login worker, and the upstream app's reverse-engineering material

> The crate is named `instatt_saas`; the live panel is titled "Instatt 自动签到".

> **⚠️ Disclaimer: this project is for security research and study only.** Every upstream endpoint, config constant and data point described here was obtained by reverse-engineering the publicly distributed InstAtt APK (see [10. Reverse Engineering, Security Analysis and Hardening](#10-reverse-engineering-security-analysis-and-hardening)). Do not use it for actual proxy sign-in or anything that violates school rules, the operator's terms of service, or local law. All consequences are the user's own.

---

## Table of Contents

1. [Features](#1-features)
2. [How It Works](#2-how-it-works)
3. [Repository Layout](#3-repository-layout)
4. [Local Development](#4-local-development)
5. [Configuration](#5-configuration)
6. [Deployment](#6-deployment)
7. [HTTP API](#7-http-api)
8. [Data Model](#8-data-model)
9. [Bundled Tools and Directories](#9-bundled-tools-and-directories)
10. [Reverse Engineering, Security Analysis and Hardening](#10-reverse-engineering-security-analysis-and-hardening)
11. [Known Limitations and Notes](#11-known-limitations-and-notes)

---

## 1. Features

### User side (log in with a school account)

- **Login is registration**: the first login creates the account and grants 400 cents (¥4.00) of balance with one `register_bonus` ledger row; later logins only refresh credentials and do not re-grant.
- **Dashboard**: balance, account status, per-course auto-sign toggles (each course independent), a "Sync now" button to re-fetch the timetable and device info, and the list of currently supported venues.
- **Timetable**: a per-day view that combines the upstream timetable, official attendance, live unlock status, and this system's own sign-in result.
- **Sign-in records / balance ledger**: paginated, 20 rows per page.

### Admin side (admin / superadmin)

- **Dashboard**: total accounts, active accounts, today's successful sign-ins, today's revenue.
- **Account management**: search, manual top-up (writes a ledger row), enable/disable, view detail (profile, that day's timetable, sign-in ledger), and edit a user's course selection and auto-sign switch on their behalf.
- **Superadmin only**: appoint/revoke admins, change engine parameters (price, poll interval, concurrency, jitter), change the superadmin password.

### Sign-in engine

- **Shared poller**: pulls the public `ongoingClasses` every N seconds; one request serves everyone, independent of user count.
- **Firestore gRPC realtime listener**: a new unlock wakes the poller immediately, cutting latency to sub-second; on disconnect it reconnects with backoff while the poller keeps covering as a fallback.
- **Bounded-concurrency worker pool**: a global semaphore caps concurrency, work is serialized per account, and each job gets random jitter to spread out bursts.
- **Idempotent billing**: a unique constraint prevents double sign-ins; a single conditional statement prevents overdraft; only an upstream first success (200) is charged.
- **Two-layer token refresh**: short tokens are refreshed on demand before signing (cached 10 minutes, single-flight deduped); the full chain is run every 24 hours to keep the long tokens alive.

---

## 2. How It Works

### 2.1 Authentication chain

Upstream auth is a two-stage Azure AD → Firebase flow. Login runs the whole chain and creates the account (`src/auth/device.rs`):

```
School account (Azure AD)
  → Azure access / refresh token
  → Graph /me                        email prefix → account_name, employeeId → student_id
  → cloud function userLogin         Azure access → Firebase custom token
  → signInWithCustomToken            → Firebase ID token + Firebase refresh token
  → Firestore students/{id}          deviceUID, programme
  → Firestore students/{id}/modules  current-year modules; falls back to newest year if upstream has none yet
  → upsert accounts                  keyed by account_name; keeps old device/modules if this run fetched none
```

The backend offers three ways to obtain the Azure token:

| Method | Endpoints | Notes |
|---|---|---|
| **Assisted login** (used by the current login page) | `POST /api/auth/assisted/start` → `GET /api/auth/assisted/poll` | The server spawns a Playwright headless-browser worker (`tools/assisted_login.py`) that drives the authorization-code login for the user and relays the MFA "number matching" digits back to the page; the authorization code is captured via the app's own `https://localhost` redirect. The password is passed to the worker over stdin only, never written to disk or argv |
| **Direct password login** (ROPC) | `POST /api/auth/password/login` | The server exchanges username+password for the token directly. The username may be just the prefix; `@nottingham.edu.my` is appended automatically. **Accounts with MFA enabled will fail** |
| **Device Code** | `POST /api/auth/device/start` → `GET /api/auth/device/poll` | The standard Microsoft device-code flow; the session is stored in `device_code_sessions`. A code comment records that this flow was blocked by the tenant (AADSTS 7000218), which is why the login page switched to assisted login |

On success the backend issues an HS256 JWT in the `session` cookie (HttpOnly, SameSite=Strict, 30 days; `Secure` is added when `COOKIE_SECURE=true`). Each request re-reads the role from the database, so promotion/demotion takes effect immediately.

The superadmin uses a separate `super_admin` table (argon2 hash) and logs in from the `/manage` page; the superadmin has no personal sign-in account and lands on the admin dashboard.

### 2.2 Auto sign-in flow

```
                 ┌─────────────────────────────────────────┐
                 │  Firestore (upstream, publicly readable) │
                 │  ongoingClasses = currently unlocked     │
                 └──────────┬───────────────┬───────────────┘
      gRPC Listen (realtime)│               │ REST (every poll_interval_sec)
                            ▼               ▼
                    ┌──────────────┐   ┌──────────────┐
                    │  listener    │──▶│   poller     │  wake.notify → poll at once
                    └──────────────┘   └──────┬───────┘
                                              │ for each (unlocked class × account):
                                              │  • account active and balance ≥ price
                                              │  • enabled_modules contains the course code
                                              │  • no sign_records in 24h, not in inflight
                                              ▼
                                   ┌───────────────────────┐
                                   │  worker job           │  jitter → global semaphore → per-account lock
                                   │  1 decrypt deviceUID   │  missing → failed(missing_device_uid)
                                   │  2 venue → BSSID       │  not found → failed(missing_bssid)
                                   │  3 get short token     │  TokenManager
                                   │  4 signAttendance      │  3 inline retries on network error
                                   └───────────┬───────────┘
                                               ▼
                                   ┌───────────────────────┐
                                   │  billing.commit       │  in one transaction:
                                   │  200 → ok_200 + charge │   INSERT sign_records ON CONFLICT DO NOTHING
                                   │  202 → already (free)  │   UPDATE balance WHERE balance ≥ price
                                   │  other → failed (free) │   INSERT balance_transactions(sign_charge)
                                   └───────────────────────┘
```

The sign-in request goes to the upstream cloud function `signAttendance` with fields `moduleID / venue / courseType / courseYear / classDate / startTime / MACaddress / studentID / deviceUID`. `MACaddress` carries the venue's Wi-Fi BSSID; `courseType` / `courseYear` are split out of `moduleKey` (e.g. `COMP4082_AUM_26-27`).

Result and billing:

| `result` | Meaning | Charge |
|---|---|---|
| `ok_200` | Upstream first success | Charges `price_per_sign_cents` |
| `already_202` | Upstream says already signed | No charge |
| `failed` | See `detail`: `missing_bssid:<venue>` (venue unsupported), `missing_device_uid` (device not registered), `status_<code>:<body>` (upstream rejected) | No charge |

Accounts that are out of balance or not active are filtered out during polling and produce no record; they become eligible again on the next round once balance recovers. In a rare race, "signed but failed to charge" marks `detail` as `signed_but_insufficient_balance`. When network retries are exhausted no record is written, leaving it for the next round.

### 2.3 Token refresh

| Token | Lifetime | Use |
|---|---|---|
| Firebase ID token (short) | ~1 hour | Query timetable, sign in |
| Firebase refresh token (long) | Very long | Cheaply mint a new short token |
| Azure refresh token (long) | ~90 days, rotated on each use | Fallback when the Firebase RT fails; re-runs the full chain |

- **On-demand refresh** (`src/engine/tokens.rs`): short tokens are cached in-process for 10 minutes; on a miss, refresh is single-flighted per account, trying the Firebase RT first and falling back to the full Azure chain, encrypting and writing back the new Firebase RT.
- **Daily refresh** (`src/engine/refresher.rs`): every 24 hours the full chain runs for all active accounts to keep the long tokens alive and rotate them.
- If both layers fail, the account is set to `needs_relogin`, the panel shows a red banner, and the engine skips the account until the user logs in again.

### 2.4 Timetable and status derivation

`GET /api/me/classes?date=YYYYMMDD` uses a Firestore `runQuery` to fetch that day's classes from `students/{id}/classes`, overlays the day's `ongoingClasses` and the local `sign_records`, and derives a combined status per class:

| Status | Meaning |
|---|---|
| `cancelled` | Cancelled upstream |
| `attended` | Officially recorded as attended |
| `unlocked` | Unlocked right now, not yet signed |
| `absent` | Already conducted but not signed |
| `upcoming` / `locked` / `not_unlocked` | To be held and not unlocked: not started / within the class window / window already passed |

Times are computed in the campus time zone (UTC+8); the academic year rolls over in September and matches the upstream `courseYear` format (2026-09-30 → `26-27`).

### 2.5 Venues and BSSID

The upstream **server** validates the `MACaddress` submitted in the sign-in request against the venue's allowed-BSSID list: the list lives in Firestore, is not readable by clients, and a match counts as present (the cloud function returns 200), otherwise 403. So this system **can only sign venues whose BSSID has been collected** (`VENUE_BSSIDS` in `src/engine/instatt.rs`, currently 22 entries, with some adjacent rooms sharing one AP), plus the 5 venues flagged `ignoreWifi=true` upstream (`DA05 / DA07 / NB03 / NOLOC / ONLINE`, where any BSSID passes). Other venues get a `failed / missing_bssid` record and are not charged.

To add a venue, collect its BSSID with `tools/wifi-bssid` and append it to `VENUE_BSSIDS`; both the panel's "supported venues" and `GET /api/venues` read from that table.

### 2.6 Where the data came from: reverse-engineering the InstAtt APK

Every upstream detail used above — the Firebase project and API key, the Azure tenant/client IDs, the cloud-function names (`signAttendance`, `userLogin`, `unlock`, …), the `signAttendance` request fields, and how the client reports its connected BSSID (`MACaddress`) and handles `ignoreWifi` — came from decompiling the publicly distributed InstAtt Android APK (`instatt.instatt`, v1.43), kept in this repo under `app/`. Using the config taken from the APK, the publicly readable Firestore collections were then queried directly and written up in [`InstAtt_Database_Info.md`](InstAtt_Database_Info.md).

Note: venue BSSIDs are **not** in the APK; the allowed-BSSID list used for validation lives server-side (in Firestore, but not readable by a student token and not in the public `rooms` documents — `rooms` only has `filterStrength` and `ignoreWifi`), so they can only be surveyed on site with `tools/wifi-bssid`. The APK is a release build yet ships with no obfuscation or hardening at all, so the bar to reverse it is extremely low; see [10. Reverse Engineering, Security Analysis and Hardening](#10-reverse-engineering-security-analysis-and-hardening).

---

## 3. Repository Layout

```
.
├─ Cargo.toml / src/                 Rust backend (crate: instatt_saas)
│  ├─ main.rs                        startup: read .env → connect+migrate DB → seed → start engine → start HTTP
│  ├─ config.rs                      environment variables (panics if missing)
│  ├─ state.rs / models.rs           shared state, sqlx row models
│  ├─ crypto.rs                      AES-256-GCM field encryption (12-byte nonce ‖ ciphertext)
│  ├─ db/                            connection pool + embedded migrations; seed superadmin and default config
│  ├─ auth/                          device (device-code / ROPC / account-creation chain), assisted (assisted login),
│  │                                 jwt, password (argon2), middleware (role extractors)
│  ├─ api/                           router wiring, auth_routes, me, classes, admin
│  └─ engine/                        instatt (upstream client), listener, poller, worker,
│                                    billing, tokens, refresher
├─ migrations/                       sqlx migrations: 0001 initial schema, 0002 per-course toggle + programme, 0003 module-name map
├─ web/                              React + Vite frontend; the build output web/dist is served by the backend
│  └─ src/pages/                     Login, AdminLogin, Dashboard, Classes, Records, Transactions, admin/*
├─ deploy/                           systemd unit, nginx config, server deployment notes
├─ scripts/                          sync.py (package + upload source to the server), rexec.py (run remote commands)
├─ tools/
│  ├─ assisted_login.py              assisted-login worker (Playwright, runs on the server)
│  └─ wifi-bssid/                    classroom BSSID collector (Rust, Windows; Python version included)
├─ docs/superpowers/                 the 2026-06-05 design doc and implementation plan
├─ InstAtt_Database_Info.md          upstream Firestore public-data write-up (collection access, field meanings, sign-in request format)
├─ app/ + build.gradle + settings.gradle
│                                    decompiled upstream InstAtt Android app (v1.43), for reference only
├─ MyXposed/                         Xposed / LSPosed module (own repo: JokerEzreal/InstAttPlugin) that hooks the upstream app on-device
├─ docker-compose.dev.yml            PostgreSQL 16 for local development
└─ .env.example                      environment-variable template
```

---

## 4. Local Development

### Prerequisites

- Rust stable (edition 2021), Node.js 18+, Docker (or any working PostgreSQL 16)
- The upstream services are all public; the dev machine must be able to reach `login.microsoftonline.com`, `graph.microsoft.com`, `*.googleapis.com`, `*.cloudfunctions.net`

### Steps

```bash
# 1. Database
docker compose -f docker-compose.dev.yml up -d
#    → postgres:16, connection string postgres://instatt:instatt@localhost:5432/instatt

# 2. Config
cp .env.example .env
#    adjust JWT_SECRET as needed; ENCRYPTION_KEY must be exactly 32 bytes

# 3. Backend (runs migrations on startup and seeds the superadmin + default system_config)
cargo run
#    listening on 0.0.0.0:8080; GET /api/health → {"status":"ok"}
#    note: the engine starts with the process and immediately begins polling the real upstream ongoingClasses

# 4. Frontend dev server (/api proxied to :8080)
cd web && npm install && npm run dev

# Production build: npm run build → web/dist; the backend serves it via fallback, no separate static server needed
```

### Tests

```bash
# Requires DATABASE_URL pointing at a working PostgreSQL:
# #[sqlx::test] creates a temporary database per test and runs the migrations; all upstream calls are mocked with wiremock
cargo test
```

Covered by tests: encryption round-trip, JWT issue/expiry, argon2, cookie parsing, the class-status matrix, academic-year and time-zone conversion, upstream response parsing, token single-flight and the fallback chain, idempotent billing and no-overdraft under concurrency, 50 accounts signing the same class end-to-end, the registration bonus granted exactly once, and seed idempotency.

---

## 5. Configuration

### Environment variables

| Variable | Required | Default | Notes |
|---|---|---|---|
| `DATABASE_URL` | yes | | PostgreSQL connection string |
| `JWT_SECRET` | yes | | Session signing key |
| `ENCRYPTION_KEY` | yes | | AES-256-GCM key, **must be exactly 32 bytes**; changing it makes existing tokens and device IDs in the DB undecryptable |
| `SUPERADMIN_USERNAME` / `SUPERADMIN_PASSWORD` | yes | | Seeds the superadmin on first start; if the username already exists it is not overwritten, change the password in the panel afterwards |
| `BIND_ADDR` | no | `0.0.0.0:8080` | Listen address; `127.0.0.1:8080` is recommended behind nginx |
| `COOKIE_SECURE` | no | `false` | Set `true` under HTTPS to add `Secure` to the cookie |
| `WEB_DIR` | no | `web/dist` | Frontend build-output directory |
| `RUST_LOG` | no | `instatt_saas=info` | Log filter |

In development these are read from `.env` in the working directory (a minimal built-in parser that does not override already-set variables); in production they are injected by the systemd `EnvironmentFile`.

### system_config (runtime parameters, stored in the DB)

| key | seed default | Notes |
|---|---|---|
| `price_per_sign_cents` | `100` | Charge per successful sign-in (cents) |
| `poll_interval_sec` | `5` | Interval for polling `ongoingClasses` |
| `max_concurrency` | `50` | Cap on concurrent sign-in requests to upstream |
| `jitter_ms_max` | `1500` | Upper bound of the random delay before each sign-in job |
| `realtime_listen` | not seeded, defaults to `1` | Set `0` to disable the Firestore realtime listener and rely on polling only |

Edit these on the "System config" page or via `PATCH /api/admin/config`. **The engine reads these values only at startup, so a restart is required after changing them.**

---

## 6. Deployment

Production shape: Ubuntu 24.04, source at `/opt/instatt_saas`, a single release binary + local PostgreSQL, guarded by systemd, with nginx as the port 80/443 entry reverse-proxying to `127.0.0.1:8080`. The full steps (environment variables, systemd, nginx, certbot, daily `pg_dump`) are in [`deploy/README.md`](deploy/README.md).

```bash
# Local: package and upload the source to the server
#   excludes .git / target / node_modules / dist, and .env / .deploy.env / instatt_tokens.json
python scripts/sync.py

# Local: run a command on the server (auto cd into REMOTE_DIR)
python scripts/rexec.py "cargo build --release && cd web && npm run build && cd .. && systemctl restart instatt"
```

Both scripts depend on `paramiko` and read connection details from `.deploy.env` at the repo root (`HOST / USER / PASS / REMOTE_DIR`, gitignored).

### Server dependencies for assisted login

Assisted login spawns a headless browser on the server; the paths are hard-coded in `src/auth/assisted.rs` and `tools/assisted_login.py`:

- A Python virtualenv at `/opt/pwlogin` with `playwright` (including chromium) and `cryptography` installed
- The worker script `/opt/instatt_saas/tools/assisted_login.py`, which reads `ENCRYPTION_KEY` from `/opt/instatt_saas/.env`
- The status directory `/tmp/assisted/`, one `<session_id>.json` per login, deleted by the backend on success or failure
- A cap of 5 concurrent workers; it waits up to 180 seconds for the user to approve in Authenticator, and the frontend polls for up to 200 seconds

Without this setup, `/api/auth/assisted/start` returns 503; the device-code and ROPC endpoints are unaffected.

---

## 7. HTTP API

All endpoints return JSON; errors are uniformly `{"error": "..."}`. The `page` parameter starts at 0, 20 rows per page. In the access column, "user" means a logged-in user with a personal sign-in account, and "auth" includes the superadmin.

| Method Path | Access | Notes |
|---|---|---|
| `GET /api/health` | public | Liveness check |
| `POST /api/auth/assisted/start` | public | `{username, password}` → `{session_id}` |
| `GET /api/auth/assisted/poll?id=` | public | `pending` / `mfa{number}` / `done` (sets cookie) / `failed{error}` |
| `POST /api/auth/password/login` | public | `{username, password}`, ROPC direct login |
| `POST /api/auth/device/start` | public | → `{session_id, user_code, verification_uri, interval, expires_in}` |
| `GET /api/auth/device/poll?id=` | public | `pending` / `declined` / `expired` / `done` |
| `POST /api/auth/admin/login` | public | Superadmin username/password |
| `POST /api/auth/logout` | public | Clears the cookie |
| `GET /api/me` | auth | Role + account profile; the superadmin's `account` is `null` |
| `PATCH /api/me/auto-sign` | user | `{auto_sign}`, master auto-sign switch |
| `PATCH /api/me/modules` | user | `{enabled: [...]}`, must be a subset of the user's own timetable |
| `POST /api/me/sync` | user | Re-fetch device ID, programme, timetable; new courses default to auto-sign on |
| `GET /api/me/classes?date=` | user | Combined timetable view for a day, defaults to campus today |
| `GET /api/me/records?page=` | user | Sign-in records |
| `GET /api/me/transactions?page=` | user | Balance ledger |
| `GET /api/venues` | auth | List of supported venues |
| `GET /api/admin/accounts?search=&page=` | admin | Account list, fuzzy search by `account_name` |
| `POST /api/admin/accounts/:id/topup` | admin | `{amount_cents, note}`, writes a `topup` ledger row |
| `PATCH /api/admin/accounts/:id/status` | admin | `active` / `disabled` |
| `GET /api/admin/accounts/:id/detail` | admin | Account detail |
| `GET /api/admin/accounts/:id/classes?date=` | admin | That account's timetable for a day |
| `GET /api/admin/accounts/:id/records?page=` | admin | That account's sign-in records |
| `PATCH /api/admin/accounts/:id/modules` | admin | Edit course selection on behalf of a user |
| `PATCH /api/admin/accounts/:id/auto-sign` | admin | Edit the auto-sign switch on behalf of a user |
| `GET /api/admin/stats` | admin | Total / active / today's successful sign-ins / today's revenue |
| `GET /api/admin/config` | admin | Read system_config |
| `PATCH /api/admin/config` | superadmin | Write system_config (takes effect after restart) |
| `PATCH /api/admin/accounts/:id/role` | superadmin | `user` / `admin` |
| `POST /api/admin/superadmin/password` | superadmin | `{old, new}`, new password at least 6 chars |

Any non-`/api` path falls back to `WEB_DIR/index.html` to support client-side routing.

---

## 8. Data Model

All money is an integer number of cents (`BIGINT`); `device_uid / azure_rt / firebase_rt` are stored as AES-256-GCM ciphertext (`BYTEA`).

| Table | Purpose | Key fields |
|---|---|---|
| `accounts` | A school account is the user, 1:1 | `account_name` unique; `student_id`; `my_modules` (timetable), `enabled_modules` (auto-sign selection, a subset), `module_info` (code → course name); `balance_cents`; `auto_sign`; `role` (`user` / `admin`); `status` (`active` / `needs_relogin` / `disabled`); `course` (programme) |
| `super_admin` | Built-in superadmin | `username`, argon2 `password_hash`, `role='superadmin'` |
| `device_code_sessions` | Device-code login sessions | `device_code`, `user_code`, `status` (`pending` / `done` / `expired` / `error`), `result_account_name` |
| `sign_records` | Sign-in results, also billing proof | `result`, `charged_cents`, `detail`; the **unique constraint `(account_id, module_key, class_date, start_time)`** is the ultimate idempotency guarantee |
| `balance_transactions` | Append-only balance ledger | `amount_cents` (positive for top-up, negative for charge), `type` (`topup` / `sign_charge` / `register_bonus` …), `ref_id` (links a sign_record), `balance_after`, `operator` |
| `system_config` | Runtime parameters | `key` / `value` |

Migration files live in `migrations/` and run automatically at startup via `sqlx::migrate!`.

---

## 9. Bundled Tools and Directories

### tools/wifi-bssid

A small Windows tool that records the currently connected Wi-Fi BSSID with zero dependencies (it only calls `netsh wlan`), so helpers can survey AP addresses in classrooms. It writes `bssid_records.csv` and prints a line that can be pasted straight into `VENUE_BSSIDS`. A Python version is included to work around the Windows 11 24H2 location-permission issue. Usage and FAQ are in [`tools/wifi-bssid/README.md`](tools/wifi-bssid/README.md).

### tools/assisted_login.py

The assisted-login worker, see [6. Deployment](#6-deployment). It reads one line of JSON from stdin and writes a status file; the Azure token it obtains is encrypted with the server's `ENCRYPTION_KEY` before being written to disk.

### app/ (with the root build.gradle / settings.gradle)

The decompiled upstream InstAtt Android app (package `instatt.instatt`, version 1.43), about 9,000 files, used to confirm the upstream endpoint fields, Firebase config, and Wi-Fi checking logic. It is not part of this service's build.

### MyXposed/

A standalone Xposed / LSPosed module in its own repository, [JokerEzreal/InstAttPlugin](https://github.com/JokerEzreal/InstAttPlugin), that runs on a rooted phone and hooks the upstream app: it auto-taps sign-in after detecting an unlock, forges the BSSID, and forces `ignoreWifi`. It is independent of the server approach and represents an alternative "on-device" route. See `MyXposed/README.md`, `FEATURES.md`, `USAGE_GUIDE.md`.

### InstAtt_Database_Info.md

A write-up of the upstream Firestore public data: which collections are readable without auth (`global/*`, `rooms/*`, `ongoingClasses`), what each field means, the class-type and status codes, and the `signAttendance` request format and response codes. The constants and parsing in `src/engine/instatt.rs` are based on it.

### docs/superpowers/

The 2026-06-05 SaaS design doc and staged implementation plan. The current state diverges from the docs: login grew from a single device-code flow to three methods; the Firestore realtime listener, per-course toggles, the timetable page, and the registration bonus were added; the CSRF token mentioned in the docs is not implemented and the system relies on the `SameSite=Strict` cookie instead.

### Files kept locally, not committed

`.gitignore` excludes `.env`, `.deploy.env` (server credentials), `instatt_tokens.json` (real users' refresh tokens), and the standalone Python scripts that preceded the server (`auto_sign_daemon*.py` / `offline_sign*.py`). `src/engine/instatt.rs` was ported from those two scripts.

---

## 10. Reverse Engineering, Security Analysis and Hardening

> This whole section is the result of static analysis of the **publicly distributed, freely downloadable** InstAtt APK. Its purpose is to explain where this project's data comes from and to give defensive recommendations. It involves no intrusion into or unauthorized access of the upstream servers.

### 10.1 How it was reverse-engineered

1. Decompile the InstAtt Android APK (`instatt.instatt`, versionName 1.43) into readable Java, kept under `app/` (109 business classes in the `instatt` package).
2. Extract the client config and protocol: Firebase project/key, Azure tenant and client IDs, the cloud-function name list, the field structure of requests such as `signAttendance`, and how the client reports its connected BSSID (`MACaddress`) and handles `ignoreWifi`.
3. Use that config to query the **publicly readable** Firestore collections directly (`global/*`, `rooms/*`, `ongoingClasses`) and write up the field meanings, code tables, and sign-in request format in [`InstAtt_Database_Info.md`](InstAtt_Database_Info.md).
4. Reimplement the above in Rust as this service's upstream client (`src/engine/instatt.rs`).

### 10.2 Finding: the APK is unobfuscated, unencrypted, unpacked

The release build (`BuildConfig.BUILD_TYPE = "release"`, `DEBUG = false`) decompiles straight into Java with original package/class/field names, with almost no barrier to reverse:

| Observation | Evidence (`app/` in this repo) |
|---|---|
| Class, method, and field names all preserved, no `a/b/c` obfuscation | 109 meaningfully named classes in the `instatt` package, e.g. `LecturerHomeFragment`, `WifiConnectionReceiver`, `FirebaseFunctionName` |
| Cloud-function names hard-coded in cleartext | `FirebaseFunctionName.java`: `signAttendance` / `userLogin` / `unlock` / `lock` / `createClass` / `modifyAttendanceAdmin` … |
| Firebase keys in cleartext | `google_api_key`, `project_id`, `firebase_database_url` in `res/values/strings.xml` |
| Azure identity constants in cleartext | `AzureParameters.java` (tenant ID, client ID) |
| Client Wi-Fi collection and pre-check logic readable and editable | reading the connected BSSID and the `ignoreWifi` switch in `GlobalStatic`, `CustomWifi`, `WifiConnectionReceiver` (the authoritative check is server-side) |

"Unencrypted / unpacked" is inferred from the above: the DEX decompiles directly into source with original identifiers, and keys and endpoints appear in cleartext, which indicates neither string encryption nor packing (no dedicated unpacking test was run).

### 10.3 Resulting attack surface

- **The server validates a client-reported BSSID, which is forgeable and replayable**: the location check really is server-side — the cloud function `signAttendance` compares the request's `MACaddress` against the venue's allowed-BSSID list in Firestore, succeeding (200) only on a match, otherwise returning 403 "BSSID validation failed". But it compares **the value the client itself put in the request**, and the server has no way to confirm the device is actually connected to that AP. So once one valid BSSID for a venue is surveyed (the list is not client-readable, but connecting on site once yields it), it can be replayed in the request from any network or location and pass the check reliably — both this project and the `MyXposed/` module rest on this.
- **Credentials and endpoints fully exposed**: the Firebase API key, project ID, and Azure tenant/client IDs are available in cleartext, which together with the login flow is enough to complete the whole auth chain off campus.
- **Publicly readable Firestore**: `ongoingClasses` exposes in real time which classes across the campus are unlocked, along with each venue's `ignoreWifi` config, scrapable without authentication (see [`InstAtt_Database_Info.md`](InstAtt_Database_Info.md) section 7, "Security issues summary").
- **No on-device integrity protection**: no obfuscation, no root/hook detection, no certificate pinning, so runtime instrumentation such as Xposed can freely rewrite `ignoreWifi`, forge the BSSID, and auto-tap sign-in.

### 10.4 Hardening recommendations for the vendor

1. **Do not treat a client-reported BSSID as proof of presence**: the check is server-side, but it trusts the `MACaddress` the client wrote into the request. Switch to a signal the server can observe independently and the client cannot forge (e.g. a one-time unlock nonce from the lecturer side, the server-observed access network, or a time-boxed nonce bound to the class session) rather than letting the student device self-report which AP it is on.
2. **Tighten Firestore security rules**: expose by least privilege; `ongoingClasses` / `rooms` / `global` should not be wholly open to unauthenticated clients.
3. **Enable code obfuscation and string encryption**: R8/ProGuard (or a commercial packer) plus resource/string encryption to raise the reverse-engineering cost significantly.
4. **Add runtime integrity and anti-instrumentation**: root / emulator / debugger / Xposed detection, certificate pinning, anti-sniffing, and repackaging signature checks.
5. **Adopt Firebase App Check** and restrict the API key by origin/application to block direct calls that bypass the official app.
6. **Server-side risk controls**: detection and rate limiting for reused `deviceUID`, abnormal sign-in frequency, and geographic/network anomalies.

> These recommendations target the upstream root cause — **trusting what the client reports**. Once they are in place, this service's sign-in path stops working.

### 10.5 For study only, and disclaimer

This project (including the server, the `MyXposed/` module, and all reverse-engineering material) is **for security research, protocol analysis, and study only**, to illustrate the classic problem of "a server over-trusting client-reported data (such as the connected BSSID)". Do not use it for real proxy sign-in, or anything that violates school rules, the operator's terms of service, or local law. Using it in production means submitting false attendance to the school's system on a student's behalf; the risk and responsibility rest with the operator and the user, and the author and this repository accept no liability for any resulting consequences.

**Infringement and takedown contact**: if anything in this repo infringes your rights or raises other concerns, email [fs840594947@gmail.com](mailto:fs840594947@gmail.com) and I will handle it promptly (removing or taking down the content in question).

---

## 11. Known Limitations and Notes

- **Limited venue coverage**: only venues in `VENUE_BSSIDS` plus the Wi-Fi-free ones can be signed; the rest are recorded as failed and not charged, so the table needs ongoing collection.
- **Engine parameters apply on restart**: `system_config` is read only at process startup.
- **The first daily refresh is 24h after startup**: the refresher sleeps before its first run, so for the first 24h after a restart it relies on on-demand refresh only.
- **Assisted login is bound to server paths and Linux-only**: see [6. Deployment](#6-deployment); the endpoint is unavailable on a local dev machine.
- **ROPC does not support MFA**: accounts with MFA or conditional access will fail and should use assisted login.
- **The login page currently exposes only assisted login**: the device-code and ROPC endpoints still exist in the backend, but `web/src/pages/Login.tsx` has no entry point for them.
- **A price string is hard-coded in the frontend**: the dashboard says "¥2.00 per success", while the seed default price is 100 cents; when changing the price, remember to update the copy in `Dashboard.tsx`.
- **Changing `ENCRYPTION_KEY` effectively wipes credentials**: all refresh tokens and device IDs in the DB become undecryptable and everyone must log in again.
- **Security**: change the superadmin's initial password in the panel as soon as possible; keep `JWT_SECRET`, `ENCRYPTION_KEY`, and `.deploy.env` out of the repo; always enable HTTPS in production and set `COOKIE_SECURE` to `true`.
- **Compliance and disclaimer**: this system submits attendance on a student's behalf and passes the upstream location check with a surveyed BSSID, in direct conflict with school rules, and the accounts involved may be penalized. This project is for study and research only; see [10.5 For study only, and disclaimer](#105-for-study-only-and-disclaimer) for the full statement.

---

## ☕ Support the author

If this project or the write-up helped you and you like the author's work, a small tip is very welcome — thank you!

<table>
  <tr>
    <td align="center">
      <img src="docs/sponsor/alipay.jpg" width="240" alt="Alipay"><br/>
      <b>Alipay</b>
    </td>
    <td align="center">
      <img src="docs/sponsor/wechat.jpg" width="240" alt="WeChat Pay"><br/>
      <b>WeChat Pay</b>
    </td>
  </tr>
</table>

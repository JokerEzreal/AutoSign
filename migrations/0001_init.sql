-- InstAtt SaaS 初始 schema。金额单位一律为「分」(bigint)。
-- 敏感字段(device_uid / azure_rt / firebase_rt)以 AES-GCM 密文(bytea)存储。

CREATE TABLE accounts (
    id              BIGSERIAL PRIMARY KEY,
    account_name    TEXT NOT NULL UNIQUE,          -- 学校账号,如 niubi666
    student_id      TEXT NOT NULL DEFAULT '',      -- device_uid 前 8 位
    device_uid      BYTEA,                         -- 加密
    azure_rt        BYTEA,                         -- 加密
    firebase_rt     BYTEA,                         -- 加密
    my_modules      JSONB NOT NULL DEFAULT '[]'::jsonb,
    balance_cents   BIGINT NOT NULL DEFAULT 0,
    auto_sign       BOOLEAN NOT NULL DEFAULT TRUE,
    role            TEXT NOT NULL DEFAULT 'user',    -- 'user' | 'admin'
    status          TEXT NOT NULL DEFAULT 'active',  -- 'active' | 'needs_relogin' | 'disabled'
    last_synced_at  TIMESTAMPTZ,
    last_refresh_at TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE super_admin (
    id              BIGSERIAL PRIMARY KEY,
    username        TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,                 -- argon2
    role            TEXT NOT NULL DEFAULT 'superadmin',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE device_code_sessions (
    id                BIGSERIAL PRIMARY KEY,
    device_code       TEXT NOT NULL,
    user_code         TEXT NOT NULL,
    verification_uri  TEXT NOT NULL,
    interval_sec      INT NOT NULL DEFAULT 5,
    expires_at        TIMESTAMPTZ NOT NULL,
    status            TEXT NOT NULL DEFAULT 'pending', -- pending|done|expired|error
    result_account_name TEXT,
    error             TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE sign_records (
    id              BIGSERIAL PRIMARY KEY,
    account_id      BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    module_key      TEXT NOT NULL,
    module_name     TEXT NOT NULL DEFAULT '',
    venue           TEXT NOT NULL DEFAULT '',
    class_date      BIGINT NOT NULL,
    start_time      BIGINT NOT NULL,
    result          TEXT NOT NULL,                 -- 'ok_200' | 'already_202' | 'failed'
    charged_cents   BIGINT NOT NULL DEFAULT 0,
    attempt         INT NOT NULL DEFAULT 0,
    detail          TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, module_key, class_date, start_time)
);

CREATE TABLE balance_transactions (
    id              BIGSERIAL PRIMARY KEY,
    account_id      BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    amount_cents    BIGINT NOT NULL,               -- 充值正,扣费负
    type            TEXT NOT NULL,                 -- topup|sign_charge|refund|adjust
    ref_id          BIGINT,                        -- sign_record id 等
    balance_after   BIGINT NOT NULL,
    note            TEXT NOT NULL DEFAULT '',
    operator        TEXT NOT NULL DEFAULT '',      -- 操作者标识
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE system_config (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_sign_records_account_created ON sign_records(account_id, created_at DESC);
CREATE INDEX idx_balance_tx_account_created ON balance_transactions(account_id, created_at DESC);
CREATE INDEX idx_accounts_status ON accounts(status);

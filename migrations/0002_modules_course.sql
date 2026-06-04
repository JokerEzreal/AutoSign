-- 每门课可单独选择是否自动签(enabled_modules,my_modules 的子集),并记录专业。
ALTER TABLE accounts ADD COLUMN enabled_modules JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE accounts ADD COLUMN course TEXT NOT NULL DEFAULT '';

-- 存量账号:默认全部启用,保持原有行为(用户可在面板取消勾选)。
UPDATE accounts SET enabled_modules = my_modules;

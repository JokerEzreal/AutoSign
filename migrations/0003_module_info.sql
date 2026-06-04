-- 课程详细信息映射:{ "COMP3040": "Software Engineering", ... }
ALTER TABLE accounts ADD COLUMN module_info JSONB NOT NULL DEFAULT '{}'::jsonb;

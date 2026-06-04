-- migrations/0010_template_default.sql
-- Mark a template as a "default" (featured starter shown in the rule gallery).
--
-- is_default is a featuring flag: default templates sort first and get a star in
-- the "Start from a template" browse list. Scope follows existing visibility:
-- an admin's default is saved global (is_shared=1) so every user sees it; a
-- non-admin's default stays personal (visible only to its owner) unless they
-- also share it.

ALTER TABLE templates ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_templates_default ON templates(is_default);

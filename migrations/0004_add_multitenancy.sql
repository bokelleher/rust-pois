-- migrations/0004_add_multitenancy.sql
-- Add multi-tenancy support: ownership tracking and soft deletes

-- Add ownership and soft delete to channels
ALTER TABLE channels ADD COLUMN owner_user_id INTEGER REFERENCES users(id);
ALTER TABLE channels ADD COLUMN deleted_at TEXT;

-- Add ownership and soft delete to rules
ALTER TABLE rules ADD COLUMN owner_user_id INTEGER REFERENCES users(id);
ALTER TABLE rules ADD COLUMN deleted_at TEXT;

-- Create indexes for performance
CREATE INDEX IF NOT EXISTS idx_channels_owner ON channels(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_channels_deleted ON channels(deleted_at);
CREATE INDEX IF NOT EXISTS idx_rules_owner ON rules(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_rules_deleted ON rules(deleted_at);

-- Note: Existing channels/rules will have NULL owner_user_id
-- Admins can see NULL-owned resources, regular users cannot

#!/usr/bin/env bash
# fix_default_channel.sh
# Fixes: "default" channel cannot be re-created after deletion
# Cause: SQLite soft-delete leaves row with deleted_at set, blocking UNIQUE constraint
# Usage: sudo bash fix_default_channel.sh

set -e

DB="${POIS_DB:-/opt/pois/pois.db}"
DB="${DB#sqlite:///}"  # strip sqlite:// prefix if present

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  POIS Default Channel Fix"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# Check DB exists
if [ ! -f "$DB" ]; then
  echo "✗ Database not found at: $DB"
  echo "  Set POIS_DB env var if your DB is in a custom location."
  exit 1
fi

echo "▶ Database: $DB"
echo ""

# Show current state
echo "▶ Current channels (including soft-deleted):"
sqlite3 "$DB" "SELECT id, name, COALESCE(deleted_at, 'active') as status FROM channels ORDER BY id;"
echo ""

# Check if default is soft-deleted
SOFT_DELETED=$(sqlite3 "$DB" "SELECT COUNT(*) FROM channels WHERE name='default' AND deleted_at IS NOT NULL;")

if [ "$SOFT_DELETED" = "0" ]; then
  ACTIVE=$(sqlite3 "$DB" "SELECT COUNT(*) FROM channels WHERE name='default' AND deleted_at IS NULL;")
  if [ "$ACTIVE" = "1" ]; then
    echo "✓ 'default' channel is already active — no fix needed."
    exit 0
  else
    echo "✓ No 'default' channel found — POIS will seed it on next restart."
    exit 0
  fi
fi

echo "✗ Found soft-deleted 'default' channel blocking re-creation."
echo ""

# Backup first
BACKUP="${DB}.backup.$(date +%Y%m%d%H%M%S)"
echo "▶ Creating backup: $BACKUP"
cp "$DB" "$BACKUP"
echo "  ✓ Backup created."
echo ""

# Hard-delete the soft-deleted default row
echo "▶ Removing soft-deleted 'default' channel..."
sqlite3 "$DB" "DELETE FROM channels WHERE name='default' AND deleted_at IS NOT NULL;"
echo "  ✓ Done."
echo ""

# Verify
REMAINING=$(sqlite3 "$DB" "SELECT COUNT(*) FROM channels WHERE name='default';")
if [ "$REMAINING" = "0" ]; then
  echo "✓ Cleared. Restart POIS to auto-seed the default channel and rule:"
  echo ""
  echo "  sudo systemctl restart pois"
  echo ""
  echo "  Or re-create manually via the UI or API."
else
  echo "✗ Row still present — manual intervention needed."
  sqlite3 "$DB" "SELECT id, name, deleted_at FROM channels WHERE name='default';"
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

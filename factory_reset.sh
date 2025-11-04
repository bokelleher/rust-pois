#!/usr/bin/env bash
#
# POIS factory reset script
# - Stops the systemd service
# - Deletes pois.db in the install directory
# - Restarts the service so it can re-run migrations and reseed defaults
#
# Intended to be run as root:
#   sudo ./factory_reset.sh
#
# NOTE: This will erase ALL channels/rules/events in the SQLite DB.

set -euo pipefail

echo "=== POIS factory reset ==="

if [[ "$(id -u)" -ne 0 ]]; then
  echo "ERROR: Please run this script as root (e.g.: sudo $0)"
  exit 1
fi

DEFAULT_SERVICE_NAME="pois"
DEFAULT_INSTALL_DIR="/opt/pois"

read -rp "Enter systemd service name [${DEFAULT_SERVICE_NAME}]: " SERVICE_NAME
SERVICE_NAME=${SERVICE_NAME:-$DEFAULT_SERVICE_NAME}

read -rp "Enter install directory containing pois.db [${DEFAULT_INSTALL_DIR}]: " INSTALL_DIR
INSTALL_DIR=${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}

DB_PATH="${INSTALL_DIR}/pois.db"

echo
echo "Service:    ${SERVICE_NAME}"
echo "Install dir:${INSTALL_DIR}"
echo "DB path:    ${DB_PATH}"
echo
read -rp "This will DELETE ${DB_PATH} and all data. Continue? [y/N]: " CONFIRM
CONFIRM=${CONFIRM:-N}

if [[ ! "${CONFIRM}" =~ ^[Yy]$ ]]; then
  echo "Aborted."
  exit 0
fi

echo
echo "1) Stopping systemd service '${SERVICE_NAME}'..."
systemctl stop "${SERVICE_NAME}" || {
  echo "WARNING: Failed to stop ${SERVICE_NAME} (it may not be running)."
}

echo
echo "2) Deleting database file ${DB_PATH}..."
if [[ -f "${DB_PATH}" ]]; then
  rm -f "${DB_PATH}"
  echo "   Removed ${DB_PATH}."
else
  echo "   ${DB_PATH} does not exist; nothing to delete."
fi

echo
echo "3) Restarting systemd service '${SERVICE_NAME}'..."
systemctl restart "${SERVICE_NAME}"
sleep 2

echo
echo "4) Checking service status..."
systemctl --no-pager status "${SERVICE_NAME}" || true

echo
echo "5) Verifying new database..."
if [[ -f "${DB_PATH}" ]]; then
  echo "   New DB created at ${DB_PATH}. Listing channels and rules:"
  sqlite3 "${DB_PATH}" "SELECT id, name, enabled, timezone FROM channels;" 2>/dev/null || true
  sqlite3 "${DB_PATH}" "SELECT channel_id, name, action, priority FROM rules;" 2>/dev/null || true
else
  echo "   WARNING: ${DB_PATH} was not recreated. Check service logs:"
  echo "   sudo journalctl -u ${SERVICE_NAME} -f"
fi

echo
echo "=== Factory reset complete ==="


#!/usr/bin/env bash
#
# POIS ESAM Server uninstaller for Ubuntu 24.04
#
# - Stops and disables the systemd service
# - Removes the systemd unit file
# - Optionally removes:
#     - Binary
#     - Application directory
#     - Service user
#     - ufw rule for the configured port
#
# Intended to be run as root:
#   sudo ./uninstall.sh

set -euo pipefail

echo "=== POIS ESAM Server uninstaller ==="

if [[ "$(id -u)" -ne 0 ]]; then
  echo "ERROR: Please run this script as root (e.g.: sudo $0)"
  exit 1
fi

SERVICE_NAME_DEFAULT="pois"
read -rp "Enter systemd service name to uninstall [${SERVICE_NAME_DEFAULT}]: " SERVICE_NAME
SERVICE_NAME=${SERVICE_NAME:-$SERVICE_NAME_DEFAULT}
UNIT_FILE="/etc/systemd/system/${SERVICE_NAME}.service"

if [[ ! -f "${UNIT_FILE}" ]]; then
  echo "WARNING: Unit file ${UNIT_FILE} not found."
  echo "         Service may already be removed or was installed with a different name."
else
  echo
  echo "1) Stopping and disabling systemd service '${SERVICE_NAME}'..."
  systemctl stop "${SERVICE_NAME}" 2>/dev/null || true
  systemctl disable "${SERVICE_NAME}" 2>/dev/null || true

  # Extract service user from unit file (User= line)
  SERVICE_USER=""
  if SERVICE_USER_LINE=$(grep -E '^User=' "${UNIT_FILE}" | head -n1 || true); then
    SERVICE_USER="${SERVICE_USER_LINE#User=}"
  fi

  # Extract ExecStart and try to detect binary path + port
  EXEC_START=""
  if EXEC_LINE=$(grep -E '^ExecStart=' "${UNIT_FILE}" | head -n1 || true); then
    EXEC_START="${EXEC_LINE#ExecStart=}"
  fi

  BINARY_PATH=""
  if [[ -n "${EXEC_START}" ]]; then
    # First token in ExecStart line is usually the binary
    BINARY_PATH=$(echo "${EXEC_START}" | awk '{print $1}')
  fi

  DETECTED_PORT=""
  if [[ -n "${EXEC_START}" ]]; then
    DETECTED_PORT=$(echo "${EXEC_START}" | sed -n 's/.*--port[= ]\([0-9]\+\).*/\1/p' || true)
  fi

  echo
  echo "2) Removing unit file ${UNIT_FILE}..."
  rm -f "${UNIT_FILE}"
  systemctl daemon-reload
fi

# Default assumptions (if we didn't find better info above)
DEFAULT_INSTALL_DIR="/opt/pois"

echo
echo "3) Binary and install directory cleanup"

if [[ -z "${BINARY_PATH:-}" ]]; then
  # Fall back to common location
  BINARY_PATH="/usr/local/bin/pois"
fi

if [[ -x "${BINARY_PATH}" ]]; then
  read -rp "Remove binary ${BINARY_PATH}? [y/N]: " REMOVE_BIN
  REMOVE_BIN=${REMOVE_BIN:-N}
  if [[ "${REMOVE_BIN}" =~ ^[Yy]$ ]]; then
    rm -f "${BINARY_PATH}"
    echo "   Removed ${BINARY_PATH}."
  else
    echo "   Skipping removal of ${BINARY_PATH}."
  fi
else
  echo "   Binary ${BINARY_PATH} not found or not executable; nothing to remove."
fi

read -rp "Enter application directory to remove [${DEFAULT_INSTALL_DIR}] (leave blank to skip): " APP_DIR
APP_DIR=${APP_DIR:-$DEFAULT_INSTALL_DIR}

if [[ -d "${APP_DIR}" ]]; then
  read -rp "Remove application directory ${APP_DIR}? This will delete the database and all files. [y/N]: " REMOVE_APP_DIR
  REMOVE_APP_DIR=${REMOVE_APP_DIR:-N}
  if [[ "${REMOVE_APP_DIR}" =~ ^[Yy]$ ]]; then
    rm -rf "${APP_DIR}"
    echo "   Removed ${APP_DIR}."
  else
    echo "   Skipping removal of ${APP_DIR}."
  fi
else
  echo "   Directory ${APP_DIR} does not exist; nothing to remove."
fi

echo
echo "4) Service user cleanup"

if [[ -z "${SERVICE_USER:-}" ]]; then
  echo "   No service user detected from unit file."
  read -rp "If you know the service user name and want to delete it, enter it now (or leave blank to skip): " SERVICE_USER
fi

if [[ -n "${SERVICE_USER}" ]]; then
  if id -u "${SERVICE_USER}" >/dev/null 2>&1; then
    read -rp "Delete service user '${SERVICE_USER}' (and its home, if any)? [y/N]: " DEL_USER
    DEL_USER=${DEL_USER:-N}
    if [[ "${DEL_USER}" =~ ^[Yy]$ ]]; then
      userdel -r "${SERVICE_USER}" 2>/dev/null || userdel "${SERVICE_USER}" || true
      echo "   User ${SERVICE_USER} deleted (if it existed)."
    else
      echo "   Skipping deletion of user ${SERVICE_USER}."
    fi
  else
    echo "   User ${SERVICE_USER} does not exist; nothing to delete."
  fi
else
  echo "   No service user specified; skipping user deletion."
fi

echo
echo "5) Firewall (ufw) cleanup"

PORT_HINT="${DETECTED_PORT:-8080}"
if ! command -v ufw >/dev/null 2>&1; then
  echo "   ufw not installed; skipping firewall cleanup."
else
  read -rp "Enter port to remove from ufw rules [${PORT_HINT}] (leave blank to skip): " FW_PORT
  FW_PORT=${FW_PORT:-$PORT_HINT}
  if [[ -n "${FW_PORT}" ]]; then
    read -rp "Attempt to remove 'allow ${FW_PORT}/tcp' from ufw? [y/N]: " DEL_UFW
    DEL_UFW=${DEL_UFW:-N}
    if [[ "${DEL_UFW}" =~ ^[Yy]$ ]]; then
      ufw delete allow "${FW_PORT}"/tcp || true
      echo "   Attempted to remove ufw rule for ${FW_PORT}/tcp (if it existed)."
    else
      echo "   Skipping ufw rule removal."
    fi
  else
    echo "   No port provided; skipping ufw rule removal."
  fi
fi

echo
echo "=== POIS uninstallation routine complete ==="
echo "You may want to manually verify:"
echo "  - No remaining systemd unit: /etc/systemd/system/${SERVICE_NAME}.service"
echo "  - No running service: sudo systemctl status ${SERVICE_NAME}"
echo "  - No binary: ${BINARY_PATH}"
echo "  - No app directory: ${APP_DIR}"


#!/usr/bin/env bash
#
# POIS ESAM Server installer for Ubuntu 24.04
# - Installs system dependencies & Rust
# - Clones/updates repo into /opt/pois
# - Builds release binary
# - Initializes SQLite database with migrations
# - Sets up and starts a systemd service
#
# Intended to be run as root:
#   sudo ./install.sh

set -euo pipefail

# === Static configuration ===
REPO_URL="https://github.com/bokelleher/rust-pois.git"
INSTALL_DIR="/opt/pois"
SERVICE_NAME="pois"
BINARY_NAME="pois"      # will be installed to /usr/local/bin/${BINARY_NAME}

echo "=== POIS ESAM Server installer for Ubuntu 24.04 ==="

# --- Root check ---
if [[ "$(id -u)" -ne 0 ]]; then
  echo "ERROR: Please run this script as root (e.g.: sudo $0)"
  exit 1
fi

# --- Prompt for service user ---
DEFAULT_USER="pois"
read -rp "Enter system user to run POIS as [${DEFAULT_USER}]: " SERVICE_USER
SERVICE_USER=${SERVICE_USER:-$DEFAULT_USER}
echo "Using service user: ${SERVICE_USER}"

# --- Prompt for port ---
DEFAULT_PORT=8080
read -rp "Enter port for POIS to listen on [${DEFAULT_PORT}]: " PORT
PORT=${PORT:-$DEFAULT_PORT}
echo "Using port: ${PORT}"

echo
echo "1) Installing system dependencies..."
apt update
apt install -y build-essential pkg-config libssl-dev sqlite3 libsqlite3-dev curl git

# ufw is optional; don't fail if it isn't present later
if ! dpkg -s ufw >/dev/null 2>&1; then
  apt install -y ufw || true
fi

echo
echo "2) Installing Rust toolchain (if needed)..."
if ! command -v cargo >/dev/null 2>&1; then
  # Install rustup non-interactively for root
  curl https://sh.rustup.rs -sSf | sh -s -- -y
fi

# Ensure cargo is on PATH for this session
export PATH="/root/.cargo/bin:${PATH}"

echo "   Rust version:  $(rustc --version || echo 'not found')"
echo "   Cargo version: $(cargo --version || echo 'not found')"

echo
echo "3) Cloning or updating repository at ${INSTALL_DIR}..."
if [[ -d "${INSTALL_DIR}/.git" ]]; then
  echo "   Repo already exists at ${INSTALL_DIR}, pulling latest..."
  git -C "${INSTALL_DIR}" fetch --all
  # Try main, then master as fallback
  if git -C "${INSTALL_DIR}" rev-parse origin/main >/dev/null 2>&1; then
    git -C "${INSTALL_DIR}" reset --hard origin/main
  elif git -C "${INSTALL_DIR}" rev-parse origin/master >/dev/null 2>&1; then
    git -C "${INSTALL_DIR}" reset --hard origin/master
  else
    echo "WARNING: Could not find origin/main or origin/master; leaving current branch as-is."
  fi
else
  mkdir -p "$(dirname "${INSTALL_DIR}")"
  git clone "${REPO_URL}" "${INSTALL_DIR}"
fi

cd "${INSTALL_DIR}"

echo
echo "4) Building POIS (release)..."
cargo build --release

# Determine built binary
if [[ -f "target/release/pois-esam-server" ]]; then
  BUILT_BIN="target/release/pois-esam-server"
else
  echo "ERROR: Expected target/release/pois-esam-server not found."
  echo "       Check Cargo.toml/bin name or adjust BINARY_NAME/BUILT_BIN in install.sh."
  exit 1
fi

echo
echo "5) Installing binary to /usr/local/bin/${BINARY_NAME}..."
cp "${BUILT_BIN}" "/usr/local/bin/${BINARY_NAME}"
chmod 755 "/usr/local/bin/${BINARY_NAME}"

echo
echo "6) Initializing database (pois.db)..."
if [[ -f "pois.db" ]]; then
  echo "   pois.db already exists; NOT overwriting."
else
  if [[ -f "migrations/0001_init.sql" ]]; then
    echo "   Applying migrations/0001_init.sql..."
    sqlite3 pois.db < migrations/0001_init.sql
  else
    echo "   WARNING: migrations/0001_init.sql not found."
  fi

  if [[ -f "migrations/0002_event_logging.sql" ]]; then
    echo "   Applying migrations/0002_event_logging.sql..."
    sqlite3 pois.db < migrations/0002_event_logging.sql
  else
    echo "   WARNING: migrations/0002_event_logging.sql not found."
  fi

  echo "   Database initialization complete."
fi

echo "   Current tables/views in pois.db:"
sqlite3 pois.db "SELECT name, type FROM sqlite_master WHERE type IN ('table','view');" || true

echo
echo "7) Creating service user '${SERVICE_USER}' (if needed)..."
if id -u "${SERVICE_USER}" >/dev/null 2>&1; then
  echo "   User ${SERVICE_USER} already exists."
else
  useradd -r -s /usr/sbin/nologin "${SERVICE_USER}"
  echo "   User ${SERVICE_USER} created."
fi

echo
echo "8) Setting ownership of ${INSTALL_DIR} to ${SERVICE_USER}:${SERVICE_USER}..."
chown -R "${SERVICE_USER}":"${SERVICE_USER}" "${INSTALL_DIR}"

echo
echo "9) Creating systemd service /etc/systemd/system/${SERVICE_NAME}.service..."

cat >/etc/systemd/system/${SERVICE_NAME}.service <<EOF
[Unit]
Description=POIS ESAM Server
After=network.target

[Service]
Type=simple
WorkingDirectory=${INSTALL_DIR}
ExecStart=/usr/local/bin/${BINARY_NAME} --port ${PORT}
Environment=RUST_LOG=info
Environment=POIS_STORE_RAW_PAYLOADS=true
Restart=on-failure
RestartSec=5
User=${SERVICE_USER}
Group=${SERVICE_USER}
NoNewPrivileges=true
ProtectSystem=full
ProtectHome=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF

echo
echo "10) Reloading systemd and starting service..."
systemctl daemon-reload
systemctl enable "${SERVICE_NAME}"
systemctl restart "${SERVICE_NAME}"

echo
echo "11) Opening firewall port ${PORT} (if ufw is active)..."
if command -v ufw >/dev/null 2>&1; then
  ufw allow "${PORT}"/tcp || true
fi

echo
echo "=== POIS installation complete ==="
systemctl --no-pager status "${SERVICE_NAME}" || true

IP_ADDR=$(hostname -I 2>/dev/null | awk '{print $1}')

echo
echo "You should now be able to access:"
echo "  UI:     http://${IP_ADDR:-<server-ip>}:${PORT}/"
echo "  Events: http://${IP_ADDR:-<server-ip>}:${PORT}/events.html"
echo
echo "Logs (journalctl):"
echo "  sudo journalctl -u ${SERVICE_NAME} -f"
echo
echo "To change port or user later, edit:"
echo "  /etc/systemd/system/${SERVICE_NAME}.service"
echo "and then run:"
echo "  sudo systemctl daemon-reload && sudo systemctl restart ${SERVICE_NAME}"

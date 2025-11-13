#!/usr/bin/env bash
#
# POIS ESAM Server installer for Ubuntu 24.04
# - Installs system dependencies & Rust
# - Clones/updates repo into /opt/pois
# - Builds release binary
# - Initializes SQLite database with migrations
# - Creates initial admin user
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
apt install -y build-essential pkg-config libssl-dev sqlite3 libsqlite3-dev curl git python3 python3-pip

# Install bcrypt for password hashing
echo "   Installing python3-bcrypt for password hashing..."
apt install -y python3-bcrypt || pip3 install bcrypt

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
DB_EXISTED=false
if [[ -f "pois.db" ]]; then
  echo "   pois.db already exists; NOT overwriting."
  DB_EXISTED=true
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

  if [[ -f "migrations/0003_add_jwt_auth.sql" ]]; then
    echo "   Applying migrations/0003_add_jwt_auth.sql..."
    sqlite3 pois.db < migrations/0003_add_jwt_auth.sql
  else
    echo "   WARNING: migrations/0003_add_jwt_auth.sql not found."
  fi

  if [[ -f "migrations/0004_add_multitenancy.sql" ]]; then
    echo "   Applying migrations/0004_add_multitenancy.sql..."
    sqlite3 pois.db < migrations/0004_add_multitenancy.sql
  else
    echo "   WARNING: migrations/0004_add_multitenancy.sql not found."
  fi

  echo "   Database initialization complete."
fi

echo "   Current tables/views in pois.db:"
sqlite3 pois.db "SELECT name, type FROM sqlite_master WHERE type IN ('table','view');" || true

# Create admin user if this is a fresh database
if [[ "$DB_EXISTED" == "false" ]]; then
  echo
  echo "=== Creating admin user ==="
  
  # Prompt for admin credentials
  DEFAULT_ADMIN_USER="admin"
  read -rp "Enter admin username [${DEFAULT_ADMIN_USER}]: " ADMIN_USER
  ADMIN_USER=${ADMIN_USER:-$DEFAULT_ADMIN_USER}
  
  DEFAULT_ADMIN_EMAIL="admin@example.com"
  read -rp "Enter admin email [${DEFAULT_ADMIN_EMAIL}]: " ADMIN_EMAIL
  ADMIN_EMAIL=${ADMIN_EMAIL:-$DEFAULT_ADMIN_EMAIL}
  
  # Prompt for password (with confirmation)
  while true; do
    read -rsp "Enter admin password: " ADMIN_PASSWORD
    echo
    read -rsp "Confirm admin password: " ADMIN_PASSWORD_CONFIRM
    echo
    
    if [[ "$ADMIN_PASSWORD" == "$ADMIN_PASSWORD_CONFIRM" ]]; then
      if [[ ${#ADMIN_PASSWORD} -lt 8 ]]; then
        echo "ERROR: Password must be at least 8 characters long."
        continue
      fi
      break
    else
      echo "ERROR: Passwords do not match. Please try again."
    fi
  done
  
  # Generate bcrypt hash
  echo "   Hashing password..."
  PASSWORD_HASH=$(python3 -c "import bcrypt; print(bcrypt.hashpw('${ADMIN_PASSWORD}'.encode('utf-8'), bcrypt.gensalt()).decode('utf-8'))")
  
  # Insert admin user into database
  echo "   Creating admin user in database..."
  sqlite3 pois.db <<SQL
INSERT INTO users (username, email, password_hash, created_at, updated_at) 
VALUES ('${ADMIN_USER}', '${ADMIN_EMAIL}', '${PASSWORD_HASH}', datetime('now'), datetime('now'));
SQL
  
  echo "   ✓ Admin user '${ADMIN_USER}' created successfully!"
fi

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
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  POIS is now running!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo
echo "  Login:  http://${IP_ADDR:-<server-ip>}:${PORT}/login.html"
echo "  Admin:  http://${IP_ADDR:-<server-ip>}:${PORT}/"
echo "  Events: http://${IP_ADDR:-<server-ip>}:${PORT}/events.html"
echo
if [[ "$DB_EXISTED" == "false" ]]; then
  echo "  Admin username: ${ADMIN_USER}"
  echo "  Admin email:    ${ADMIN_EMAIL}"
  echo
fi
echo "  Logs: sudo journalctl -u ${SERVICE_NAME} -f"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
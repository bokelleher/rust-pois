#!/usr/bin/env bash
#
# POIS ESAM Server installer for Ubuntu 24.04
# - Handles both FRESH INSTALLS and UPGRADES
# - Installs system dependencies & Rust
# - Clones/updates repo into /opt/pois
# - Builds release binary
# - Rust app handles all database migrations automatically on startup
# - Creates initial admin user (fresh installs only)
# - Sets up and starts a systemd service
#
# Usage:
#   Fresh install:  sudo ./install.sh
#   Upgrade:        sudo ./install.sh
#
# The script detects whether this is fresh or upgrade automatically.

set -euo pipefail

# === Static configuration ===
REPO_URL="https://github.com/bokelleher/rust-pois.git"
INSTALL_DIR="/opt/pois"
SERVICE_NAME="pois"
BINARY_NAME="pois"
DB_PATH="${INSTALL_DIR}/pois.db"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  POIS ESAM Server Installer for Ubuntu 24.04"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# --- Root check ---
if [[ "$(id -u)" -ne 0 ]]; then
  echo "❌ ERROR: Please run this script as root (e.g.: sudo $0)"
  exit 1
fi

# --- Detect if this is an upgrade or fresh install ---
IS_UPGRADE=false
if [[ -f "${DB_PATH}" ]]; then
  IS_UPGRADE=true
  echo "📦 Detected existing installation - UPGRADE MODE"
  echo ""
  read -rp "⚠️  This will upgrade your existing POIS installation. Continue? [y/N]: " CONFIRM_UPGRADE
  CONFIRM_UPGRADE=${CONFIRM_UPGRADE:-N}
  if [[ ! "${CONFIRM_UPGRADE}" =~ ^[Yy]$ ]]; then
    echo "Installation cancelled."
    exit 0
  fi
else
  echo "🆕 No existing installation detected - FRESH INSTALL MODE"
fi

echo ""

# --- Prompt for service user (skip if exists) ---
DEFAULT_USER="pois"
if id -u "${DEFAULT_USER}" >/dev/null 2>&1; then
  SERVICE_USER="${DEFAULT_USER}"
  echo "✓ Using existing service user: ${SERVICE_USER}"
else
  read -rp "Enter system user to run POIS as [${DEFAULT_USER}]: " SERVICE_USER
  SERVICE_USER=${SERVICE_USER:-$DEFAULT_USER}
  echo "→ Will create service user: ${SERVICE_USER}"
fi

# --- Prompt for port (read from existing systemd unit if upgrade) ---
DEFAULT_PORT=8080
if [[ "$IS_UPGRADE" == true ]] && [[ -f "/etc/systemd/system/${SERVICE_NAME}.service" ]]; then
  # Try to extract port from existing systemd unit
  EXISTING_PORT=$(grep -oP 'ExecStart=.*--port\s+\K\d+' "/etc/systemd/system/${SERVICE_NAME}.service" 2>/dev/null || echo "")
  if [[ -n "$EXISTING_PORT" ]]; then
    DEFAULT_PORT=$EXISTING_PORT
    echo "✓ Using existing port: ${DEFAULT_PORT}"
    PORT=$DEFAULT_PORT
  else
    read -rp "Enter port for POIS to listen on [${DEFAULT_PORT}]: " PORT
    PORT=${PORT:-$DEFAULT_PORT}
  fi
else
  read -rp "Enter port for POIS to listen on [${DEFAULT_PORT}]: " PORT
  PORT=${PORT:-$DEFAULT_PORT}
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Installation Summary:"
echo "  Mode:         $([ "$IS_UPGRADE" == true ] && echo "UPGRADE" || echo "FRESH INSTALL")"
echo "  Install Dir:  ${INSTALL_DIR}"
echo "  Service User: ${SERVICE_USER}"
echo "  Port:         ${PORT}"
echo "  Database:     ${DB_PATH}"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

# --- Stop service if running (upgrades only) ---
if [[ "$IS_UPGRADE" == true ]]; then
  echo "🛑 Stopping existing service..."
  systemctl stop "${SERVICE_NAME}" || true
  echo ""
fi

# --- Backup database if upgrading ---
if [[ "$IS_UPGRADE" == true ]]; then
  BACKUP_PATH="${DB_PATH}.backup.$(date +%Y%m%d_%H%M%S)"
  echo "💾 Backing up database..."
  echo "   ${DB_PATH} → ${BACKUP_PATH}"
  cp "${DB_PATH}" "${BACKUP_PATH}"
  echo "   ✓ Backup complete"
  echo ""
fi

echo "📦 Step 1/8: Installing system dependencies..."
apt update -qq
apt install -y build-essential pkg-config libssl-dev sqlite3 libsqlite3-dev curl git python3 python3-pip >/dev/null 2>&1

# Install bcrypt for password hashing (fresh installs only)
if [[ "$IS_UPGRADE" == false ]]; then
  echo "   → Installing python3-bcrypt for password hashing..."
  apt install -y python3-bcrypt >/dev/null 2>&1 || pip3 install bcrypt >/dev/null 2>&1
fi

# Install ufw if not present
if ! dpkg -s ufw >/dev/null 2>&1; then
  apt install -y ufw >/dev/null 2>&1 || true
fi
echo "   ✓ Dependencies installed"
echo ""

echo "🦀 Step 2/8: Installing Rust toolchain..."
if ! command -v cargo >/dev/null 2>&1; then
  echo "   → Installing Rust..."
  curl -sSf https://sh.rustup.rs | sh -s -- -y >/dev/null 2>&1
  export PATH="/root/.cargo/bin:${PATH}"
  echo "   ✓ Rust installed"
else
  export PATH="/root/.cargo/bin:${PATH}"
  echo "   ✓ Rust already installed ($(rustc --version))"
fi
echo ""

echo "📥 Step 3/8: Cloning/updating repository..."
if [[ -d "${INSTALL_DIR}/.git" ]]; then
  echo "   → Updating existing repository..."
  git -C "${INSTALL_DIR}" fetch --all -q
  # Try main, then master as fallback
  if git -C "${INSTALL_DIR}" rev-parse origin/main >/dev/null 2>&1; then
    git -C "${INSTALL_DIR}" reset --hard origin/main -q
  elif git -C "${INSTALL_DIR}" rev-parse origin/master >/dev/null 2>&1; then
    git -C "${INSTALL_DIR}" reset --hard origin/master -q
  else
    echo "   ⚠️  WARNING: Could not find origin/main or origin/master"
  fi
  echo "   ✓ Repository updated"
else
  echo "   → Cloning repository..."
  mkdir -p "$(dirname "${INSTALL_DIR}")"
  git clone "${REPO_URL}" "${INSTALL_DIR}" -q
  echo "   ✓ Repository cloned"
fi
echo ""

cd "${INSTALL_DIR}"

echo "🔨 Step 4/8: Building POIS (this may take a few minutes)..."
cargo build --release 2>&1 | grep -E "(Compiling|Finished)" || true

# Determine built binary
if [[ -f "target/release/pois-esam-server" ]]; then
  BUILT_BIN="target/release/pois-esam-server"
else
  echo "❌ ERROR: Expected target/release/pois-esam-server not found."
  echo "   Check Cargo.toml/bin name or build output above."
  exit 1
fi
echo "   ✓ Build complete"
echo ""

echo "📦 Step 5/8: Installing binary..."
cp "${BUILT_BIN}" "/usr/local/bin/${BINARY_NAME}"
chmod 755 "/usr/local/bin/${BINARY_NAME}"
echo "   ✓ Binary installed to /usr/local/bin/${BINARY_NAME}"
echo ""

# --- Create service user if needed ---
echo "👤 Step 6/8: Setting up service user..."
if id -u "${SERVICE_USER}" >/dev/null 2>&1; then
  echo "   ✓ User ${SERVICE_USER} already exists"
else
  useradd -r -s /usr/sbin/nologin "${SERVICE_USER}"
  echo "   ✓ User ${SERVICE_USER} created"
fi

chown -R "${SERVICE_USER}":"${SERVICE_USER}" "${INSTALL_DIR}"
echo "   ✓ Ownership set: ${SERVICE_USER}:${SERVICE_USER}"
echo ""

# --- Create admin user (fresh installs only) ---
if [[ "$IS_UPGRADE" == false ]]; then
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo "  👤 Admin User Setup"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo ""
  echo "Create your admin user account:"
  echo ""
  
  # Prompt for admin credentials
  DEFAULT_ADMIN_USER="admin"
  read -rp "  Admin username [${DEFAULT_ADMIN_USER}]: " ADMIN_USER
  ADMIN_USER=${ADMIN_USER:-$DEFAULT_ADMIN_USER}
  
  DEFAULT_ADMIN_EMAIL="admin@example.com"
  read -rp "  Admin email [${DEFAULT_ADMIN_EMAIL}]: " ADMIN_EMAIL
  ADMIN_EMAIL=${ADMIN_EMAIL:-$DEFAULT_ADMIN_EMAIL}
  
  # Prompt for password (with confirmation)
  while true; do
    read -rsp "  Admin password (min 8 chars): " ADMIN_PASSWORD
    echo
    
    if [[ ${#ADMIN_PASSWORD} -lt 8 ]]; then
      echo "  ❌ Password must be at least 8 characters long."
      echo ""
      continue
    fi
    
    read -rsp "  Confirm password: " ADMIN_PASSWORD_CONFIRM
    echo
    
    if [[ "$ADMIN_PASSWORD" != "$ADMIN_PASSWORD_CONFIRM" ]]; then
      echo "  ❌ Passwords do not match. Please try again."
      echo ""
      continue
    fi
    
    break
  done
  
  echo ""
  echo "  → Hashing password..."
  PASSWORD_HASH=$(python3 -c "import bcrypt; print(bcrypt.hashpw('${ADMIN_PASSWORD}'.encode('utf-8'), bcrypt.gensalt()).decode('utf-8'))")
  
  # Store admin credentials for post-install display
  ADMIN_USERNAME_DISPLAY="${ADMIN_USER}"
  ADMIN_EMAIL_DISPLAY="${ADMIN_EMAIL}"
  
  echo "  ✓ Admin user configured (will be created on first startup)"
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo ""
fi

# --- Create systemd service ---
echo "⚙️  Step 7/8: Creating systemd service..."

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

systemctl daemon-reload
systemctl enable "${SERVICE_NAME}" >/dev/null 2>&1
echo "   ✓ Systemd service configured"
echo ""

# --- Create admin user in database if fresh install ---
if [[ "$IS_UPGRADE" == false ]]; then
  echo "🗄️  Creating admin user in database..."
  echo "   → Starting POIS temporarily to run migrations..."
  
  # Start service temporarily to let it create database and run migrations
  systemctl start "${SERVICE_NAME}"
  sleep 3
  
  # Insert admin user
  sudo -u "${SERVICE_USER}" sqlite3 "${DB_PATH}" <<SQL
INSERT INTO users (username, email, password_hash, created_at, updated_at) 
VALUES ('${ADMIN_USER}', '${ADMIN_EMAIL}', '${PASSWORD_HASH}', datetime('now'), datetime('now'));
SQL
  
  # Stop service (will be restarted properly in next step)
  systemctl stop "${SERVICE_NAME}"
  sleep 1
  
  echo "   ✓ Admin user created in database"
  echo ""
fi

# --- Start service ---
echo "🚀 Step 8/8: Starting POIS service..."
systemctl start "${SERVICE_NAME}"
sleep 2

if systemctl is-active --quiet "${SERVICE_NAME}"; then
  echo "   ✓ Service started successfully"
else
  echo "   ⚠️  Service may have issues. Check logs:"
  echo "      sudo journalctl -u ${SERVICE_NAME} -n 50"
fi
echo ""

# --- Firewall ---
if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
  echo "🔥 Configuring firewall..."
  ufw allow "${PORT}"/tcp >/dev/null 2>&1 || true
  echo "   ✓ Port ${PORT}/tcp allowed"
  echo ""
fi

# --- Get IP address ---
IP_ADDR=$(hostname -I 2>/dev/null | awk '{print $1}')
IP_ADDR=${IP_ADDR:-"<server-ip>"}

# --- Success message ---
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  ✅ POIS Installation Complete!"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

if [[ "$IS_UPGRADE" == true ]]; then
  echo "📋 UPGRADE SUMMARY:"
  echo "   • Binary updated to latest version"
  echo "   • Database migrations applied automatically"
  echo "   • Service restarted"
  echo "   • Backup saved: ${BACKUP_PATH}"
  echo ""
  echo "🌐 Access your upgraded POIS instance:"
else
  echo "📋 FRESH INSTALL SUMMARY:"
  echo ""
  echo "👤 Admin Credentials:"
  echo "   Username: ${ADMIN_USERNAME_DISPLAY}"
  echo "   Email:    ${ADMIN_EMAIL_DISPLAY}"
  echo "   Password: (the one you just entered)"
  echo ""
  echo "🌐 Access POIS:"
fi

echo "   Login:  http://${IP_ADDR}:${PORT}/login.html"
echo "   Admin:  http://${IP_ADDR}:${PORT}/"
echo "   Events: http://${IP_ADDR}:${PORT}/events.html"
echo ""
echo "📊 Useful Commands:"
echo "   View logs:       sudo journalctl -u ${SERVICE_NAME} -f"
echo "   Service status:  sudo systemctl status ${SERVICE_NAME}"
echo "   Restart:         sudo systemctl restart ${SERVICE_NAME}"
if [[ "$IS_UPGRADE" == true ]]; then
  echo "   Restore backup:  cp ${BACKUP_PATH} ${DB_PATH}"
fi
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

if [[ "$IS_UPGRADE" == false ]]; then
  echo "💡 Next Steps:"
  echo "   1. Log in at http://${IP_ADDR}:${PORT}/login.html"
  echo "   2. Generate an API token from the Tokens page"
  echo "   3. Start creating channels and rules!"
  echo ""
fi

echo "📚 Documentation: https://github.com/bokelleher/rust-pois"
echo ""

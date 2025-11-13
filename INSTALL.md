# POIS Installation Guide (Ubuntu 24.04)

This guide covers installing, verifying, and maintaining the POIS ESAM Server.

---

## ⚙️ System Requirements

- Ubuntu 24.04 LTS (x86_64 or ARM64)
- `curl`, `git`, `sqlite3`, `python3`
- Rust toolchain (installed automatically)

---

## 🪄 One-Shot Installation

From the repo root:

```bash
chmod +x install.sh
sudo ./install.sh
```

You will be prompted for:

- **System user** (default: `pois`)
- **HTTP port** (default: `8080`)
- **Admin username** (default: `admin`) - *for fresh installs only*
- **Admin email** (default: `admin@example.com`) - *for fresh installs only*
- **Admin password** (minimum 8 characters) - *for fresh installs only*

The installer will:

1. Install dependencies (`build-essential`, `libssl-dev`, `sqlite3`, `python3-bcrypt`, etc.)
2. Build the release binary
3. Copy it to `/usr/local/bin/pois`
4. Create `/opt/pois/pois.db` with migrations
5. Create admin user (fresh installs only)
6. Set up `/etc/systemd/system/pois.service`
7. Start and enable the service
8. Open the firewall port (if UFW is enabled)

---

## ✅ Post-Install Verification

Check that the service is running:

```bash
sudo systemctl status pois
```

Check the database tables:

```bash
cd /opt/pois
sqlite3 pois.db ".tables"
```

Expected tables (v3.0.3+):
- `channels`
- `rules`
- `events`
- `users`
- `api_tokens`

### Login to Web UI

Navigate to the login page:

```
http://<server-ip>:8080/login.html
```

Enter your admin credentials (set during installation).

### Generate API Token

After logging in:

1. Navigate to the **Tokens** page
2. Click **Generate Token**
3. Copy the token for use in API scripts
4. Use in API calls: `Authorization: Bearer YOUR_TOKEN`

---

## 🔐 Authentication (v3.0.3+)

POIS uses JWT-based authentication. All API endpoints require a valid Bearer token.

### For Fresh Installations

The installer creates an admin user during setup. Use these credentials to log in at `/login.html`.

### For Existing Installations

If upgrading from v2.x, the first user you create will be associated with all existing channels, rules, and events.

### Managing Users

- **Via Web UI**: Login and navigate to the Users page (admin only)
- **Via Database**: Direct SQLite access (not recommended for production)

---

## 🧑‍💻 Developer Setup

Clone locally and use the provided `Makefile`:

```bash
make run-dev
```

This runs the app on `localhost:18080` with a temporary `pois.dev.db`.

**Note**: For development, you'll need to create an admin user manually:

```bash
sqlite3 pois.dev.db
INSERT INTO users (username, email, password_hash, created_at, updated_at) 
VALUES ('admin', 'admin@localhost', 'BCRYPT_HASH', datetime('now'), datetime('now'));
.quit
```

Or run `./install.sh` once to seed the database, then use `make run-dev`.

---

## 🔁 Factory Reset (Wipe + Reseed)

To reset to a fresh state **without reinstalling**:

```bash
sudo ./factory_reset.sh
```

It will:

1. Stop the service
2. Delete `/opt/pois/pois.db`
3. Restart and automatically recreate the schema

⚠️ **Warning:** This permanently deletes all channels, rules, events, **and users**.

**After factory reset**, you'll need to create a new admin user:

```bash
cd /opt/pois
sudo -u pois sqlite3 pois.db
-- Insert admin user with bcrypt hash
INSERT INTO users (username, email, password_hash, created_at, updated_at) 
VALUES ('admin', 'admin@example.com', 'YOUR_BCRYPT_HASH', datetime('now'), datetime('now'));
.quit
```

Or re-run the installer to recreate the admin user.

---

## 🔄 Updating from v2.x to v3.0.3+

**Breaking Change**: v3.0.3 introduces authentication. All API endpoints now require JWT tokens.

### Update Steps

1. **Backup your database**:
   ```bash
   cp /opt/pois/pois.db /opt/pois/pois.db.v2.backup
   ```

2. **Pull latest code and reinstall**:
   ```bash
   cd /opt/pois
   git pull
   sudo systemctl stop pois
   cd /path/to/rust-pois
   sudo ./install.sh
   ```

3. **Migrations run automatically** on startup:
   - `0003_add_jwt_auth.sql` - Adds users and api_tokens tables
   - `0004_add_multitenancy.sql` - Adds user_id to channels, rules, events

4. **Create admin user** (installer will prompt if database is fresh)

5. **Update API scripts** with new authentication:
   ```bash
   # Old (v2.1.0)
   curl -H "Authorization: Bearer dev-token" http://localhost:8080/api/channels
   
   # New (v3.0.3)
   curl -H "Authorization: Bearer YOUR_JWT_TOKEN" https://localhost:8080/api/channels
   ```

---

## 🧹 Uninstallation

```bash
sudo ./uninstall.sh
```

You will be prompted whether to remove the service user, app directory, and binary.

---

## 🔍 Health & Logs

| Purpose | Command |
|----------|----------|
| View logs | `sudo journalctl -u pois -f` |
| Health check | `curl http://localhost:8080/healthz` |
| Database inspect | `sqlite3 /opt/pois/pois.db ".tables"` |
| Check users | `sqlite3 /opt/pois/pois.db "SELECT id, username, email FROM users;"` |

---

## 🔒 Security Notes

- **Change JWT Secret**: Set a strong `POIS_JWT_SECRET` environment variable in production
- **Use HTTPS**: Always use TLS in production (configure via `POIS_TLS_CERT` and `POIS_TLS_KEY`)
- **Strong Passwords**: Enforce strong password policies for user accounts
- **Token Rotation**: Regularly rotate API tokens
- **Firewall**: Restrict access to port 8080 to trusted networks only

---

## 📄 License

MIT License © 2025 Bo Kelleher

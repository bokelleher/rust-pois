# POIS Installation Guide (Ubuntu 24.04)

This guide covers installing, verifying, and maintaining the POIS ESAM Server.

---

## ⚙️ System Requirements

- Ubuntu 24.04 LTS (x86_64 or ARM64)
- `curl`, `git`, `sqlite3`
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

The installer will:

1. Install dependencies (`build-essential`, `libssl-dev`, `sqlite3`, etc.)
2. Build the release binary
3. Copy it to `/usr/local/bin/pois`
4. Create `/opt/pois/pois.db`
5. Set up `/etc/systemd/system/pois.service`
6. Start and enable the service
7. Open the firewall port (if UFW is enabled)

---

## ✅ Post-Install Verification

Check that the service is running:

```bash
sudo systemctl status pois
```

Confirm the default data:

```bash
cd /opt/pois
sqlite3 pois.db "SELECT id, name FROM channels;"
sqlite3 pois.db "SELECT channel_id, name, action FROM rules;"
```

Expected output:

| Table | Example Data |
|--------|---------------|
| channels | `(1, 'default')` |
| rules | `(1, 'Default noop', 'noop')` |

Visit in a browser:

```
http://<server-ip>:8080/
```

---

## 🧑‍💻 Developer Setup

Clone locally and use the provided `Makefile`:

```bash
make run-dev
```

This runs the app on `localhost:18080` with a temporary `pois.dev.db`.

---

## 🔁 Factory Reset (Wipe + Reseed)

To reset to a fresh state **without reinstalling**:

```bash
sudo ./factory_reset.sh
```

It will:

1. Stop the service
2. Delete `/opt/pois/pois.db`
3. Restart and automatically recreate the schema + default data

⚠️ **This permanently deletes all channels, rules, and logged events.**

---

## 🧹 Uninstallation

```bash
sudo ./uninstall.sh
```

You will be prompted whether to remove the service user, app directory, and binary.

---

## 📄 License

MIT License © 2025 Bo Kelleher

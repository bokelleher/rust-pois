# POIS ESAM Server

A lightweight, self-contained Rust server for **Program Opportunity Information Service (POIS)** and **ESAM (Event Signaling and Management)** workflows.  
It provides a REST/JSON API, simple web UI, and SQLite persistence for defining channels, rules, and ESAM event logs.

---

## 🚀 Features

- ESAM / SCTE-35 XML processing via REST endpoints
- Front-end web UI served directly from `/static`
- Channel and Rule management API with bearer authentication
- Automatic SQLite migrations
- Built-in default `default` channel + `Default noop` rule (seeded automatically)
- Event logging and monitoring
- TLS optional (via `POIS_TLS_CERT` / `POIS_TLS_KEY`)
- Zero external dependencies beyond SQLite and Rust
- One-shot install and uninstall scripts for Ubuntu 24.04

---

## 🧩 Environment Variables

| Variable | Description | Default |
|-----------|-------------|----------|
| `POIS_PORT` | Listening port for the web server | `8080` |
| `POIS_DB` | SQLite database URL (relative or absolute) | `sqlite://pois.db` |
| `POIS_ADMIN_TOKEN` | Bearer token for `/api` calls | `dev-token` |
| `POIS_TLS_CERT`, `POIS_TLS_KEY` | Optional TLS PEM paths | _unset_ |

These are injected automatically by the installer into the systemd unit.

---

## 🛠️ Quick Start (Production)

```bash
sudo apt update
sudo apt install git -y
git clone https://github.com/bokelleher/rust-pois.git
cd rust-pois
chmod +x install.sh
sudo ./install.sh
```

Follow prompts for:
- service user (default `pois`)
- port (default `8080`)

When complete:

```bash
sudo systemctl status pois
```

Visit the UI:

```
http://<server-ip>:8080/
```

---

## 🧑‍💻 Developer Shortcuts

```bash
make install      # wraps: sudo ./install.sh
make uninstall    # wraps: sudo ./uninstall.sh
make run-dev      # runs a dev server on localhost:18080 using pois.dev.db
make build        # cargo build --release
```

Override defaults if needed:

```bash
make run-dev DEV_PORT=19000
```

---

## 🧹 Factory Reset

To wipe the SQLite database and reseed the default channel/rule:

```bash
sudo ./factory_reset.sh
```

The script will:
1. Stop the `pois` service
2. Delete `/opt/pois/pois.db`
3. Restart the service — migrations + seed will run automatically

⚠️ **Warning:** This erases all channels, rules, and events.

---

## 🔍 Health & Logs

| Purpose | Command |
|----------|----------|
| View logs | `sudo journalctl -u pois -f` |
| Health check | `curl http://localhost:8080/healthz` |
| Database inspect | `sqlite3 /opt/pois/pois.db ".tables"` |

---

## 📦 Uninstall

```bash
sudo ./uninstall.sh
```

Removes the binary, systemd unit, and optional user/data directory.

---

## 🧾 License

MIT License © 2025 Bo Kelleher

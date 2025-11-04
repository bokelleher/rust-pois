# POIS Installation & Troubleshooting Guide

> Target platform: **Ubuntu 24.04 LTS**

---

## ⚙️ 1. Requirements

- Internet access
- `sudo` privileges
- Outbound access for package & cargo downloads

---

## 🚀 2. Quick Install (interactive)

```bash
git clone https://github.com/bokelleher/rust-pois.git
cd rust-pois
chmod +x install.sh
sudo ./install.sh
```

During setup you’ll be prompted for:
- **Service user** → runs the POIS process (default `pois`)
- **Listening port** → HTTP port to expose (default `8080`)

The installer automatically:
1. Installs dependencies (`build-essential`, `sqlite3`, `openssl`, `curl`, `git`)
2. Installs the Rust toolchain
3. Clones or updates POIS to `/opt/pois`
4. Builds the release binary
5. Applies SQL migrations to create `pois.db`
6. Creates a system user & systemd service
7. Starts POIS and opens the selected port in `ufw`

---

## 🧱 3. Directory Layout

| Path | Purpose |
|------|----------|
| `/opt/pois` | Application root |
| `/opt/pois/migrations/` | SQL schema migrations |
| `/opt/pois/pois.db` | SQLite database |
| `/usr/local/bin/pois` | Installed binary |
| `/etc/systemd/system/pois.service` | systemd unit file |

---

## 🧩 4. Managing the Service

| Task | Command |
|------|----------|
| Check status | `sudo systemctl status pois` |
| Restart | `sudo systemctl restart pois` |
| Stop | `sudo systemctl stop pois` |
| Enable at boot | `sudo systemctl enable pois` |
| View logs | `sudo journalctl -u pois -f` |

---

## 🌐 5. Default Endpoints

| Path | Description |
|------|--------------|
| `/` | Web UI |
| `/events.html` | Event viewer |
| `/api/...` | REST endpoints (if enabled) |

Visit in a browser:

```
http://<server-ip>:<port>/
```

---

## 🧾 6. Troubleshooting

| Symptom | Cause | Resolution |
|----------|--------|------------|
| `bash: ./pois: No such file` | Binary not built | Run `cargo build --release` |
| `cannot execute binary file` | Architecture mismatch | Rebuild on target host |
| `sqlite3: No such file or directory` | Missing migrations | Re-run install or apply SQL manually |
| Port not reachable | Firewall closed | `sudo ufw allow <port>/tcp` |
| Service fails to start | Incorrect `ExecStart` path | Edit `/etc/systemd/system/pois.service` and reload |
| Empty UI | Database uninitialized | Rerun migrations and restart service |

---

## 🧰 7. Maintenance

Update to latest build:
```bash
cd /opt/pois
sudo git pull
sudo cargo build --release
sudo systemctl restart pois
```

---

## 🏁 8. Verification Checklist

| Test | Expected Result |
|------|-----------------|
| `systemctl status pois` | active (running) |
| `curl http://localhost:<port>/` | returns HTML |
| `sqlite3 pois.db ".tables"` | shows channels, rules, esam_events |
| Browser `/events.html` | loads event log page |

---

## 🪪 License

See [LICENSE](LICENSE).

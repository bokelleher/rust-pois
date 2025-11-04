# POIS (Program Opportunity Information Service)

**POIS** is a lightweight Rust-based ESAM/POIS server for managing SCTE-224 and event-driven ad-break metadata.  
It provides a REST API, an HTML dashboard, and an embedded SQLite database for small deployments.

---

### 🚀 Quick Start (Ubuntu 24.04)

1. Clone this repository:

   ```bash
   git clone https://github.com/bokelleher/rust-pois.git
   cd rust-pois
   ```

2. Run the one-shot installer:

   ```bash
   chmod +x install.sh
   sudo ./install.sh
   ```

   The installer will:
   - Install system dependencies & Rust
   - Build POIS in release mode
   - Initialize the database
   - Ask which user to run under (default `pois`)
   - Ask which port to listen on (default `8080`)
   - Create a systemd service and start it automatically

3. When finished, visit:

   ```
   http://<server-ip>:<port>/
   ```

   Default UI path → `/`  
   Events page → `/events.html`

---

### 🧩 Service Management

| Action | Command |
|--------|----------|
| Check status | `sudo systemctl status pois` |
| Restart | `sudo systemctl restart pois` |
| Stop | `sudo systemctl stop pois` |
| View logs | `sudo journalctl -u pois -f` |

---

### 📜 Documentation

Full installation, configuration, and troubleshooting details are available in [INSTALL.md](INSTALL.md).

---

### 🪪 License

See [LICENSE](LICENSE).

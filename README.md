# 🎯 POIS - Placement Opportunity Information Service

A high-performance Rust-based service for processing SCTE-35 signaling in ESAM (Event Signaling and Management) workflows. Features a modern dark-themed web UI for real-time event monitoring and rule management.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

## ✨ Features

### Core Functionality
- ✅ **ESAM XML Processing** - Parse and process SignalProcessingEvent messages
- ✅ **SCTE-35 Support** - Full support for splice_insert, time_signal, and segmentation descriptors
- ✅ **Rule-Based Filtering** - Flexible JSON-based rules with pattern matching
- ✅ **Multi-Channel** - Manage multiple channels with independent rule sets
- ✅ **Event Logging** - Comprehensive logging of all ESAM requests and processing

### Web Interface
- 🎨 **Modern Dark Theme** - Gradient background with frosted glass panels
- 📊 **Real-Time Event Monitor** - Track ESAM requests, processing times, and rule matches
- 🔧 **SCTE-35 Builder** - Generate SCTE-35 messages for testing
- ⚙️ **Admin Panel** - Manage channels, rules, and configurations
- 📱 **Responsive Design** - Works on desktop, tablet, and mobile

### SCTE-35 Capabilities
- Parse splice commands: `splice_insert`, `time_signal`, `splice_null`
- Extract PTS times and splice event details
- Decode segmentation descriptors (types 0x00-0x51, 0x80+)
- Support for UPID types: Ad-ID, ISAN, TI, ADI, EIDR, etc.

## 🚀 Quick Start

### Prerequisites

- Rust 1.70 or higher
- SQLite 3.x

### Installation

```bash
# Clone the repository
git clone https://github.com/bokelleher/rust-pois.git
cd rust-pois

# Build
cargo build --release

# Run
./target/release/rust-pois
```

### Configuration

Set environment variables:

```bash
# Database location (default: sqlite://pois.db)
export POIS_DB="sqlite://pois.db"

# Admin API token (default: dev-token)
export POIS_ADMIN_TOKEN="your-secret-token"

# HTTP port (default: 8090)
export POIS_PORT=8090

# Optional: Enable HTTPS
export POIS_TLS_CERT="/path/to/cert.pem"
export POIS_TLS_KEY="/path/to/key.pem"
```

### Accessing the UI

Once running, access the web interface:

- **Admin Panel**: http://localhost:8090/static/admin.html
- **Event Monitor**: http://localhost:8090/static/events.html
- **SCTE-35 Builder**: http://localhost:8090/static/tools.html

**Note**: Set your bearer token in the top-right corner of each page.

## 📖 Usage

### 1. Create a Channel

```bash
curl -X POST "http://localhost:8090/api/channels" \
  -H "Authorization: Bearer dev-token" \
  -H "Content-Type: application/json" \
  -d '{"name": "my-channel"}'
```

Or use the Admin Panel UI.

### 2. Add Rules

Rules use JSON-based matching:

```json
{
  "name": "Filter Provider Ads",
  "priority": 100,
  "enabled": true,
  "match_expr": {
    "anyOf": [
      {"scte35.segmentation_type_id": "0x30"},
      {"scte35.segmentation_type_name": "Provider Advertisement Start"}
    ]
  },
  "action": "delete"
}
```

Actions:
- `noop` - Pass through unchanged
- `delete` - Filter out the signal  
- `replace` - Modify signal (provide replacement SCTE-35)

### 3. Send ESAM Requests

```bash
curl -X POST "http://localhost:8090/esam?channel=my-channel" \
  -H "Content-Type: application/xml" \
  -d '<SignalProcessingEvent xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1">
        <AcquiredSignal acquisitionSignalID="test-123">
          <sig:UTCPoint utcPoint="2024-11-04T10:00:00Z"/>
          <sig:SCTE35PointDescriptor>
            <sig:SCTE35Data>/DA0AAAAAAAA///wBQb+cr0AUAAeAhxDVUVJSAAAjn/PAAGlmbAICAAAAAAsoKGC</sig:SCTE35Data>
          </sig:SCTE35PointDescriptor>
        </AcquiredSignal>
      </SignalProcessingEvent>'
```

### 4. Monitor Events

View processed events in real-time:
- **Event Monitor UI**: http://localhost:8090/static/events.html
- **API**: `GET /api/events?limit=100`

## 🎨 Customization

The UI is fully customizable! See [CUSTOMIZATION.md](CUSTOMIZATION.md) for:

- Adding your own logo
- Changing color schemes
- Adjusting fonts and spacing
- Creating custom themes

**Quick Logo Change:**

```html
<!-- In events.html, tools.html, admin.html -->
<img src="/static/logo.png" alt="Your Company" class="logo">
```

## 🔌 API Reference

### Channels

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/channels` | List all channels |
| POST | `/api/channels` | Create a channel |
| PUT | `/api/channels/:id` | Update a channel |
| DELETE | `/api/channels/:id` | Delete a channel |

### Rules

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/channels/:id/rules` | List rules for channel |
| POST | `/api/channels/:id/rules` | Create a rule |
| PUT | `/api/rules/:id` | Update a rule |
| DELETE | `/api/rules/:id` | Delete a rule |

### Events

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/events` | List events (paginated) |
| GET | `/api/events/:id` | Get event details |
| GET | `/api/events/stats` | Get event statistics |

### ESAM

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/esam?channel=name` | Process ESAM SignalProcessingEvent |

**Query Parameters:**
- `channel` - Channel name (required)

## 🏗️ Architecture

```
┌─────────────────────────────────────────────┐
│              Web Interface                  │
│  (Admin Panel, Event Monitor, Tools)        │
└──────────────────┬──────────────────────────┘
                   │ HTTP/HTTPS
┌──────────────────▼──────────────────────────┐
│           Axum Web Server                   │
│  - REST API                                 │
│  - Bearer Token Auth                        │
│  - Static File Serving                      │
└──────────────────┬──────────────────────────┘
                   │
┌──────────────────▼──────────────────────────┐
│          POIS Core Engine                   │
│  - ESAM XML Parsing                         │
│  - SCTE-35 Decoding                         │
│  - Rule Matching                            │
│  - Event Logging                            │
└──────────────────┬──────────────────────────┘
                   │
┌──────────────────▼──────────────────────────┐
│            SQLite Database                  │
│  - Channels & Rules                         │
│  - Event History                            │
└─────────────────────────────────────────────┘
```

## 📦 Dependencies

### Rust Crates

- `axum` - Web framework
- `tokio` - Async runtime
- `sqlx` - Database access
- `serde` - Serialization
- `quick-xml` - XML parsing
- `base64` - Base64 encoding/decoding
- `tower-http` - HTTP middleware

### Frontend

- Vanilla JavaScript (ES6+)
- Preact 10.24+ (admin panel only)
- No build step required!

## 🧪 Testing

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_parse_splice_insert
```

## 📝 Development

### Project Structure

```
rust-pois/
├── src/
│   ├── main.rs           # Entry point, web server
│   ├── scte35.rs         # SCTE-35 parsing
│   ├── esam.rs           # ESAM XML handling  
│   └── rules.rs          # Rule matching logic
├── static/
│   ├── app.css           # Stylesheet (dark theme)
│   ├── app.js            # Shared JavaScript
│   ├── admin.html        # Admin panel
│   ├── events.html       # Event monitor
│   └── tools.html        # SCTE-35 builder
├── migrations/           # Database migrations
└── Cargo.toml           # Rust dependencies
```

### Adding Features

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on:
- Code style
- Testing requirements
- Pull request process

## 🐛 Troubleshooting

**Port already in use:**
```bash
export POIS_PORT=8091
./target/release/rust-pois
```

**Database locked:**
```bash
# Stop the service
pkill rust-pois

# Check for stale connections
lsof pois.db

# Restart
./target/release/rust-pois
```

**UI not loading:**
- Clear browser cache (Ctrl+Shift+R)
- Check static files exist in `static/` directory
- Verify `static/` is in the same directory as binary

**API returns 401:**
- Check bearer token is set in UI (top-right)
- Verify `POIS_ADMIN_TOKEN` environment variable
- Default token is `dev-token`

## 📚 Resources

- **SCTE-35 Spec**: [SCTE 35 2023](https://www.scte.org/standards/library/)
- **ESAM Spec**: [SCTE 130-5](https://www.scte.org/standards/library/)
- **Rust Book**: [doc.rust-lang.org/book](https://doc.rust-lang.org/book/)

## 🤝 Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## 📄 License

This project is licensed under the MIT License - see [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- UI design inspired by modern dark-themed web applications
- SCTE-35 parsing based on SCTE standards
- Community contributions and feedback

## 📞 Support

- **Issues**: [GitHub Issues](https://github.com/bokelleher/rust-pois/issues)
- **Discussions**: [GitHub Discussions](https://github.com/bokelleher/rust-pois/discussions)

---

Made with ❤️ by the POIS community

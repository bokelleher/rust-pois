# POIS - Program Operations Integration Server

A high-performance ESAM (Event Signaling and Management) server for processing SCTE-35 signals with advanced rule-based filtering, real-time monitoring, and comprehensive logging capabilities.

## Features

- **ESAM Signal Processing**: Full SCTE-35 command parsing with support for segmentation descriptors, UPID decoding, and PTS time extraction
- **Rule-Based Filtering**: Flexible JSON-based rules for signal filtering, modification, and routing
- **Real-Time Monitoring**: Web-based dashboard for monitoring events, performance metrics, and system health
- **Advanced SCTE-35 Decoding**: Human-readable segmentation type names, UPID type decoding, and comprehensive signal analysis
- **TLS/HTTPS Support**: Production-ready with Let's Encrypt certificate integration
- **SQLite Database**: Lightweight, embedded database for event logging and configuration
- **REST API**: Complete API for configuration management and event querying
- **Systemd Integration**: Production deployment with automatic startup and restart capabilities

## Quick Start

### Development Setup

1. **Clone and build**:
   ```bash
   git clone <repository-url>
   cd pois
   cargo build
   ```

2. **Configure environment**:
   ```bash
   export POIS_DB=sqlite://pois.db
   export POIS_ADMIN_TOKEN='dev-token'
   export POIS_PORT=8080
   ```

3. **Start the server**:
   ```bash
   cargo run
   ```

4. **Access the web interface**:
   - Open http://localhost:8080/
   - Enter your admin token (`dev-token`) in the toolbar
   - Navigate between Events, Channels & Rules, and SCTE-35 Builder

### Initial Configuration

5. **Create a default channel**:
   ```bash
   curl -X POST http://localhost:8080/api/channels \
     -H "Authorization: Bearer dev-token" \
     -H "Content-Type: application/json" \
     -d '{"name":"default"}'
   ```

6. **Add a sample rule** (delete splice_insert commands):
   ```bash
   curl -X POST http://localhost:8080/api/channels/1/rules \
     -H "Authorization: Bearer dev-token" \
     -H "Content-Type: application/json" \
     -d '{
       "name": "Delete splice_insert",
       "priority": -1,
       "enabled": true,
       "match_json": {"anyOf": [{"scte35.command": "splice_insert"}]},
       "action": "delete",
       "params_json": {}
     }'
   ```

### Testing ESAM Endpoint

7. **Send a test ESAM signal**:
   ```bash
   curl -X POST http://localhost:8080/esam?channel=default \
     -H 'Content-Type: application/xml' \
     -d '<SignalProcessingEvent xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1" xmlns:sig="urn:cablelabs:md:xsd:signaling:3.0">
           <AcquiredSignal acquisitionSignalID="abc-123">
             <sig:UTCPoint utcPoint="2012-09-18T10:14:34Z"/>
             <sig:BinaryData signalType="SCTE35">BASE64_SCTE35_DATA_HERE</sig:BinaryData>
           </AcquiredSignal>
         </SignalProcessingEvent>'
   ```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `POIS_DB` | SQLite database path | `sqlite://pois.db` |
| `POIS_ADMIN_TOKEN` | Admin API token | Required |
| `POIS_PORT` | Server port | `8080` |
| `POIS_TLS_CERT` | TLS certificate path | Optional |
| `POIS_TLS_KEY` | TLS private key path | Optional |
| `RUST_LOG` | Logging level | `info` |

### Production TLS Configuration

For HTTPS with Let's Encrypt certificates:

```bash
export POIS_TLS_CERT=/etc/letsencrypt/live/your-domain.com/fullchain.pem
export POIS_TLS_KEY=/etc/letsencrypt/live/your-domain.com/privkey.pem
export POIS_PORT=8090
```

## Production Deployment

### Systemd Service Setup

1. **Build release binary**:
   ```bash
   cd /opt/pois
   cargo build --release
   ```

2. **Create systemd service**:
   ```bash
   sudo tee /etc/systemd/system/pois.service > /dev/null << 'EOF'
   [Unit]
   Description=POIS ESAM Server
   After=network.target network-online.target
   Wants=network-online.target

   [Service]
   Type=simple
   User=root
   Group=root
   WorkingDirectory=/opt/pois
   ExecStart=/opt/pois/target/release/pois-esam-server
   Restart=always
   RestartSec=5
   StandardOutput=journal
   StandardError=journal

   Environment=POIS_TLS_CERT=/etc/letsencrypt/live/your-domain.com/fullchain.pem
   Environment=POIS_TLS_KEY=/etc/letsencrypt/live/your-domain.com/privkey.pem
   Environment=POIS_PORT=8090
   Environment=POIS_ADMIN_TOKEN=your-secure-token-here
   Environment=RUST_LOG=info

   NoNewPrivileges=true
   ProtectSystem=strict
   ReadWritePaths=/opt/pois
   ProtectHome=true
   LimitNOFILE=65536

   [Install]
   WantedBy=multi-user.target
   EOF
   ```

3. **Enable and start service**:
   ```bash
   sudo systemctl daemon-reload
   sudo systemctl enable pois.service
   sudo systemctl start pois.service
   ```

4. **Monitor service**:
   ```bash
   sudo systemctl status pois.service
   sudo journalctl -u pois.service -f
   ```

## API Reference

### Channels

- `GET /api/channels` - List all channels
- `POST /api/channels` - Create a new channel
- `DELETE /api/channels/{id}` - Delete a channel

### Rules

- `GET /api/channels/{id}/rules` - List rules for a channel
- `POST /api/channels/{id}/rules` - Create a new rule
- `PUT /api/rules/{id}` - Update a rule
- `DELETE /api/rules/{id}` - Delete a rule

### Events

- `GET /api/events` - List events with filtering and pagination
- `GET /api/events/{id}` - Get detailed event information
- `GET /api/events/stats` - Get event statistics

### ESAM Endpoint

- `POST /esam?channel={channel_name}` - Process ESAM SignalProcessingEvent

## Rule Configuration

Rules use JSON-based matching with flexible conditions:

### Match Conditions

```json
{
  "anyOf": [
    {"scte35.command": "splice_insert"},
    {"scte35.segmentation_type_name": "Provider Advertisement Start"}
  ]
}
```

### Available Fields

- `scte35.command` - SCTE-35 command type
- `scte35.segmentation_type_id` - Raw segmentation type (e.g., "0x30")
- `scte35.segmentation_type_name` - Human-readable type name
- `scte35.upid_type_name` - UPID type name
- `scte35.segmentation_upid` - Decoded UPID data
- `scte35.pts_time` - PTS time value
- `acquisitionSignalID` - Signal ID from ESAM
- `utcPoint` - UTC timestamp

### Actions

- `noop` - Pass through unchanged
- `delete` - Filter out the signal
- `replace` - Modify signal content (provide new SCTE-35 in params)

## SCTE-35 Features

### Supported Commands
- `splice_null` (0x00)
- `splice_schedule` (0x04)
- `splice_insert` (0x05) - with PTS time extraction
- `time_signal` (0x06) - with PTS time extraction
- `bandwidth_reservation` (0x07)
- `private_command` (0xFF)

### Segmentation Descriptors
- Complete segmentation type decoding (0x00-0x51, 0x80+)
- UPID type decoding (Ad-ID, ISAN, TI, ADI, EIDR, MID, URI, UUID, etc.)
- PTS time extraction and conversion to human-readable format

### UPID Support
- **Ad-ID**: 12-character advertising identifiers
- **ISAN**: International Standard Audiovisual Number
- **TI**: Turner Identifier (8-byte integer)
- **ADI**: Advertising Digital Identifier (ASCII)
- **EIDR**: Entertainment Identifier Registry
- **MID**: Managed Private UPID with sub-segments
- **URI**: Web-based identifiers
- **UUID**: Universally Unique Identifiers

## Web Interface

### Event Monitor
- Real-time event streaming with auto-refresh
- Advanced filtering by channel, action, and time range
- Sortable columns with pagination
- Detailed event views with full SCTE-35 analysis
- Performance metrics and statistics

### Channel & Rules Management
- Visual rule configuration with JSON editor
- Rule priority management and testing
- Channel creation and modification
- Real-time rule validation

### SCTE-35 Builder
- Interactive SCTE-35 signal construction
- Testing tool for rule development
- Base64 encoding/decoding utilities

## Development

### Dependencies
- Rust 1.70+
- SQLite 3
- OpenSSL (for TLS support)

### Key Crates
- `axum` 0.7.5 - Web framework
- `sqlx` 0.7.4 - Database toolkit
- `tokio` - Async runtime
- `quick-xml` - XML parsing
- `serde_json` - JSON handling
- `base64` - SCTE-35 decoding

### Building
```bash
# Development build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run
```

## Troubleshooting

### Common Issues

**Server won't start**:
- Check that the port isn't already in use: `netstat -tlnp | grep 8080`
- Verify database permissions: `ls -la pois.db`
- Check environment variables: `printenv | grep POIS`

**TLS certificate errors**:
- Verify certificate paths exist and are readable
- Check certificate expiration: `openssl x509 -in cert.pem -text -noout`
- Ensure proper file permissions (600 for private keys)

**Database issues**:
- Reset database: `rm pois.db` (will lose all data)
- Check SQLite version: `sqlite3 --version`

**Performance issues**:
- Monitor with: `sudo journalctl -u pois.service -f`
- Check system resources: `htop`
- Adjust `RUST_LOG` level to reduce logging overhead

### Logs

View application logs:
```bash
# Systemd service logs
sudo journalctl -u pois.service -f

# Development logs
RUST_LOG=debug cargo run
```

## License

[Your License Here]

## Contributing

[Contributing Guidelines Here]
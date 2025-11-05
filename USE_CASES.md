# POIS Use Cases - Rule Matching & Actions

This guide provides real-world examples of POIS rule matching and action parameters for manipulating SCTE-35 signals.

---

## Table of Contents

1. [Understanding Rules](#understanding-rules)
2. [Match Rule Syntax](#match-rule-syntax)
3. [Action Parameters](#action-parameters)
4. [Common Use Cases](#common-use-cases)
5. [Advanced Examples](#advanced-examples)
6. [SCTE-35 Reference](#scte-35-reference)

---

## Understanding Rules

POIS rules consist of four components:

1. **Name**: Descriptive identifier
2. **Priority**: Lower number = higher priority (0 is highest)
3. **Match JSON**: Conditions that must be met
4. **Action**: What to do (`noop`, `delete`, `replace`)
5. **Params JSON**: How to modify the signal (for `replace` action)

### Rule Evaluation Flow

```
Incoming ESAM Request
    ↓
Match against rules (ordered by priority)
    ↓
First matching rule wins
    ↓
Execute action with params
    ↓
Return ESAM Response
```

---

## Match Rule Syntax

### Basic Operators

#### `anyOf` - Match ANY condition (OR logic)
```json
{
  "anyOf": [
    {"scte35.command": "splice_insert"},
    {"scte35.command": "time_signal"}
  ]
}
```
Matches if the command is EITHER splice_insert OR time_signal.

#### `allOf` - Match ALL conditions (AND logic)
```json
{
  "allOf": [
    {"scte35.command": "time_signal"},
    {"scte35.segmentation_type_id": "0x30"}
  ]
}
```
Matches only if BOTH conditions are true.

#### `noneOf` - Match NONE of the conditions (NOT logic)
```json
{
  "noneOf": [
    {"scte35.segmentation_type_id": "0x10"},
    {"scte35.segmentation_type_id": "0x11"}
  ]
}
```
Matches if the type is NEITHER program start NOR program end.

### Available Match Fields

| Field | Description | Example Values |
|-------|-------------|----------------|
| `scte35.command` | SCTE-35 command type | `splice_insert`, `time_signal`, `splice_null` |
| `scte35.segmentation_type_id` | Segmentation type ID | `0x10`, `0x11`, `0x30`, `0x34` |
| `scte35.upid` | Unique Program Identifier | `ESPN12345`, `*PREMIUM*` (wildcard) |
| `scte35.pts_time` | Presentation timestamp | Numeric value |
| `acquisition_signal_id` | Signal ID from request | `promo-001`, `live-break-*` (wildcard) |
| `channel_name` | Channel receiving signal | `ESPN`, `HBO`, `*_HD` (wildcard) |

### Wildcard Matching

Use `*` for wildcard matching:

```json
{"scte35.upid": "*ESPN*"}
```
Matches any UPID containing "ESPN".

```json
{"acquisition_signal_id": "promo-*"}
```
Matches any signal ID starting with "promo-".

---

## Action Parameters

### 1. Pass Through (No Modification)

**Use Case**: Log the signal but don't modify it.

**Action**: `noop` or `replace`  
**Params**:
```json
{}
```

---

### 2. Delete Signal

**Use Case**: Block unwanted signals from upstream.

**Action**: `delete`  
**Params**:
```json
{}
```

---

### 3. Build New Splice Insert

**Use Case**: Create ad break opportunities.

**Action**: `replace`  
**Params**:
```json
{
  "build": {
    "command": "splice_insert_out",
    "duration_s": 60
  }
}
```

Creates a 60-second ad avail (out-of-network).

**Variants**:
- `splice_insert_in` - Return from ad break (back to network)
- `splice_insert_out` - Start ad break (out of network)
- `splice_insert_in_with_pts` - Return with specific PTS
- `splice_insert_out_with_pts` - Start with specific PTS

---

### 4. Use Extracted PTS

**Use Case**: Preserve timing from incoming signal.

**Action**: `replace`  
**Params**:
```json
{
  "build": {
    "command": "splice_insert_in_with_pts",
    "use_extracted_pts": true
  }
}
```

Extracts PTS from the incoming signal and uses it in the output.

**With Offset**:
```json
{
  "build": {
    "command": "splice_insert_out_with_pts",
    "use_extracted_pts": true,
    "pts_offset_s": 5
  }
}
```

Adds 5 seconds to the extracted PTS.

---

### 5. Replace with Pre-Built SCTE-35

**Use Case**: Inject a specific pre-encoded SCTE-35 message.

**Action**: `replace`  
**Params**:
```json
{
  "scte35_b64": "/DAlAAAAAAAAAP/wFAVAAAsxf+//+pTQbf4ApO6oAGkAAAAAUIWI+A=="
}
```

Replaces the entire signal with your base64-encoded SCTE-35.

---

### 6. Build Time Signal with Segmentation

**Use Case**: Create or modify segmentation descriptors.

**Action**: `replace`  
**Params**:
```json
{
  "build": {
    "command": "time_signal",
    "segmentation_type_id": "0x34",
    "duration_s": 120,
    "upid_type": "TI",
    "upid_value": "NETWORK-ID-12345"
  }
}
```

Creates a 2-minute distributor placement opportunity with network ID.

---

## Common Use Cases

### Use Case 1: Block All Upstream Ad Markers

**Scenario**: Upstream provider sends ad markers you want to ignore.

**Rule Configuration**:
- **Name**: `Block Upstream Ads`
- **Priority**: `0`
- **Action**: `delete`
- **Match**:
  ```json
  {
    "anyOf": [
      {"scte35.segmentation_type_id": "0x30"},
      {"scte35.segmentation_type_id": "0x32"}
    ]
  }
  ```
- **Params**: `{}`

**Result**: All provider and distributor ad markers are deleted.

---

### Use Case 2: Convert Program Boundaries to Ad Breaks

**Scenario**: Network sends program end markers; you want 60-second local ad breaks.

**Rule Configuration**:
- **Name**: `Program End → 60s Ad`
- **Priority**: `10`
- **Action**: `replace`
- **Match**:
  ```json
  {
    "allOf": [
      {"scte35.command": "time_signal"},
      {"scte35.segmentation_type_id": "0x11"}
    ]
  }
  ```
- **Params**:
  ```json
  {
    "build": {
      "command": "splice_insert_out",
      "duration_s": 60
    }
  }
  ```

**Result**: Every program end signal becomes a 60-second ad break.

---

### Use Case 3: Pass Through Premium Content Only

**Scenario**: Only process signals for premium sports events.

**Rule Configuration**:
- **Name**: `ESPN Premium Only`
- **Priority**: `5`
- **Action**: `replace`
- **Match**:
  ```json
  {
    "allOf": [
      {"scte35.command": "time_signal"},
      {"scte35.upid": "*ESPN_PREMIUM*"}
    ]
  }
  ```
- **Params**: `{}`

**Result**: Only ESPN premium signals are passed through.

---

### Use Case 4: Tag All Outgoing Signals with Network ID

**Scenario**: Add your network identifier to all signals.

**Rule Configuration**:
- **Name**: `Add Network UPID`
- **Priority**: `20`
- **Action**: `replace`
- **Match**:
  ```json
  {"anyOf": [{"scte35.command": "time_signal"}]}
  ```
- **Params**:
  ```json
  {
    "build": {
      "command": "time_signal",
      "upid_type": "TI",
      "upid_value": "YOUR-NETWORK-12345",
      "use_extracted_pts": true
    }
  }
  ```

**Result**: All time_signal commands get your network UPID injected.

---

### Use Case 5: Delete Specific Signal IDs

**Scenario**: Block test signals from reaching downstream.

**Rule Configuration**:
- **Name**: `Delete Test Signals`
- **Priority**: `0`
- **Action**: `delete`
- **Match**:
  ```json
  {"acquisition_signal_id": "test-*"}
  ```
- **Params**: `{}`

**Result**: Any signal with ID starting with "test-" is deleted.

---

### Use Case 6: Create Chapter Markers from Generic Signals

**Scenario**: Convert incoming signals into chapter start markers.

**Rule Configuration**:
- **Name**: `Create Chapter Markers`
- **Priority**: `15`
- **Action**: `replace`
- **Match**:
  ```json
  {"acquisition_signal_id": "chapter-*"}
  ```
- **Params**:
  ```json
  {
    "build": {
      "command": "time_signal",
      "segmentation_type_id": "0x22",
      "use_extracted_pts": true
    }
  }
  ```

**Result**: Signals with "chapter-" IDs become SCTE-35 chapter markers.

---

## Advanced Examples

### Complex Multi-Condition Rule

**Scenario**: Only process provider ads for live sports on HD channels during primetime.

**Match**:
```json
{
  "allOf": [
    {"scte35.segmentation_type_id": "0x30"},
    {"scte35.upid": "*LIVE_SPORTS*"},
    {"channel_name": "*_HD"}
  ]
}
```

---

### Conditional Duration Modification

**Scenario**: Extend ad breaks for specific content.

**Rule 1: Premium Content - 90s breaks**
- **Priority**: `5`
- **Match**: `{"scte35.upid": "*PREMIUM*"}`
- **Params**: `{"build": {"command": "splice_insert_out", "duration_s": 90}}`

**Rule 2: Standard Content - 60s breaks**
- **Priority**: `10`
- **Match**: `{"scte35.command": "time_signal"}`
- **Params**: `{"build": {"command": "splice_insert_out", "duration_s": 60}}`

**Result**: Premium gets 90s, everything else gets 60s (first match wins).

---

### Cascading Rules for Failover

**Scenario**: Try to match premium content first, fallback to generic processing.

**Rule 1: ESPN Premium**
- **Priority**: `1`
- **Match**: `{"scte35.upid": "*ESPN_PREMIUM*"}`
- **Action**: `replace`
- **Params**: `{"build": {"command": "splice_insert_out", "duration_s": 120}}`

**Rule 2: Any ESPN Content**
- **Priority**: `5`
- **Match**: `{"scte35.upid": "*ESPN*"}`
- **Action**: `replace`
- **Params**: `{"build": {"command": "splice_insert_out", "duration_s": 90}}`

**Rule 3: All Other Content**
- **Priority**: `10`
- **Match**: `{"anyOf": [{"scte35.command": "time_signal"}]}`
- **Action**: `replace`
- **Params**: `{"build": {"command": "splice_insert_out", "duration_s": 60}}`

**Result**: Premium gets 2min, standard ESPN gets 90s, everything else gets 60s.

---

## SCTE-35 Reference

### Common Segmentation Type IDs

| Type ID | Description | Use Case |
|---------|-------------|----------|
| `0x10` | Program Start | Beginning of program content |
| `0x11` | Program End | End of program content |
| `0x12` | Program Early Termination | Program cut short |
| `0x13` | Program Breakaway | Emergency override |
| `0x22` | Chapter Start | Content chapter boundary |
| `0x23` | Chapter End | End of chapter |
| `0x30` | Provider Ad Start | Network ad opportunity |
| `0x31` | Provider Ad End | End of network ad |
| `0x32` | Distributor Ad Start | Local ad opportunity |
| `0x33` | Distributor Ad End | End of local ad |
| `0x34` | Provider Placement Opportunity Start | Network ad avail |
| `0x35` | Provider Placement Opportunity End | End of network avail |
| `0x36` | Distributor Placement Opportunity Start | Local ad avail |
| `0x37` | Distributor Placement Opportunity End | End of local avail |

### UPID Types

| Type | Description | Format Example |
|------|-------------|----------------|
| `TI` | Turner Identifier | `NETWORK-ID-12345` |
| `ISAN` | International Standard Audiovisual Number | `0000-0000-1234-5678-A` |
| `TID` | Tribune ID | `SH123456` |
| `ADS` | Ad ID | `AD-12345` |
| `EIDR` | Entertainment Identifier Registry | `10.5240/1234-ABCD` |

### Command Types

| Command | Description | Usage |
|---------|-------------|-------|
| `splice_insert` | Legacy ad marker | Immediate or timed splice |
| `time_signal` | Timing marker | Used with segmentation descriptors |
| `splice_null` | Heartbeat | Keep-alive signal |

---

## Testing Your Rules

### 1. Create Test Channel

```bash
curl -X POST "http://localhost:8090/api/channels" \
  -H "Authorization: Bearer dev-token" \
  -H "Content-Type: application/json" \
  -d '{"name": "test-channel", "enabled": true}'
```

### 2. Add Test Rule

```bash
curl -X POST "http://localhost:8090/api/channels/test-channel/rules" \
  -H "Authorization: Bearer dev-token" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Test Rule",
    "priority": 1,
    "action": "replace",
    "match_json": "{\"anyOf\":[{\"scte35.command\":\"time_signal\"}]}",
    "params_json": "{\"build\":{\"command\":\"splice_insert_out\",\"duration_s\":60}}"
  }'
```

### 3. Send Test ESAM Request

```bash
curl -X POST "http://localhost:8090/esam?channel=test-channel" \
  -H "Content-Type: application/xml" \
  -d '<?xml version="1.0"?>
<SignalProcessingEvent xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1">
  <AcquiredSignal acquisitionSignalID="test-001">
    <UTCPoint utcPoint="2025-11-05T20:00:00Z"/>
  </AcquiredSignal>
</SignalProcessingEvent>'
```

### 4. Check Event Monitor

Open `http://localhost:8090/static/events.html` and verify:
- Event appears in table
- Correct rule matched
- Action applied correctly
- Raw XML shows modifications

---

## Best Practices

### 1. Priority Management

- **0-10**: Critical overrides (delete unwanted signals)
- **10-50**: Content-specific processing (premium vs standard)
- **50-100**: Generic fallback rules (catch-all)

### 2. Rule Naming

Use descriptive names:
- ✅ `Delete Upstream Provider Ads`
- ✅ `ESPN Premium → 90s Ad Breaks`
- ❌ `Rule 1`
- ❌ `Test`

### 3. Match Specificity

More specific matches should have higher priority (lower number):

```
Priority 1:  Match ESPN Premium content
Priority 10: Match any ESPN content
Priority 50: Match all content
```

### 4. Testing Strategy

1. Create test channel
2. Add rules one at a time
3. Test with known ESAM requests
4. Verify in Event Monitor
5. Check raw XML output
6. Deploy to production channel

### 5. Documentation

Document your rules in the rule name or keep a separate mapping:

```
Channel: live-sports
  - Rule "Delete Test Signals" (Priority 0): Blocks test-* signals
  - Rule "Premium 120s" (Priority 5): ESPN_PREMIUM → 120s ads
  - Rule "Standard 60s" (Priority 10): All others → 60s ads
```

---

## Troubleshooting

### Rule Not Matching

**Check**:
1. Is the channel enabled?
2. Is the rule enabled?
3. Is another higher-priority rule matching first?
4. Is the match JSON valid?
5. Are field names correct (check Event Monitor raw XML)?

### Action Not Applied

**Check**:
1. Is `POIS_STORE_RAW_PAYLOADS=true` set?
2. Check Event Monitor for errors
3. Verify params JSON is valid
4. Check server logs: `sudo journalctl -u pois -n 100`

### Unexpected Results

**Debug Steps**:
1. Check Event Monitor → Click "View" on event
2. Compare ESAM Request XML vs Response XML
3. Verify which rule matched
4. Check processing time (slow = complex rules)

---

## Additional Resources

- **SCTE-35 Specification**: https://www.scte.org/standards/library/catalog/scte-35/
- **SCTE-130 (ESAM) Specification**: https://www.scte.org/standards/library/catalog/scte-130/
- **POIS API Documentation**: `/static/docs.html`
- **GitHub Repository**: https://github.com/bokelleher/rust-pois

---

**Questions or suggestions?** Open an issue on GitHub!

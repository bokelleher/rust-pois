# POIS Server — End‑to‑End Test Plan

This document contains copy‑pasteable steps and payloads to exercise the POIS server through its Web UI and REST API.

> Assumptions
> - Server running on `http://localhost:8080`
> - Admin token: `dev-token`
> - Database URL: `sqlite://./data/pois.db`

---

## 0) One‑time bootstrap

```bash
# create a channel "default" (id = 1)
curl -s -X POST http://localhost:8080/api/channels \
  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \
  -d '{"name":"default"}'
```

---

## 1) Convert `splice_insert` (type 0x05) → `time_signal` (type 0x06)

**Rule**

```bash
curl -s -X POST http://localhost:8080/api/channels/1/rules \  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \  -d '{
        "name":"Convert splice_insert -> time_signal",
        "priority": -10,
        "enabled": true,
        "match_json": { "anyOf": [ { "scte35.command": "splice_insert" } ] },
        "action":"replace",
        "params_json": { "scte35_b64": "AAAA" }   // TODO: replace with a real type-6 payload (base64)
      }'
```

**Dry‑run**

```bash
curl -s http://localhost:8080/api/dryrun \  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \  -d '{"channel":"default","esam_xml":"<SignalProcessingEvent xmlns=\"urn:cablelabs:iptvservices:esam:xsd:signal:1\" xmlns:sig=\"urn:cablelabs:md:xsd:signaling:3.0\"><AcquiredSignal acquisitionSignalID=\"abc-123\"><sig:UTCPoint utcPoint=\"2025-08-24T00:00:00Z\"/></AcquiredSignal><sig:BinaryData signalType=\"SCTE35\">/9w=</sig:BinaryData></SignalProcessingEvent>"}'
```

Expected: `matched_rule_id` not null, `action = "replace"`.

---

## 2) Program End (`segmentation_type_id=0x34`) → CUE‑OUT with 60s auto‑return

**Rule**

```bash
curl -s -X POST http://localhost:8080/api/channels/1/rules \  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \  -d '{
        "name":"Program-end -> CUE-OUT 60s",
        "priority": -5,
        "enabled": true,
        "match_json": {
          "allOf": [
            { "scte35.command": "time_signal" },
            { "scte35.segmentation_type_id": "0x34" }
          ]
        },
        "action":"replace",
        "params_json": { "scte35_b64": "AAAA" }  // TODO: real base64 of splice_insert OUT w/ duration=60
      }'
```

**Dry‑run**: same command as above (payload must contain a segmentation descriptor with `type_id=0x34`).

Expected: `matched_rule_id` not null, `action = "replace"`.

---

## 3) Normalize/Filter by Provider UPID

Match on provider UPID and replace with standardized payload (example pattern).

**Rule**

```bash
curl -s -X POST http://localhost:8080/api/channels/1/rules \  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \  -d '{
        "name":"Normalize provider UPID",
        "priority": 0,
        "enabled": true,
        "match_json": { "anyOf": [ { "scte35.segmentation_upid": "MyProvider*" } ] },
        "action":"replace",
        "params_json": { "scte35_b64": "AAAA" }  // standardized payload
      }'
```

---

## 4) Force CUE‑IN at Program Start (`segmentation_type_id=0x10`)

**Rule**

```bash
curl -s -X POST http://localhost:8080/api/channels/1/rules \  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \  -d '{
        "name":"Program-start -> CUE-IN",
        "priority": 5,
        "enabled": true,
        "match_json": {
          "allOf": [
            { "scte35.command": "time_signal" },
            { "scte35.segmentation_type_id": "0x10" }
          ]
        },
        "action":"replace",
        "params_json": { "scte35_b64": "AAAA" }  // CUE-IN payload
      }'
```

---

## 5) Blackout / Maintenance Window (delete during time window)

**Rule**

```bash
curl -s -X POST http://localhost:8080/api/channels/1/rules \  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \  -d '{
        "name": "Blackout window",
        "priority": 10,
        "enabled": true,
        "match_json": { "allOf": [ { "utcBetween": { "start": "2025-08-24T00:00:00Z", "end": "2025-08-24T02:00:00Z" } } ] },
        "action": "delete",
        "params_json": {}
      }'
```

---

## 6) Per‑feed Scope (only handle certain acquisitionSignalID prefixes)

**Rule**

```bash
curl -s -X POST http://localhost:8080/api/channels/1/rules \  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \  -d '{
        "name": "Only my feeds",
        "priority": 20,
        "enabled": true,
        "match_json": { "anyOf": [ { "acquisitionSignalID": "east-*" }, { "acquisitionSignalID": "west-*" } ] },
        "action": "noop",
        "params_json": {}
      }'
```

---

## 7) Default Catch‑all (pass‑through)

**Rule**

```bash
curl -s -X POST http://localhost:8080/api/channels/1/rules \  -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \  -d '{
        "name":"Default pass-through",
        "priority": 9990,
        "enabled": true,
        "match_json": {},
        "action":"noop",
        "params_json": {}
      }'
```

---

## ESAM Request Samples

Create a file `event-basic.xml`:

```xml
<SignalProcessingEvent xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1" xmlns:sig="urn:cablelabs:md:xsd:signaling:3.0">
  <AcquiredSignal acquisitionSignalID="feed-001">
    <sig:UTCPoint utcPoint="2025-08-24T00:00:00Z"/>
  </AcquiredSignal>
</SignalProcessingEvent>
```

Send it to `/esam`:

```bash
curl -s "http://localhost:8080/esam?channel=default" \  -H "Content-Type: application/xml" \  -d @event-basic.xml
```

With placeholder SCTE‑35 (adjust to your actual base64) — `event-scte35.xml`:

```xml
<SignalProcessingEvent xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1" xmlns:sig="urn:cablelabs:md:xsd:signaling:3.0">
  <AcquiredSignal acquisitionSignalID="abc-123">
    <sig:UTCPoint utcPoint="2025-08-24T00:00:00Z"/>
  </AcquiredSignal>
  <sig:BinaryData signalType="SCTE35">/9w=</sig:BinaryData>
</SignalProcessingEvent>
```

```bash
curl -s "http://localhost:8080/esam?channel=default" \  -H "Content-Type: application/xml" \  -d @event-scte35.xml
```

---

## Tips & Notes

- **Priorities:** lower numbers run first. Keep “catch‑all” at high priority (e.g., 9990).
- **Matchers available:** `scte35.command`, `scte35.segmentation_type_id`, `scte35.segmentation_upid`, `acquisitionSignalID` (glob `*`), `utcBetween` window.
- **Replace action:** provide `params_json.scte35_b64` (base64). The server embeds it into the ESAM response.
- **Dry‑run** is the easiest way to confirm matching without emitting XML to your downstream.
- If you need help building valid SCTE‑35 base64 payloads, ask and we can add a small `scte35.rs` builder that generates them for you (e.g., `splice_insert OUT` with duration N seconds, `time_signal` immediate, etc.).

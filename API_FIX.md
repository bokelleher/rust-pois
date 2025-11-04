# API Endpoint Fix - Events Monitor 🔧

## Issue Found

The Event Monitor page was calling incorrect API endpoints that didn't match the backend.

## What Was Fixed

### ❌ Before (Incorrect)
```javascript
// Wrong endpoint
fetch('/api/events/statistics')  // 400 error!

// Wrong parameter names
fetch('/api/events?page=1&page_size=50&sort_by=timestamp')
```

### ✅ After (Correct)
```javascript
// Correct endpoint
fetch('/api/events/stats')  // Works!

// Correct parameter names
fetch('/api/events?limit=50&offset=0')
```

---

## Changes Made to `events.html`

### 1. Statistics Endpoint
**Line ~283**: Changed `/api/events/statistics` → `/api/events/stats`

### 2. Statistics Field Names
Updated to match backend response:
- `stats.total` → `stats.total_events`
- `stats.last_24h` → `stats.last_24h_events`
- `stats.avg_processing_time` → `stats.avg_processing_time_ms`
- `stats.action_breakdown` → `stats.action_counts`

### 3. Events Query Parameters
Changed from pagination-style to limit/offset:
- `page` → removed (calculate offset instead)
- `page_size` → `limit`
- Added: `offset` (calculated as `(page - 1) * limit`)
- Removed: `sort_by`, `sort_order` (not used by backend)

---

## Backend API Reference

Based on the Rust code, here are the correct endpoints:

### GET `/api/events`
**Query Parameters:**
- `limit` (optional) - Number of events to return (default: 100, max: 1000)
- `offset` (optional) - Number of events to skip (default: 0)
- `channel` (optional) - Filter by channel name
- `action` (optional) - Filter by action (noop, delete, replace)
- `since` (optional) - Filter by time period

**Response:** Array of event objects

### GET `/api/events/stats`
**Response:**
```json
{
  "total_events": 1234,
  "last_24h_events": 567,
  "avg_processing_time_ms": 45.2,
  "action_counts": {
    "noop": 100,
    "delete": 50,
    "replace": 25
  }
}
```

### GET `/api/events/:id`
**Response:** Single event object with full details

---

## Testing

After deploying the fixed `events.html`:

1. **Check browser console** - Errors should be gone
2. **Statistics should load** - Top cards should show numbers
3. **Events table should populate** - If events exist in database
4. **Filters should work** - Channel, action, since, search

---

## If Still No Events

If the page loads correctly but shows "No events found":

### Cause: Database is empty
You need to send ESAM requests to generate events:

```bash
# Example: Send a test ESAM request
curl -X POST "http://localhost:8090/esam?channel=test-channel" \
  -H "Content-Type: application/xml" \
  -d '<SignalProcessingEvent xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1">
        <AcquiredSignal acquisitionSignalID="test-123">
          <sig:UTCPoint utcPoint="2024-11-04T10:00:00Z"/>
        </AcquiredSignal>
      </SignalProcessingEvent>'
```

### Check if events are being logged:

```bash
# Check the database
sqlite3 pois.db "SELECT COUNT(*) FROM esam_events;"

# Or check via API
curl -H "Authorization: Bearer dev-token" \
  "http://localhost:8090/api/events/stats"
```

---

## Deployment

Simply replace the events.html file:

```bash
cp events.html static/
# No service restart needed - static files reload automatically
```

Then hard refresh your browser: **Ctrl+Shift+R**

---

## Summary

✅ Fixed `/api/events/statistics` → `/api/events/stats`  
✅ Fixed parameter names to match backend  
✅ Fixed response field names  
✅ Page should now load without errors  
✅ Events will display once database has data  

---

**Fixed**: November 4, 2025  
**File Updated**: events.html  
**Issue**: API endpoint mismatch

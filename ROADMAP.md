# Roadmap

Deferred / proposed enhancements, captured as they come up. Not commitments —
a backlog to prune and prioritize.

## SCTE-35 Tools

### SMPTE timecode / frame-based rendering of SCTE-35 times
Optionally render decoded `pts_time` and `break_duration` as `HH:MM:SS:FF`
timecode and/or frame counts, with a frame-rate selector
(23.976 / 24 / 25 / 29.97 DF / 29.97 NDF / 30 / 50 / 59.94 DF / 59.94 NDF / 60),
including drop-frame vs non-drop-frame handling — i.e. the "Frame Rate" option
seen in parsers like tools.middleman.tv.

> **Caveat (must be honored in the UI).** A SCTE-35 PTS is a 90 kHz,
> program-relative presentation timestamp — it is **not** wall-clock and **not**
> a SMPTE timecode. Without a PCR/timeline origin or a real timecode track, any
> rendered `HH:MM:SS:FF` is *PTS-relative* (elapsed-from-zero), not a broadcast
> timecode, and must be labeled as such to avoid implying false precision. POIS
> itself keys rule timing off the ESAM `UTCPoint` (real UTC), not the PTS, so
> this is a decoder *display* aid only.

### SCTE-35 → SCTE-104 conversion
Convert a decoded SCTE-35 message to SCTE-104. This is frame/timecode-oriented,
so it depends on the frame-rate selection above.

## Event Monitor / logging

### Human-readable UPID in the descriptor panel
The `esam_events.scte35_upid` column stores the raw hex form
(`0x{type}:{hex}`). Store/display the *decoded* representation
(`decode_upid_data`) so the monitor shows the same value the rule engine matches
on (e.g. `REGION-AFE1`) instead of hex.

### Log SESAME rejections
401/403 SESAME rejections (insufficient-tier, scope-denied) are currently not
written to `esam_events` — only successes and signature/replay failures are.
Log them so blocked/failed auth attempts are visible in the Event Monitor.

## SESAME (SCTE 130-9)

### Distributed replay cache
The in-memory `ReplayCache` is per-process; back it with a shared store (e.g.
Redis) so replay protection holds across horizontally-scaled POIS nodes. The
trait seam already exists.

### Optional `ring` crypto backend
A faster AEAD/HMAC backend behind a feature flag, for high-throughput
deployments. The default stays pure-Rust RustCrypto (deploy-anywhere).

### Paper items pending DVS ratification
See `docs/SESAME_paper_errata.md` — notably promoting response signing to a
normative requirement (errata E8) in the SCTE 130-9 draft.

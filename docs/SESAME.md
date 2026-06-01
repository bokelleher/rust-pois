# SESAME Wire Format (rust-pois reference implementation)

This document specifies, byte-for-byte, the SESAME (Secure ESAM Authentication
and Message Encryption) protocol as implemented by `rust-pois`. It mirrors
**ANSI/SCTE 130-9 (SESAME) draft v0.4** and is the authority for what this
codebase puts on the wire.

Where the draft is silent or self-contradictory, this implementation adopts a
working construction marked **[BO]** below; each such point is also logged in
[`SESAME_reconciliation.md`](./SESAME_reconciliation.md) for ratification back
into the paper. The implementation and the paper must end up identical on the
wire — until the [BO] items are resolved in the paper text, **this file is the
operative spec** for interop testing.

SESAME adds nothing to the ESAM XML. Tiers 1 and 2 leave the body untouched;
Tier 3 replaces the body with ciphertext. No SCTE 130-2/-5/-7/-8 schema changes.

## Tiers

| Tier | Capability | Mechanism |
|---|---|---|
| 0 | Unauthenticated baseline | no SESAME headers (backward compatible) |
| 1 | Authentication + integrity | HMAC-SHA256 over a canonical string |
| 2 | Channel-scoped authorization | signed `X-SESAME-Scope`, policy lookup |
| 3 | Payload encryption | AES-256-GCM (96-bit IV, 128-bit tag) |

Tiers are additive and independently enableable. A channel's **minimum required
tier** is server-side policy (`channels.sesame_min_tier`, default 0); there is
**no on-wire tier-advertisement header** — see [BO-3].

## Headers (Appendix A.6)

| Header | Tier | Required | Value |
|---|---|---|---|
| `X-SESAME-Version` | 1+ | yes | `1.0` |
| `X-SESAME-KeyId` | 1+ | yes | signing credential id |
| `X-SESAME-Timestamp` | 1+ | yes | ISO-8601 UTC, e.g. `2026-02-24T18:00:00Z` |
| `X-SESAME-Nonce` | 1+ | yes | 128-bit random, **lowercase hex** (32 chars) |
| `X-SESAME-Signature` | 1+ | yes | HMAC-SHA256, **lowercase hex** (64 chars) |
| `X-SESAME-Scope` | 2+ | yes (Tier 2+) | `channel=<id>` |
| `X-SESAME-Encrypted` | 3 | yes (Tier 3) | `true` |
| `X-SESAME-EncKeyId` | 3 | yes (Tier 3) | encryption credential id (separate namespace) |
| `X-SESAME-IV` | 3 | yes (Tier 3) | 96-bit GCM IV, **lowercase hex** (24 chars) |

Tier 3 sets `Content-Type: application/octet-stream`; the original
`application/xml` type is **not** preserved on the wire (§8.4).

## Tier 1 — canonical signing string (§8.2.2)

The HMAC-SHA256 signature covers this exact string (newline = `\n` = `%x0A`):

```
<HTTP-METHOD>\n
<request-target>\n          ; path + query, exactly as sent, e.g. /esam?channel=SportsFeed-East
<X-SESAME-Timestamp>\n
<X-SESAME-Nonce>\n
<lowercase-hex SHA-256 of the body AS TRANSMITTED>
```

* The body hash is over the **transmitted** body. With Tier 3 active the
  transmitted body is the ciphertext, so the scheme is **encrypt-then-MAC**.
  ([BO-6]: the draft does not state the ordering; this is the safe choice.)
* There is **no** version-prefix line and **no** key-id line — the draft's ABNF
  and worked example (§8.2.3) define exactly five fields.

### [BO-1] Tier 2 scope binding

The draft's §8.2.2 ABNF has no scope field, but §8.3 says the scope header
"SHALL be included in the signature computation." Working construction: when
`X-SESAME-Scope` is present, **append it as a sixth line** equal to the exact
header value:

```
<method>\n<target>\n<timestamp>\n<nonce>\n<body-hash>\n<scope-value>
```

### [BO-2] Response canonical string

The draft signs responses (Appendix A.2/A.4) but never defines a canonical form
for them (a response has no method/path). Working construction:

```
RESPONSE\n
<correlation>\n             ; acquisitionSignalID being answered
<X-SESAME-Timestamp>\n
<X-SESAME-Nonce>\n
<body-hash>
[ \n<scope-value> ]         ; present iff Tier 2+
```

`correlation` binds the signed response to the specific request signal,
defeating response-substitution. **This is the primary protection**: a forged or
tampered POIS decision (spoofed blackout/avail/redirect) fails verification.

## Tier 3 — AES-256-GCM (§8.4)

* Body ← `AES-256-GCM(key, IV, AAD, plaintext)` = `ciphertext || 128-bit tag`.
* **IV**: fresh 96-bit value from the OS CSPRNG **per message**. Never reused
  with a key. ([BO-4]: Appendix A.4 reuses one IV across request and response
  under the same `EncKeyId` — a catastrophic GCM nonce reuse. **Paper fix drafted
  in [`SESAME_paper_errata.md`](./SESAME_paper_errata.md) E1**; the code is
  already correct.)
* **AAD**: the canonical SESAME header set, newline-joined:
  `version\nkey-id\ntimestamp\nnonce[\nscope]`. Binds the ciphertext to its
  headers. ([BO-5]: the draft does not specify AAD — **paper fix drafted in
  errata E2**.)
* Encryption keys live in a **separate namespace** from signing keys
  (`X-SESAME-EncKeyId` vs `X-SESAME-KeyId`) and rotate independently (§8.4).

## Order of operations (all tiers active)

**Send:** serialize XML → Tier 3 encrypt (AAD = headers) → Tier 1 SHA-256 over
ciphertext, build canonical, HMAC → attach headers → send.

**Receive:** Tier 1 verify (version, freshness, signature) → replay check →
Tier 2 authorize → Tier 3 decrypt → parse XML. Fail closed at each step with the
distinct error below.

## Replay protection (§8.2.4)

* Reject timestamps outside ±`replay_window_secs` (default **300 s**, §8.5).
* Reject any `(KeyId, Nonce)` already seen within the window.
* The reference cache is in-memory and **per-process**. Horizontally-scaled POIS
  deployments MUST back it with a shared store (e.g. Redis) via the `ReplayCache`
  trait — a single-process cache does not prevent cross-node replay.
* Replay is checked **after** signature validation, so unauthenticated traffic
  cannot poison the cache.

## Error codes (Appendix A.7)

| Error | HTTP | Cause |
|---|---|---|
| `sesame_missing_headers` | 401 | required headers absent / tier below channel minimum |
| `sesame_invalid_version` | 400 | unsupported `X-SESAME-Version` |
| `sesame_unknown_key` | 401 | key-id not in credential store |
| `sesame_expired_timestamp` | 401 | timestamp outside replay window |
| `sesame_replay_detected` | 401 | nonce already used |
| `sesame_signature_mismatch` | 401 | HMAC mismatch |
| `sesame_scope_denied` | 403 | key not authorized for scope / scope≠target |
| `sesame_decrypt_failed` | 400 | GCM tag/decrypt failure |
| `sesame_key_revoked` | 401 | key-id explicitly revoked |

## Key configuration (operator responsibility, §8.2.5)

Key distribution is out of band. The reference `KeyProvider` is populated from
`POIS_SESAME_KEYS` (JSON):

```json
{
  "signing": [
    {"key_id": "sas-east-01",  "secret_hex": "…", "channels": ["SportsFeed-East"]},
    {"key_id": "pois-primary", "secret_hex": "…", "channels": ["*"]}
  ],
  "encryption": [
    {"enc_key_id": "enc-sportsfeed-2026q1", "key_hex": "<64 hex chars = 32 bytes>"}
  ]
}
```

Relevant environment variables:

| Variable | Meaning | Default |
|---|---|---|
| `POIS_SESAME_KEYS` | signing/encryption key material (JSON above) | unset → SESAME inactive |
| `POIS_SESAME_MIN_TIER` | default minimum tier when a channel has none | `0` |
| `POIS_SESAME_REPLAY_WINDOW` | replay/freshness window, seconds | `300` |
| `POIS_SESAME_RESPONSE_KEYID` | signing key-id used to sign POIS responses | unset → responses unsigned |
| `POIS_SESAME_RESPONSE_ENCID` | encryption key-id for Tier 3 responses | unset |

Per-channel minimum tier is stored in `channels.sesame_min_tier` (migration
0006). Key rotation: a key-id MAY have multiple valid signing keys during an
overlap window (§8.2.5); verification accepts any, signing uses the primary.

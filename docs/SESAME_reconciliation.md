# SESAME ⇄ rust-pois Reconciliation Note (for Bo)

Status of the implementation against **ANSI/SCTE 130-9 (SESAME) draft v0.4**, and
every point where the code had to **deviate from, disambiguate, or correct** the
paper. Per the handoff guardrail §0.1, the paper is authoritative — so each item
below is a request to either ratify the implementation's choice into the paper
text, or tell me to change the code.

The wire format the code currently emits is fully specified in
[`SESAME.md`](./SESAME.md); the measured performance is in
[`benchmarks.md`](./benchmarks.md).

---

## A. The implementation now exists (the headline gap is closed)

Before this work, `rust-pois` contained **no SESAME code**, yet §9 of the paper
describes it as a "working open-source reference implementation" with a measured
performance table. That table was a projection. It is now backed by code:

* Framework-agnostic core (`src/sesame/`): all three tiers, bidirectional.
* Axum integration (`src/sesame_axum.rs`) wired into the ESAM request **and**
  response paths.
* 45 passing tests including **RFC 4231** (HMAC-SHA256) and **NIST SP 800-38D**
  (AES-256-GCM) known-answer vectors, the full negative matrix, and live
  end-to-end HTTP checks.
* Criterion benchmarks producing the sub-millisecond table.

---

## B. Header / format corrections (handoff proposal was wrong; paper wins)

The original handoff §2 proposed header names and encodings that disagreed with
the paper. The code follows the **paper**, not the handoff:

| Field | Handoff (rejected) | Paper / code (used) |
|---|---|---|
| Version value | `1` | `1.0` |
| Key-id header | `X-SESAME-Key-Id` | `X-SESAME-KeyId` |
| Timestamp | unix-ms | ISO-8601 UTC |
| Nonce / Signature / IV | base64 | lowercase **hex** |
| Channel (Tier 2) | `X-SESAME-Channel` | `X-SESAME-Scope: channel=<id>` |
| Tier-3 indicator | `X-SESAME-Enc` | `X-SESAME-Encrypted: true` |
| Tier-3 key | reuse `Key-Id` | separate `X-SESAME-EncKeyId` |
| Content-Type (Tier 3) | wrapper header? | `application/octet-stream` |
| Canonical string | 8 lines (+version/key-id) | **5 lines** (method,path,ts,nonce,body-hash) |
| Freshness window | ±30 s | **300 s** |

**No action needed** — just confirming the handoff's proposal was superseded by
the paper.

---

## C. Defects / gaps IN the paper that needed a decision (please ratify)

These could not be resolved by "the paper wins" because the paper is silent,
contradictory, or wrong. The code adopts the construction noted; **please ratify
each into the paper text, or direct a change.**

**[BO-1] Tier 2 scope is missing from the canonical-string ABNF. — ✅ FIX DRAFTED.**
§8.2.2 fixes the canonical string at five fields with no scope, but §8.3 says the
scope "SHALL be included in the signature computation." Contradiction, and no
Tier-2 worked example. → Code appends the scope as a **sixth canonical line**
(exact `X-SESAME-Scope` value). **Drop-in paper fix: errata E7** (ABNF amendment +
Tier-2 worked example). Apply to source to close.

**[BO-2] Response canonical string is undefined. — ✅ FIX DRAFTED.**
Appendix A.2/A.4 show signed responses, but §8.2.2's ABNF requires method+path,
which a response has neither. → Code uses
`RESPONSE\n<acquisitionSignalID>\n<ts>\n<nonce>\n<body-hash>[\n<scope>]`. **Drop-in
paper fix: errata E8** (new normative §8.2.6 defining the response canonical
string).

**[BO-3] Response signing is only *informative*. — ✅ FIX DRAFTED (needs WG sign-off).**
The §8.5 normative summary lists request-validation duties only; response
signing appears solely in the informative Appendix A. Yet authenticating the
POIS's outbound decision is the **highest-value protection** (a forged
blackout/avail/redirect is the main threat). → Code signs responses by default.
**Drop-in paper fix: errata E8** promotes it to a normative SHALL (new §8.2.6 +
§8.5 item 14). This is a substantive change — recommend WG adoption.

**[BO-4] 🔴 Cryptographic bug: GCM IV reuse in Appendix A.4. — ✅ FIX DRAFTED.**
The Tier-3 example uses the **same `X-SESAME-IV`** (`7c8d9e0f1a2b3c4d5e6f7a8b`) on
both request and response under the **same `X-SESAME-EncKeyId`**. Reusing a GCM
nonce under one key is catastrophic (NIST SP 800-38D §8.3): it breaks
confidentiality and allows tag forgery. → Code draws a **fresh CSPRNG IV per
message** and never reuses it. **Drop-in paper fix in
[`SESAME_paper_errata.md`](./SESAME_paper_errata.md) E1** (distinct response IV +
normative IV-uniqueness SHALL). Apply to source to close.

**[BO-5] Tier 3 AAD is unspecified. — ✅ FIX DRAFTED.**
§8.4 never mentions GCM additional authenticated data. → Code binds
`version\nkey-id\ntimestamp\nnonce[\nscope]` as AAD so headers cannot be swapped
under the ciphertext. **Drop-in paper fix in errata E2** (adds the AAD definition
to §8.4 and §8.5 item 11).

**[BO-6] Encrypt-then-MAC ordering not stated. — ✅ FIX DRAFTED.**
§8.2.1 hashes "the request body"; §8.4 says ciphertext replaces the body. The
two together imply the Tier-1 hash is over ciphertext (encrypt-then-MAC), but it
is never said. → Code does encrypt-then-MAC. **Drop-in paper fix in errata E3**
(clarifies §8.2.1 and adds §8.5 item 13).

**[BO-7] Error semantics leak key existence.**
Appendix A.7 distinguishes `sesame_unknown_key` from `sesame_signature_mismatch`
(both 401), and the JSON `detail` echoes "Key X not authorized for channel Y".
This is a mild key/channel enumeration oracle, conflicting with the handoff's
no-leak goal. → Code follows the paper (distinct codes) but the error status is
centralized so an operator can collapse them to one opaque 401. Flagging the
trade-off; your call whether to keep the distinction normative.

**[BO-8] Request-target form must match the reference impl.**
The paper signs `/esam?channel=SportsFeed-East` (query param). `rust-pois` also
exposes `/esam/channel/{channel}` (path segment) and derives the channel from the
XML body. The canonical string signs the target **verbatim**, so client and
server must agree. → Code signs the exact `OriginalUri` path+query as received.
Please confirm the paper's examples should standardize on one target form (I'd
suggest documenting both routes and signing whatever is actually sent).

---

## D. Performance: re-measure on the cited hardware before publishing

**[BO-9] — ✅ RESOLVED (measured on real c6i.xlarge).** §9.2's original table
("AWS c6i.xlarge, 4 vCPU, 8 GB RAM") predated any implementation; its numbers
(e.g. Tier 3 0.32/0.51 ms) were not measured. The benchmark has now been run on
an **actual c6i.xlarge** (Intel Xeon Platinum 8375C, Ice Lake, AES-NI + SHA
extensions confirmed present), Ubuntu 24.04, rustc 1.96.0 — see
[`benchmarks.md`](./benchmarks.md). Measured results:

* Combined **inbound** (verify+authz+decrypt) p99: 3.4 µs (1 KB) → **0.027 ms** (16 KB).
* Combined **outbound** (encrypt+sign) p99: 4.3 µs (1 KB) → **0.028 ms** (16 KB).
* Tier 3 decrypt at 16 KB: **0.013 ms** — ~25–40× faster than the 0.32/0.51 ms
  the draft prints.

**Action: replace §9.2's table with the two tables in `benchmarks.md`** (and the
hardware row). The real numbers are *better* than the draft claims — worst-case
combined p99 is ~35× inside the 1 ms budget — so this strengthens the result. A
QEMU-VM-without-AES-NI run is also recorded as a conservative portability floor
(≤ 0.22 ms p99).

---

## E. §10 open decisions — resolved

| # | Decision | Resolution |
|---|---|---|
| 1 | Header names | From paper (table B). ✅ |
| 2 | Canonical construction | Tier 1 from paper; Tier 2 [BO-1] = errata E7, response [BO-2] = errata E8. ✅ drafted |
| 3 | Tier negotiation | None on the wire; server-side per-channel `sesame_min_tier`. ✅ |
| 4 | Content-Type (Tier 3) | `application/octet-stream`, original not preserved. ✅ |
| 5 | Freshness window | 300 s default, configurable. ✅ |
| 6 | Key-id / channel model | per-key channel scope; separate signing vs encryption key namespaces; rotation overlap. ✅ |
| 7 | Response signing mandated? | [BO-3] = errata E8 promotes to normative §8.2.6. ✅ drafted, needs WG sign-off |

---

## F. Out of scope (consistent with paper §1.5)

Key distribution protocol, mutual auth, PKI integration, and a formal security
proof remain future work. The reference provider is env/JSON-backed; the
`KeyProvider` and `ReplayCache` traits are the seams for a KMS / Redis later.

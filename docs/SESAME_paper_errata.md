# SESAME Draft v0.4 — Errata & Revisions (→ v0.5)

> **STATUS (applied 2026-06-01):** All items below are now applied to both source
> documents:
> * **SCTE whitepaper** — `docs/SESAME_Whitepaper_SCTE_Draft_v0_5_final.docx`
>   (E1–E5 were already in the v0.5 clean draft; E6 perf numbers, E7 scope-canonical,
>   and E8 §8.2.6 Response Signing added here — clean edits, validated).
> * **SET IJBE journal paper** — `docs/SESAME_SET_IJBE.tex` (+ recompiled
>   `SESAME_SET_IJBE.pdf`): perf table → measured µs values, scope added to the
>   canonical equation, response signing + fresh-IV/AAD language added.
> This file is retained as the change record. E8 remains a substantive normative
> addition for Digital Video Subcommittee ratification.


Precise, drop-in corrections for **ANSI/SCTE 130-9 (SESAME) draft v0.4**, ready to
paste into the LaTeX source. They bring the paper into byte-for-byte agreement
with the `rust-pois` reference implementation (`src/sesame/`), fix one critical
crypto bug, replace the unmeasured performance table with real numbers, and close
the interop gaps so a third party can implement SESAME from the text alone.

| # | Severity | Issue | Fix |
|---|---|---|---|
| E1 | **🔴 Critical (crypto bug)** | Appendix A.4 reuses one GCM IV on request **and** response under the same `EncKeyId` — nonce reuse, catastrophic under AES-GCM (NIST SP 800-38D §8) | Distinct CSPRNG IV per message; add normative IV-uniqueness SHALL |
| E2 | Gap (interop) | Tier-3 GCM **AAD is unspecified** (§8.4) — a third party cannot reproduce the tag | Define AAD as the SESAME header set |
| E3 | Gap (interop) | **Encrypt-then-MAC ordering** never stated; §8.2.1 says "hash of the body" without saying it is the ciphertext | State EtM explicitly |
| E6 | **Accuracy (must-fix)** | §9.2 performance table predates any implementation; its numbers were never measured and are 25–40× off | Replace with measured c6i.xlarge results |
| E7 | Gap (interop) [BO-1] | Tier-2 scope is required in the signature (§8.3) but absent from the §8.2.2 canonical-string ABNF — contradiction, no worked example | Amend ABNF + add a Tier-2 example |
| E8 | **Substantive (needs WG sign-off)** [BO-2,BO-3] | Response signing appears only in informative Appendix A; no normative requirement and no defined response canonical string — yet a forged POIS response is the headline threat | Add normative §8.2.6 Response Signing |

(E4/E5 below are the normative-summary and definitions additions supporting E1–E3.)

Throughout, `LF` = `%x0A` (newline), per the existing §8.2.2 convention.
Items **E7** and **E8** encode constructions the reference implementation already
uses; they are flagged for Digital Video Subcommittee ratification because they
change/extend normative text rather than just fixing an error.

---

## E1 — GCM IV reuse (critical)

### E1a. Appendix A.4 — Response example: change the IV (and make the signature/processing distinct)

The response in A.4 currently repeats the request's IV under the same encryption
key. A GCM `(key, IV)` pair must never repeat; reuse destroys confidentiality and
allows authentication-tag forgery.

**Before** (Appendix A.4, *Response* block):

```
HTTP/1.1 200 OK
Content-Type: application/octet-stream
X-SESAME-Version: 1.0
X-SESAME-KeyId: pois-primary
X-SESAME-Timestamp: 2026-02-24T18:00:00Z
X-SESAME-Nonce: 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d
X-SESAME-Signature: 8a7b6c5d4e3f2a1b...(64 hex chars)
X-SESAME-Encrypted: true
X-SESAME-EncKeyId: enc-sportsfeed-2026q1
X-SESAME-IV: 7c8d9e0f1a2b3c4d5e6f7a8b
X-POIS-Processing-Time: 13ms

[AES-256-GCM encrypted SignalProcessingNotification | GCM tag]
```

**After** (IV changed to a distinct value; signature placeholder made distinct so
the example does not imply signature reuse either):

```
HTTP/1.1 200 OK
Content-Type: application/octet-stream
X-SESAME-Version: 1.0
X-SESAME-KeyId: pois-primary
X-SESAME-Timestamp: 2026-02-24T18:00:00Z
X-SESAME-Nonce: 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d
X-SESAME-Signature: 2e1f0a9b8c7d6e5f...(64 hex chars)
X-SESAME-Encrypted: true
X-SESAME-EncKeyId: enc-sportsfeed-2026q1
X-SESAME-IV: 3b1f8c0a9d2e4f6b8a0c1d2e
X-POIS-Processing-Time: 13ms

[AES-256-GCM encrypted SignalProcessingNotification | GCM tag]
```

> Add an italic caption beneath the example: *"Note: the response IV
> (`3b1f8c0a9d2e4f6b8a0c1d2e`) differs from the request IV
> (`7c8d9e0f1a2b3c4d5e6f7a8b`). Each is generated fresh by a CSPRNG; an IV is
> never reused with a given encryption key (see §8.4)."*

### E1b. §8.4 — Add the normative IV requirement

**Before** (§8.4, final paragraph):

> The authentication tag produced by GCM SHALL be appended to the ciphertext,
> providing authenticated encryption. The POIS decrypts the payload, processes it
> as standard ESAM XML, encrypts the response, and returns it with corresponding
> encryption headers. Encryption keys SHALL be managed separately from signing
> keys, enabling independent rotation schedules.

**After:**

> The authentication tag produced by GCM SHALL be appended to the ciphertext,
> providing authenticated encryption. A fresh initialization vector SHALL be
> generated by a CSPRNG for every encryption operation and SHALL NOT be reused
> with a given encryption key, in accordance with [NIST SP 800-38D] §8. In
> particular, a request and its response SHALL use independent IVs even when they
> share an encryption key. The POIS decrypts the payload, processes it as standard
> ESAM XML, encrypts the response **under a newly generated IV**, and returns it
> with the corresponding encryption headers. Encryption keys SHALL be managed
> separately from signing keys, enabling independent rotation schedules.

---

## E2 — Specify the Tier-3 AAD

GCM authenticates additional authenticated data (AAD) alongside the ciphertext.
The draft never says what the AAD is, so two conformant implementations would
compute different tags and fail to interoperate. Binding the SESAME header set as
AAD prevents an attacker from swapping those headers under the encryption.

**Insert** into §8.4, immediately after the `X-SESAME-...` header block (before
"The authentication tag produced by GCM..."):

> The GCM additional authenticated data (AAD) SHALL be the concatenation, in the
> following order and separated by LF (`%x0A`), of the values of:
>
> ```
> X-SESAME-Version LF
> X-SESAME-KeyId   LF
> X-SESAME-Timestamp LF
> X-SESAME-Nonce
> ```
>
> When Tier 2 is active, the value of `X-SESAME-Scope` SHALL be appended as a
> further LF-separated field. The AAD binds the ciphertext to these headers so
> that they cannot be altered in transit without causing tag verification to fail.

*(The `X-SESAME-IV` and `X-SESAME-EncKeyId` headers are bound implicitly: a change
to either causes decryption — and thus tag verification — to fail. They are
therefore not repeated in the AAD.)*

---

## E3 — State encrypt-then-MAC explicitly

§8.2.1 says the Tier-1 signature covers "the SHA-256 hash of the request body,"
and §8.4 says the ciphertext "replaces the original body." Together these imply
the Tier-1 hash is over the ciphertext (encrypt-then-MAC), but it is never stated,
leaving room for an implementer to hash the plaintext instead.

### E3a. §8.2.1 — clarify the body hash

**Before** (§8.2.1, second sentence):

> This approach ensures that both the request metadata and the full XML payload
> are covered by the integrity check without requiring the POIS to buffer the
> entire body before validation.

**After:**

> This approach ensures that both the request metadata and the full payload are
> covered by the integrity check without requiring the POIS to buffer the entire
> body before validation. The body hash is computed over the body **as
> transmitted**: when Tier 3 is active, this is the AES-256-GCM ciphertext
> (including the appended tag), so SESAME is an **encrypt-then-MAC** construction.

---

## E4 — §8.5 Normative Requirements Summary: add Tier-3 items

Append the following to the numbered list in §8.5 (continuing the existing
numbering, which currently ends at item 9):

> 10. For Tier 3, the initialization vector SHALL be generated by a CSPRNG, SHALL
>     be 96 bits, and SHALL NOT be reused with a given encryption key. A request
>     and its response SHALL use independent IVs.
> 11. For Tier 3, the GCM AAD SHALL be the LF-separated concatenation of
>     `X-SESAME-Version`, `X-SESAME-KeyId`, `X-SESAME-Timestamp`,
>     `X-SESAME-Nonce`, and — when Tier 2 is active — `X-SESAME-Scope`.
> 12. The GCM authentication tag SHALL be 128 bits and SHALL be appended to the
>     ciphertext.
> 13. When Tier 3 is active, the SHA-256 body hash of the Tier-1 canonical string
>     SHALL be computed over the ciphertext (encrypt-then-MAC).

---

## E5 — (optional) §5.2 Definitions

Add, for completeness, alongside the existing definitions:

> **initialization vector (IV):** A 96-bit value supplied to AES-256-GCM for each
> encryption. Generated by a CSPRNG and never reused with the same key.

---

## E6 — Replace the §9.2 performance table with measured results

The current §9.2 table ("AWS c6i.xlarge, 4 vCPU, 8 GB RAM") was written before
any implementation existed; its figures are not measurements. The benchmark has
now been run on an **actual c6i.xlarge** (Intel Xeon Platinum 8375C, Ice Lake,
AES-NI + SHA extensions present), Ubuntu 24.04, rustc 1.96.0, over representative
ESAM payload sizes. Replace the §9.2 table with the following (and add the
hardware/CPU row). Full methodology: `docs/benchmarks.md`.

**Before** (§9.2, the three-row table with 0.04/0.08, 0.05/0.09, 0.32/0.51 ms).

**After:**

> Benchmarks on an AWS c6i.xlarge (Intel Xeon Platinum 8375C, AES-NI + SHA
> extensions) demonstrate the following per-request overhead. p50 is the
> Criterion median; p99 is the 99th percentile of per-sample iteration means.
>
> | Operation | p50 (1 KB) | p99 (1 KB) | p50 (16 KB) | p99 (16 KB) |
> |---|---|---|---|---|
> | Tier 1 (HMAC verify) | 1.37 µs | 1.39 µs | 0.013 ms | 0.013 ms |
> | Tier 2 (+ scope check) | +0.06 µs | +0.07 µs | +0.06 µs | +0.07 µs |
> | Tier 3 (+ AES-GCM decrypt) | 3.31 µs | 3.43 µs | 0.027 ms | 0.027 ms |
> | Full outbound (encrypt + sign) | 4.23 µs | 4.25 µs | 0.028 ms | 0.028 ms |
>
> All tiers operate well within the 1 ms budget of Section 7 — the worst-case
> combined path is 0.028 ms (≈ 35× margin). Measurements without hardware crypto
> acceleration (software AES/SHA) remain sub-millisecond (≤ 0.22 ms p99),
> confirming the deploy-anywhere thesis.

> Note: this *improves* the paper's claim — the real overhead is far below the
> figures previously printed. §9.1's "implemented as Axum middleware… intercepts
> ESAM requests" is now accurate against the shipped code, and §9 may cite the
> repository at its tagged commit.

---

## E7 — [BO-1] Bind the Tier-2 scope into the canonical string

§8.3 states the scope header "SHALL be included in the signature computation,"
but the §8.2.2 ABNF fixes the canonical string at five fields with no scope, and
no Tier-2 example is given. Amend the ABNF so the binding is reproducible.

**Before** (§8.2.2 ABNF, first production):

```
canonical-string = method LF path LF timestamp LF nonce LF body-hash
```

**After:**

```
canonical-string = base-fields [ LF scope ]
base-fields      = method LF path LF timestamp LF nonce LF body-hash
scope            = scope-value         ; the exact X-SESAME-Scope header value,
                                       ; e.g. "channel=SportsFeed-East";
                                       ; present if and only if Tier 2 is active
```

**Add** a Tier-2 worked example after §8.2.3 (mirroring the existing one):

> For the Tier-2 request of Appendix A.3 (`X-SESAME-Scope: channel=SportsFeed-East`),
> the canonical string is:
>
> ```
> POST\n/esam?channel=SportsFeed-East\n2026-02-24T18:00:00Z\n
> c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8\n<SHA-256 hex of body>\n
> channel=SportsFeed-East
> ```

---

## E8 — [BO-2, BO-3] Make response signing normative (new §8.2.6)

A forged or tampered POIS **response** (spoofed blackout/avail/redirect) is the
highest-value attack in the ESAM exchange, yet the draft only *illustrates*
signed responses in informative Appendix A — there is no normative requirement
and no defined canonical string for a response (which has no method or request
path). Add the following normative subsection.

**Insert** as new §8.2.6:

> #### 8.2.6 Response Signing
>
> A SESAME Server SHALL sign every response it returns to an authenticated
> request, using the same HMAC-SHA256 construction as Section 8.2.1 with the
> server's own credential. The server populates `X-SESAME-Version`,
> `X-SESAME-KeyId`, `X-SESAME-Timestamp`, `X-SESAME-Nonce`, and
> `X-SESAME-Signature` on the response, generating a fresh nonce per response.
>
> Because a response carries no HTTP method or request-target, the response
> canonical string is defined as:
>
> ```
> response-canonical = "RESPONSE" LF correlation LF timestamp LF nonce
>                      LF body-hash [ LF scope ]
> correlation        = acquisition-signal-id   ; the acquisitionSignalID of the
>                                               ; AcquiredSignal being answered
> ```
>
> The `correlation` field binds the signed response to the specific request
> signal it answers, preventing a captured response from being substituted for a
> different request. When Tier 3 is active on the response, `body-hash` is taken
> over the response ciphertext (encrypt-then-MAC, Section 8.4), and the response
> carries its own fresh `X-SESAME-IV`.
>
> A SESAME Client SHALL verify the response signature and SHALL reject a response
> whose signature, freshness, or correlation does not validate.

**Add** to the §8.5 Normative Requirements Summary:

> 14. A SESAME Server SHALL sign every response to an authenticated request per
>     Section 8.2.6, and a SESAME Client SHALL verify it.

*(WG note: this promotes response authentication from informative to normative.
It is the single most security-relevant change in this errata set — recommend
adopting it, since the threat model in Section 6 turns on response integrity.)*

---

## Conformance check

The `rust-pois` reference implementation already conforms to all of the above:

* Fresh per-message IV: `tier3_aead::random_iv()` (OS CSPRNG), called once per
  `sign_response` / per client request. Regression test:
  `sesame::tests::tier3_response_uses_fresh_iv`.
* AAD layout: `tier3_aead::aad_for_headers(version, key_id, timestamp, nonce,
  scope)`. Negative test: `tier3_aead::tests::wrong_aad_rejected`.
* Encrypt-then-MAC: `mod.rs` hashes the ciphertext (`body_hash_hex(&body)` where
  `body` is the seal output) before signing.
* 128-bit tag / 96-bit IV: `tier3_aead` constants `IV_LEN = 12`; tag length is the
  `aes-gcm` crate default (16 bytes), asserted in `seal_open_roundtrip`.
* Tier-2 scope binding (E7): `canonical::request_canonical(.., scope)` appends the
  scope line; test `canonical::tests::tier2_appends_scope_line`.
* Response signing + canonical (E8): `sign_response` / `response_canonical`; tests
  `response_sign_and_client_verify_roundtrip`, `forged_response_detected`.
* Performance (E6): measured via `scripts/run_c6i_bench.sh`; numbers in
  `docs/benchmarks.md`.

Applying these errata to the source closes all of **[BO-1], [BO-2], [BO-3],
[BO-4], [BO-5], [BO-6], [BO-9]** in `SESAME_reconciliation.md`. The remaining open
items there ([BO-7] error-leak posture, [BO-8] request-target form) are judgment
calls for the WG, not blocking defects.

// src/sesame_axum.rs
//
// Thin Axum adapter wiring the framework-agnostic SESAME core into the rust-pois
// ESAM exchange, bidirectionally. This is the only SESAME file that depends on
// Axum; all crypto/protocol logic lives in `pois_esam_server::sesame`.
//
// Inbound:  parse headers -> verify_request (Tier 1 sig+freshness+replay,
//           Tier 2 authz, Tier 3 decrypt) BEFORE the ESAM XML is parsed.
// Outbound: sign_response (Tier 1) and, if enabled, Tier 3-encrypt the POIS's
//           SignalProcessingNotification — the primary forged-response defense.
//
// Runtime configuration is loaded from environment (key distribution is out of
// band per §8.2.5). See `SesameRuntime::from_env`.

use std::sync::Arc;

use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use time::OffsetDateTime;

use pois_esam_server::sesame::keys::{AeadKey, ChannelScope, HmacKey, KeyProvider, StaticKeyProvider};
use pois_esam_server::sesame::message::{hex_decode, SesameError};
use pois_esam_server::sesame::replay::{InMemoryReplayCache, ReplayCache};
use pois_esam_server::sesame::{
    self, RequestContext, ResponseParams, SesameConfig, SesameHeaders, SignedResponse,
    VerifiedRequest,
};

// Re-export `Tier` so the binary can refer to `sesame_axum::Tier` without
// depending on the library path directly.
pub use pois_esam_server::sesame::Tier;

/// Process-wide SESAME state, held in `AppState`.
pub struct SesameRuntime {
    pub cfg: SesameConfig,
    pub provider: Arc<dyn KeyProvider>,
    pub replay: Arc<dyn ReplayCache>,
    /// Default minimum tier when a channel has no explicit policy / is unknown.
    pub default_min_tier: Tier,
    /// This POIS's signing key-id for outbound responses (X-SESAME-KeyId).
    pub response_key_id: Option<String>,
    /// This POIS's encryption key-id for Tier 3 responses (X-SESAME-EncKeyId).
    pub response_enc_key_id: Option<String>,
    /// Master switch. When false the adapter is a transparent passthrough so
    /// existing (Tier 0) deployments behave exactly as before SESAME existed.
    enabled: bool,
}

impl SesameRuntime {
    /// Build from environment variables:
    ///   POIS_SESAME_MIN_TIER       default minimum tier (0..3), default 0
    ///   POIS_SESAME_REPLAY_WINDOW  replay window seconds, default 300
    ///   POIS_SESAME_RESPONSE_KEYID signing key-id used to sign responses
    ///   POIS_SESAME_RESPONSE_ENCID encryption key-id used for Tier 3 responses
    ///   POIS_SESAME_KEYS           JSON array describing keys (see parse_keys_json)
    pub fn from_env() -> Self {
        let default_min_tier = std::env::var("POIS_SESAME_MIN_TIER")
            .ok()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .map(Tier::from_u8)
            .unwrap_or(Tier::Zero);

        let window = std::env::var("POIS_SESAME_REPLAY_WINDOW")
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(300);

        let keys_env = std::env::var("POIS_SESAME_KEYS").ok();
        let provider = keys_env
            .as_deref()
            .and_then(parse_keys_json)
            .unwrap_or_default();

        let response_key_id = std::env::var("POIS_SESAME_RESPONSE_KEYID").ok();

        // SESAME is active if any of: keys are configured, a minimum tier is
        // required, or response signing is requested. Otherwise the adapter is
        // a no-op and the ESAM path behaves exactly as before.
        let enabled =
            keys_env.is_some() || default_min_tier != Tier::Zero || response_key_id.is_some();

        SesameRuntime {
            cfg: SesameConfig {
                replay_window_secs: window,
            },
            provider: Arc::new(provider),
            replay: Arc::new(InMemoryReplayCache::new(window)),
            default_min_tier,
            response_key_id,
            response_enc_key_id: std::env::var("POIS_SESAME_RESPONSE_ENCID").ok(),
            enabled,
        }
    }

    /// True if SESAME is active. When false, callers should skip SESAME entirely
    /// (transparent passthrough) so existing deployments are unaffected.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// A SESAME rejection rendered as the paper's JSON error body (Appendix A.7).
pub struct SesameRejection {
    err: SesameError,
    key_id: Option<String>,
    detail: String,
}

impl SesameRejection {
    fn new(err: SesameError, key_id: Option<String>, detail: impl Into<String>) -> Self {
        SesameRejection {
            err,
            key_id,
            detail: detail.into(),
        }
    }

    pub fn error_code(&self) -> &'static str {
        self.err.code()
    }
}

/// Reject because the request's achieved tier is below the channel's required
/// minimum (§9.3). Surfaced as `sesame_missing_headers` (the request lacked the
/// SESAME protection the channel requires).
pub fn reject_insufficient_tier(key_id: Option<String>, required: Tier, achieved: Tier) -> SesameRejection {
    SesameRejection::new(
        SesameError::MissingHeaders,
        key_id,
        format!(
            "Channel requires SESAME tier {} but request achieved tier {}",
            required.level(),
            achieved.level()
        ),
    )
}

/// Reject because the Tier-2 declared scope does not match the channel the
/// request actually resolved to.
pub fn reject_scope_mismatch(key_id: Option<String>, declared: &str, resolved: &str) -> SesameRejection {
    SesameRejection::new(
        SesameError::ScopeDenied,
        key_id,
        format!("Declared scope channel {declared} does not match target channel {resolved}"),
    )
}

impl IntoResponse for SesameRejection {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.err.http_status()).unwrap_or(StatusCode::UNAUTHORIZED);
        let body = serde_json::json!({
            "error": self.err.code(),
            "detail": self.detail,
            "key_id": self.key_id,
        });
        (status, axum::Json(body)).into_response()
    }
}

/// Outcome of inbound verification: the (decrypted) body plus the SESAME context
/// to use when signing the response.
pub struct Incoming {
    pub body: Vec<u8>,
    /// `None` for an unauthenticated Tier-0 passthrough.
    pub ctx: Option<IncomingCtx>,
}

pub struct IncomingCtx {
    pub key_id: String,
    pub scope_channel: Option<String>,
    pub achieved_tier: Tier,
}

/// Verify an inbound ESAM request. `route_channel` is the channel from the URL
/// (`/esam/channel/{channel}`) when present. Returns the body to hand to the
/// existing ESAM pipeline, or a rejection to short-circuit with.
pub fn verify_incoming(
    rt: &SesameRuntime,
    method: &str,
    path: &str,
    route_channel: Option<&str>,
    headers: &HeaderMap,
    raw_body: &[u8],
) -> Result<Incoming, SesameRejection> {
    let parsed = parse_headers(headers);
    let now = OffsetDateTime::now_utc();
    let ctx = RequestContext {
        method,
        path,
        target_channel: route_channel,
    };
    let min_tier = rt.default_min_tier;

    match sesame::verify_request(
        &rt.cfg,
        rt.provider.as_ref(),
        rt.replay.as_ref(),
        &ctx,
        &parsed,
        raw_body,
        now,
        min_tier,
    ) {
        Ok(VerifiedRequest {
            plaintext,
            key_id,
            scope_channel,
            achieved_tier,
        }) => {
            let ctx = if achieved_tier == Tier::Zero {
                None
            } else {
                Some(IncomingCtx {
                    key_id,
                    scope_channel,
                    achieved_tier,
                })
            };
            Ok(Incoming {
                body: plaintext,
                ctx,
            })
        }
        Err(err) => Err(SesameRejection::new(
            err,
            parsed.key_id.clone(),
            describe(err, &parsed),
        )),
    }
}

/// Sign (and optionally encrypt) the outbound ESAM response. Returns the headers
/// to attach, the body to send, and the content-type. Returns `None` when SESAME
/// response signing is not configured (transparent passthrough).
pub fn sign_outgoing(
    rt: &SesameRuntime,
    correlation: &str,
    response_xml: &[u8],
    scope_channel: Option<&str>,
    tier: Tier,
) -> Option<SignedResponse> {
    let key_id = rt.response_key_id.as_deref()?;
    let scope = scope_channel.map(|c| format!("channel={c}"));
    let params = ResponseParams {
        signing_key_id: key_id,
        correlation,
        scope: scope.as_deref(),
        tier,
        enc_key_id: rt.response_enc_key_id.as_deref(),
    };
    match sesame::sign_response(&rt.cfg, rt.provider.as_ref(), &params, response_xml, OffsetDateTime::now_utc()) {
        Ok(signed) => Some(signed),
        Err(_) => None, // fail open on signing only if misconfigured; logged by caller
    }
}

/// Build the final ESAM HTTP response, signing (and optionally encrypting) it
/// when an authenticated SESAME context is present and response signing is
/// configured. Falls back to a plain `application/xml` body otherwise, so a
/// Tier-0 request yields exactly the legacy response.
///
/// `tier` is taken from the inbound request's achieved tier: a request that
/// arrived Tier-3 encrypted gets a Tier-3 encrypted response; a Tier-1 request
/// gets a signed-but-cleartext response.
pub fn build_esam_response(
    rt: &SesameRuntime,
    ctx: Option<&IncomingCtx>,
    correlation: &str,
    response_xml: &str,
) -> Response {
    if let (true, Some(ctx)) = (rt.response_key_id.is_some(), ctx) {
        let scope = ctx.scope_channel.as_deref();
        if let Some(signed) =
            sign_outgoing(rt, correlation, response_xml.as_bytes(), scope, ctx.achieved_tier)
        {
            let headers = response_headers(&signed);
            return (StatusCode::OK, headers, signed.body).into_response();
        }
    }
    // Legacy / Tier-0 response.
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/xml")],
        response_xml.to_string(),
    )
        .into_response()
}

/// Convert SESAME response headers into an axum `HeaderMap`.
pub fn response_headers(signed: &SignedResponse) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in &signed.headers {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.insert(n, v);
        }
    }
    if let Ok(ct) = HeaderValue::from_str(signed.content_type) {
        map.insert(axum::http::header::CONTENT_TYPE, ct);
    }
    map
}

// -------------------------------------------------------------------------
// helpers
// -------------------------------------------------------------------------

fn parse_headers(headers: &HeaderMap) -> SesameHeaders {
    SesameHeaders::from_lookup(|name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    })
}

/// Build the Appendix A.7 `detail` string. Deliberately avoids echoing whether a
/// key exists beyond what the error code already implies.
fn describe(err: SesameError, h: &SesameHeaders) -> String {
    match err {
        SesameError::ScopeDenied => format!(
            "Key {} not authorized for {}",
            h.key_id.as_deref().unwrap_or("?"),
            h.scope.as_deref().unwrap_or("requested scope")
        ),
        SesameError::ReplayDetected => format!(
            "Nonce {} already used",
            h.nonce.as_deref().unwrap_or("?")
        ),
        other => other.code().replace('_', " "),
    }
}

/// Parse `POIS_SESAME_KEYS` JSON into a `StaticKeyProvider`. Format:
/// ```json
/// {
///   "signing": [
///     {"key_id":"sas-east-01","secret_hex":"...","channels":["SportsFeed-East"]},
///     {"key_id":"pois-primary","secret_hex":"...","channels":["*"]}
///   ],
///   "encryption": [
///     {"enc_key_id":"enc-sportsfeed-2026q1","key_hex":"<64 hex chars>"}
///   ]
/// }
/// ```
fn parse_keys_json(json: &str) -> Option<StaticKeyProvider> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let mut provider = StaticKeyProvider::new();

    if let Some(signing) = v.get("signing").and_then(|s| s.as_array()) {
        for entry in signing {
            let key_id = entry.get("key_id")?.as_str()?;
            let secret_hex = entry.get("secret_hex")?.as_str()?;
            let secret = hex_decode(secret_hex)?;
            let channels: Vec<String> = entry
                .get("channels")
                .and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let scope = if channels.iter().any(|c| c == "*") {
                ChannelScope::all()
            } else {
                ChannelScope::list(channels)
            };
            provider = provider.with_signing_key(key_id, HmacKey(secret), scope);
        }
    }

    if let Some(enc) = v.get("encryption").and_then(|s| s.as_array()) {
        for entry in enc {
            let enc_key_id = entry.get("enc_key_id")?.as_str()?;
            let key_hex = entry.get("key_hex")?.as_str()?;
            let bytes = hex_decode(key_hex)?;
            if bytes.len() != 32 {
                return None;
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            provider = provider.with_aead_key(enc_key_id, AeadKey(k));
        }
    }

    Some(provider)
}

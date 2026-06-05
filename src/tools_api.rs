// src/tools_api.rs
// Version: 4.0.7
// Created: 2024-11-17
// Updated: 2026-03-12
// 
// Enhanced SCTE-35 Tools API - Decoder, Validator, Test Sender, Advanced Builder
//
// Changelog:
// v4.0.7 (2026-03-12): Fixed /api/tools/scte35/build endpoint
//   - BuildRequest now accepts segmentation_type_id, segmentation_upid_type, segmentation_upid
//   - build_scte35 handler routes to advanced builder when segmentation params present
//   - "time_signal" command alias added (frontend sends "time_signal", not "time_signal_immediate")
//   - Removed #[allow(dead_code)] from build_advanced_internal and AdvancedBuildRequest
// v4.0.6 (2024-11-24): CRITICAL FIX - Removed incorrect skip logic from v4.0.4
//   - v4.0.4's skip logic was causing descriptor_loop_length to be read from wrong position
//   - Command parsers already consume correct bytes; no skip needed
//   - Now correctly reads descriptor_loop_length at bitpos 232 instead of 240
//   - Segmentation descriptors now parse correctly!
// v4.0.5 (2024-11-24): Fixed descriptor_loop_length parsing to mask 6 reserved bits
//   - CRITICAL FIX: descriptor_loop_length is 10 bits, not 16 bits
//   - Now properly masks the 6 reserved bits: word & 0x03FF
//   - This was causing descriptors to never be found (reading 8194 bytes instead of 32)
// v4.0.4 (2024-11-24): Fixed command byte skipping for proper descriptor parsing
//   - NOTE: This introduced a bug that was fixed in v4.0.6
// v4.0.3 (2024-11-20): Fixed segmentation descriptor parsing
//   - CRITICAL FIX: Now properly reads 4-byte "CUEI" identifier before segmentation_event_id
// v4.0.2 (2024-11-19): Enhanced logging for descriptor parsing debugging
// v4.0.1 (2024-11-19): Enhanced decoder for segmentation descriptors
// v4.0.0 (2024-11-19): Full segmentation descriptor support
// v3.1.2 (2024-11-17): Quick Test now processes real ESAM signals
// v3.1.1 (2024-11-17): Fixed JWT authentication integration
// v3.1.0 (2024-11-17): Initial release

use axum::{
    extract::State,
    Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use crate::scte35;
use crate::AppState;
use crate::jwt_auth;

// ============================================================================
// REQUEST/RESPONSE TYPES
// ============================================================================

#[derive(Deserialize)]
pub struct BuildRequest {
    pub command: String,
    pub duration_seconds: Option<u32>,
    pub segmentation_type_id: Option<String>,
    pub segmentation_upid_type: Option<String>,
    pub segmentation_upid: Option<String>,
}

#[derive(Serialize)]
pub struct BuildResponse {
    pub base64: String,
}

#[derive(Deserialize)]
pub struct DecodeRequest {
    pub base64: String,
}

#[derive(Serialize)]
pub struct DecodeResponse {
    pub valid: bool,
    pub error: Option<String>,
    pub decoded: Option<DecodedScte35>,
}

#[derive(Serialize)]
pub struct DecodedScte35 {
    pub table_id: String,
    pub protocol_version: u8,
    pub encrypted_packet: bool,
    pub pts_adjustment: u64,
    pub command_type: String,
    pub command_type_id: u8,
    pub command_info: serde_json::Value,
    pub descriptors: Vec<DescriptorInfo>,
    pub raw_hex: String,
}

#[derive(Serialize)]
pub struct DescriptorInfo {
    pub tag: u8,
    pub tag_name: String,
    pub length: usize,
    pub data: serde_json::Value,
}

#[derive(Deserialize)]
pub struct ValidateRequest {
    pub base64: String,
}

#[derive(Serialize)]
pub struct ValidateResponse {
    pub valid: bool,
    pub error: Option<String>,
    pub info: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct AdvancedBuildRequest {
    pub command: String,
    pub duration_seconds: Option<u32>,
    pub segmentation_type_id: Option<String>,
    pub segmentation_upid_type: Option<String>,
    pub segmentation_upid: Option<String>,
    pub event_id: Option<u32>,
    pub pts_time: Option<u64>,
}

#[derive(Deserialize)]
pub struct TestSendRequest {
    pub channel_id: i64,
    pub base64: String,
}

#[derive(Serialize)]
pub struct TestSendResponse {
    pub success: bool,
    pub message: String,
    pub event_id: Option<i64>,
}

// ============================================================================
// API HANDLERS
// ============================================================================

/// POST /api/tools/scte35/build - Basic SCTE-35 builder (with optional segmentation)
pub async fn build_scte35(
    State(_st): State<std::sync::Arc<AppState>>,
    Extension(_claims): Extension<jwt_auth::Claims>,
    Json(req): Json<BuildRequest>,
) -> Response {
    // Route to advanced builder if segmentation params present
    if req.segmentation_type_id.is_some() || req.segmentation_upid_type.is_some() {
        let adv = AdvancedBuildRequest {
            command: req.command,
            duration_seconds: req.duration_seconds,
            segmentation_type_id: req.segmentation_type_id,
            segmentation_upid_type: req.segmentation_upid_type,
            segmentation_upid: req.segmentation_upid,
            event_id: None,
            pts_time: None,
        };
        return match build_advanced_internal(&adv) {
            Ok(b64) => Json(BuildResponse { base64: b64 }).into_response(),
            Err(e) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": e})),
            )
                .into_response(),
        };
    }

    let b64 = match req.command.as_str() {
        "time_signal_immediate" | "time_signal" => scte35::build_time_signal_immediate_b64(),
        "splice_insert_out" => {
            let dur = req.duration_seconds.unwrap_or(60);
            scte35::build_splice_insert_out_b64(dur)
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "unknown command"})),
            )
                .into_response()
        }
    };

    Json(BuildResponse { base64: b64 }).into_response()
}

/// POST /api/tools/scte35/decode - Decode SCTE-35 Base64 to human-readable
pub async fn decode_scte35(
    State(_st): State<std::sync::Arc<AppState>>,
    Extension(_claims): Extension<jwt_auth::Claims>,
    Json(req): Json<DecodeRequest>,
) -> Response {
    match decode_scte35_internal(&req.base64) {
        Ok(decoded) => Json(DecodeResponse {
            valid: true,
            error: None,
            decoded: Some(decoded),
        })
        .into_response(),
        Err(e) => Json(DecodeResponse {
            valid: false,
            error: Some(e),
            decoded: None,
        })
        .into_response(),
    }
}

/// POST /api/tools/scte35/validate - Validate SCTE-35 Base64
pub async fn validate_scte35(
    State(_st): State<std::sync::Arc<AppState>>,
    Extension(_claims): Extension<jwt_auth::Claims>,
    Json(req): Json<ValidateRequest>,
) -> Response {
    match validate_scte35_internal(&req.base64) {
        Ok(info) => Json(ValidateResponse {
            valid: true,
            error: None,
            info: Some(info),
        })
        .into_response(),
        Err(e) => Json(ValidateResponse {
            valid: false,
            error: Some(e),
            info: None,
        })
        .into_response(),
    }
}

/// POST /api/tools/scte35/build-advanced - Advanced builder with segmentation
#[allow(dead_code)]
pub async fn build_advanced_scte35(
    State(_st): State<std::sync::Arc<AppState>>,
    Extension(_claims): Extension<jwt_auth::Claims>,
    Json(req): Json<AdvancedBuildRequest>,
) -> Response {
    match build_advanced_internal(&req) {
        Ok(b64) => Json(BuildResponse { base64: b64 }).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

/// POST /api/tools/scte35/test-send - Send test signal to channel
pub async fn test_send(
    State(st): State<std::sync::Arc<AppState>>,
    Extension(claims): Extension<jwt_auth::Claims>,
    Json(req): Json<TestSendRequest>,
) -> Response {
    use crate::esam::{extract_facts, build_notification};
    use crate::rules::rule_matches;
    use crate::models::Rule;
    use crate::event_logging::{ClientInfo, ProcessingMetrics};
    use std::time::Instant;
    
    // Verify channel exists and user has access
    let channel_check: Result<Option<(i64, String)>, _> = sqlx::query_as(
        "SELECT id, name FROM channels WHERE id = ? AND deleted_at IS NULL"
    )
    .bind(req.channel_id)
    .fetch_optional(&st.db)
    .await;

    let (channel_id, channel_name) = match channel_check {
        Ok(Some((id, name))) => (id, name),
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "channel not found"})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("database error: {}", e)})),
            )
                .into_response();
        }
    };

    // Check ownership for non-admin users
    if claims.role != "admin" {
        let user_id: i64 = claims.sub.parse().unwrap_or(0);
        let owner_check: Result<Option<(Option<i64>,)>, _> = sqlx::query_as(
            "SELECT owner_user_id FROM channels WHERE id = ?"
        )
        .bind(channel_id)
        .fetch_optional(&st.db)
        .await;
        
        match owner_check {
            Ok(Some((Some(owner_id),))) if owner_id != user_id => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "not your channel"})),
                )
                    .into_response();
            }
            Ok(Some((None,))) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "cannot test on system channel"})),
                )
                    .into_response();
            }
            _ => {}
        }
    }
    
    // Build a proper ESAM XML request with the SCTE-35 signal
    let test_signal_id = format!("QUICKTEST-{}", chrono::Utc::now().timestamp_millis());
    let utc_point = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    
    let esam_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<SignalProcessingEvent xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1" xmlns:sig="urn:cablelabs:md:xsd:signaling:3.0">
  <AcquiredSignal acquisitionSignalID="{}">
    <sig:UTCPoint utcPoint="{}"/>
  </AcquiredSignal>
  <sig:BinaryData signalType="SCTE35">{}</sig:BinaryData>
</SignalProcessingEvent>"#,
        test_signal_id, utc_point, req.base64
    );
    
    let start = Instant::now();
    
    // Extract facts from the ESAM request
    let facts = match extract_facts(&esam_xml) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("invalid SCTE-35: {}", e)})),
            )
                .into_response();
        }
    };
    
    let obj = facts.as_object().cloned().unwrap_or_default();
    
    // Get rules for this channel
    let rules = match sqlx::query_as::<_, Rule>(
        "SELECT * FROM rules WHERE channel_id=? AND enabled=1 AND deleted_at IS NULL ORDER BY priority",
    )
    .bind(channel_id)
    .fetch_all(&st.db)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("failed to load rules: {}", e)})),
            )
                .into_response();
        }
    };
    
    // Find matching rule
    let matched_rule: Option<Rule> = rules.into_iter().find(|r| {
        let m: serde_json::Value =
            serde_json::from_str(&r.match_json).unwrap_or(serde_json::json!({}));
        rule_matches(&m, &obj)
    });
    
    // Build response based on matched rule or noop
    let (action, resp_xml) = if let Some(ref r) = matched_rule {
        let params: serde_json::Value = serde_json::from_str(&r.params_json).unwrap_or_default();
        let resp = build_notification(&test_signal_id, &utc_point, "", &r.action, &params, Some(&params));
        (r.action.clone(), resp)
    } else {
        let resp = build_notification(&test_signal_id, &utc_point, "", "noop", &serde_json::json!({}), None);
        ("noop".to_string(), resp)
    };
    
    let duration = start.elapsed();
    
    // Log the event with special marker for Quick Test
    let client_info = ClientInfo {
        source_ip: Some(format!("QuickTest:{}", claims.username)),
        user_agent: Some("POIS Quick Test Tool".to_string()),
        sesame_tier: None,
    };
    
    // Log the event - pass the actual matched rule if available
    let log_result = if let Some(ref rule) = matched_rule {
        st.event_logger.log_esam_event(
            &channel_name,
            &facts,
            Some((rule, action.as_str())),
            client_info,
            ProcessingMetrics {
                request_size: Some(esam_xml.len() as i32),
                processing_time_ms: Some(duration.as_millis() as i32),
                response_status: 200,
                error_message: None,
            },
            Some(&esam_xml),
            Some(&resp_xml),
        ).await
    } else {
        st.event_logger.log_esam_event(
            &channel_name,
            &facts,
            None,
            client_info,
            ProcessingMetrics {
                request_size: Some(esam_xml.len() as i32),
                processing_time_ms: Some(duration.as_millis() as i32),
                response_status: 200,
                error_message: None,
            },
            Some(&esam_xml),
            Some(&resp_xml),
        ).await
    };
    
    match log_result {
        Ok(event_id) => {
            let rule_info = matched_rule
                .as_ref()
                .map(|r| r.name.as_str())
                .unwrap_or("no match");
            
            Json(TestSendResponse {
                success: true,
                message: format!(
                    "Test signal processed: {} → {} | Check Event Monitor", 
                    channel_name,
                    rule_info
                ),
                event_id: Some(event_id),
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to log test event: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("logging failed: {}", e)})),
            )
                .into_response()
        }
    }
}

// ============================================================================
// INTERNAL DECODE LOGIC
// ============================================================================

/// Convert a hex string (no separators) to bytes.
fn hex_str_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("Hex input has an odd number of digits".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16).map_err(|_| "Invalid hex digit".to_string())
        })
        .collect()
}

/// Decode a SCTE-35 message supplied as base64, hex, or binary into raw bytes,
/// matching the input flexibility of common parsers (e.g. tools.middleman.tv):
///   - base64:  "/DAvAAAA..."        (the ESAM/wire form)
///   - hex:     "FC302F...", "0xFC...", "fc 30 2f", "fc:30:2f"
///   - binary:  "0b11111100...", or a whitespace-grouped run of 0/1
///
/// Whitespace and ':' separators are ignored. When the form is ambiguous (hex
/// digits are also valid base64), the interpretation whose first byte is the
/// SCTE-35 table_id (0xFC) is preferred.
fn scte35_input_to_bytes(raw: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect();
    if cleaned.is_empty() {
        return Err("Empty input".to_string());
    }

    // Binary: explicit "0b...", or an implicit run of 0/1 that begins with the
    // SCTE-35 table_id (11111100) and is a whole number of bytes.
    let bin = cleaned
        .strip_prefix("0b")
        .or_else(|| cleaned.strip_prefix("0B"))
        .unwrap_or(&cleaned);
    let explicit_bin = cleaned.starts_with("0b") || cleaned.starts_with("0B");
    if (explicit_bin || (bin.len() >= 16 && bin.starts_with("11111100")))
        && bin.len().is_multiple_of(8)
        && bin.bytes().all(|b| b == b'0' || b == b'1')
    {
        return Ok(bin
            .as_bytes()
            .chunks(8)
            .map(|c| c.iter().fold(0u8, |acc, &d| (acc << 1) | (d - b'0')))
            .collect());
    }

    // Explicit hex: "0x...".
    if let Some(h) = cleaned.strip_prefix("0x").or_else(|| cleaned.strip_prefix("0X")) {
        return hex_str_to_bytes(h);
    }

    // Ambiguous: try both base64 and hex, prefer a 0xFC-leading result.
    let as_b64 = B64.decode(cleaned.as_bytes()).ok().filter(|b| !b.is_empty());
    let as_hex = if cleaned.len().is_multiple_of(2) && cleaned.bytes().all(|b| b.is_ascii_hexdigit()) {
        hex_str_to_bytes(&cleaned).ok().filter(|b| !b.is_empty())
    } else {
        None
    };
    match (as_b64, as_hex) {
        (Some(b), Some(h)) => {
            if b.first() == Some(&0xFC) {
                Ok(b)
            } else if h.first() == Some(&0xFC) {
                Ok(h)
            } else {
                Ok(b)
            }
        }
        (Some(b), None) => Ok(b),
        (None, Some(h)) => Ok(h),
        (None, None) => Err("Input is not valid base64, hex, or binary".to_string()),
    }
}

fn decode_scte35_internal(input: &str) -> Result<DecodedScte35, String> {
    let bytes = scte35_input_to_bytes(input)?;

    if bytes.is_empty() {
        return Err("Empty data".to_string());
    }

    let table_id = bytes[0];
    if table_id != 0xFC {
        return Err(format!("Invalid table_id: 0x{:02X} (expected 0xFC)", table_id));
    }

    let mut br = BitReader::new(&bytes);
    
    // Parse header
    let _table_id = br.read_u8(8)?;
    let _section_syntax = br.read_u8(1)?;
    let _private_indicator = br.read_u8(1)?;
    let _reserved = br.read_u8(2)?;
    let _section_length = br.read_u16(12)?;
    
    let protocol_version = br.read_u8(8)?;
    let encrypted_packet = br.read_u8(1)? == 1;
    let _encryption_algorithm = br.read_u8(6)?;
    let pts_adjustment = br.read_u64(33)?;
    let _cw_index = br.read_u8(8)?;
    let _tier = br.read_u16(12)?;
    
    let splice_command_length = br.read_u16(12)? as usize;
    let command_type = br.read_u8(8)?;
    
    tracing::info!("Command type: 0x{:02X}, splice_command_length: {} bytes, bitpos: {}", 
                   command_type, splice_command_length, br.bitpos);
    
    let command_name = match command_type {
        0x00 => "splice_null",
        0x04 => "splice_schedule",
        0x05 => "splice_insert",
        0x06 => "time_signal",
        0x07 => "bandwidth_reservation",
        0xFF => "private_command",
        _ => "unknown",
    };
    
    let _command_start_pos = br.bitpos;
    let command_info = parse_command_info(&mut br, command_type)?;
    
    tracing::info!("After command parse, bitpos: {}", br.bitpos);
    tracing::info!("Before reading descriptor_loop_length, bitpos: {}, total message bits: {}", 
                   br.bitpos, br.data.len() * 8);
    
    // v4.0.5 FIX: descriptor_loop_length is 10 bits with 6 reserved bits
    let descriptor_word = br.read_u16(16)?;
    let descriptor_loop_length = (descriptor_word & 0x03FF) as usize;
    
    tracing::info!("Descriptor loop word: 0x{:04X}, masked length: {} bytes, bitpos now: {}", 
                   descriptor_word, descriptor_loop_length, br.bitpos);

    let mut descriptors = Vec::new();
    
    if descriptor_loop_length > 0 {
        let desc_start = br.bitpos;
        let desc_end = desc_start + (descriptor_loop_length * 8);
        
        while br.bitpos < desc_end {
            match parse_descriptor(&mut br) {
                Ok(desc) => {
                    descriptors.push(desc);
                }
                Err(e) => {
                    tracing::warn!("Failed to parse descriptor at bit {}: {}", br.bitpos, e);
                    if br.bitpos < desc_end {
                        let remaining_bits = desc_end - br.bitpos;
                        if let Err(skip_err) = br.skip_bits(remaining_bits as u32) {
                            tracing::error!("Failed to skip remaining descriptor bits: {}", skip_err);
                        }
                    }
                    break;
                }
            }
        }
    }
    
    Ok(DecodedScte35 {
        table_id: format!("0x{:02X}", table_id),
        protocol_version,
        encrypted_packet,
        pts_adjustment,
        command_type: command_name.to_string(),
        command_type_id: command_type,
        command_info,
        descriptors,
        raw_hex: bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" "),
    })
}

fn parse_command_info(
    br: &mut BitReader,
    command_type: u8,
) -> Result<serde_json::Value, String> {
    match command_type {
        0x00 => {
            Ok(serde_json::json!({ "command": "splice_null" }))
        }
        0x05 => {
            let event_id = br.read_u32(32)?;
            let event_cancel = br.read_u8(1)? == 1;
            br.skip_bits(7)?;
            
            if event_cancel {
                return Ok(serde_json::json!({
                    "command": "splice_insert",
                    "splice_event_id": event_id,
                    "splice_event_cancel_indicator": true
                }));
            }
            
            let out_of_network = br.read_u8(1)? == 1;
            let program_splice_flag = br.read_u8(1)? == 1;
            let duration_flag = br.read_u8(1)? == 1;
            let splice_immediate_flag = br.read_u8(1)? == 1;
            br.skip_bits(4)?;
            
            let mut result = serde_json::json!({
                "command": "splice_insert",
                "splice_event_id": event_id,
                "out_of_network_indicator": out_of_network,
                "program_splice_flag": program_splice_flag,
                "duration_flag": duration_flag,
                "splice_immediate_flag": splice_immediate_flag
            });
            
            if program_splice_flag && !splice_immediate_flag {
                let time_specified = br.read_u8(1)? == 1;
                if time_specified {
                    br.skip_bits(6)?;
                    let pts_time = br.read_u64(33)?;
                    result["pts_time"] = serde_json::json!(pts_time);
                } else {
                    br.skip_bits(7)?;
                }
            }
            
            if !program_splice_flag {
                let component_count = br.read_u8(8)?;
                let mut components = Vec::new();
                for _ in 0..component_count {
                    let tag = br.read_u8(8)?;
                    if !splice_immediate_flag {
                        let time_specified = br.read_u8(1)? == 1;
                        if time_specified {
                            br.skip_bits(6)?;
                            let pts = br.read_u64(33)?;
                            components.push(serde_json::json!({"tag": tag, "pts_time": pts}));
                        } else {
                            br.skip_bits(7)?;
                            components.push(serde_json::json!({"tag": tag}));
                        }
                    } else {
                        components.push(serde_json::json!({"tag": tag}));
                    }
                }
                result["components"] = serde_json::json!(components);
            }
            
            if duration_flag {
                let auto_return = br.read_u8(1)? == 1;
                br.skip_bits(6)?;
                let duration = br.read_u64(33)?;
                result["break_duration"] = serde_json::json!({
                    "auto_return": auto_return,
                    "duration_ticks": duration,
                    "duration_seconds": duration as f64 / 90000.0
                });
            }
            
            let unique_program_id = br.read_u16(16)?;
            let avail_num = br.read_u8(8)?;
            let avails_expected = br.read_u8(8)?;
            
            result["unique_program_id"] = serde_json::json!(unique_program_id);
            result["avail_num"] = serde_json::json!(avail_num);
            result["avails_expected"] = serde_json::json!(avails_expected);
            
            Ok(result)
        }
        0x06 => {
            let time_specified = br.read_u8(1)? == 1;
            if time_specified {
                br.skip_bits(6)?;
                let pts_time = br.read_u64(33)?;
                Ok(serde_json::json!({
                    "command": "time_signal",
                    "time_specified": true,
                    "pts_time": pts_time
                }))
            } else {
                br.skip_bits(7)?;
                Ok(serde_json::json!({
                    "command": "time_signal",
                    "time_specified": false,
                    "immediate": true
                }))
            }
        }
        0x07 => {
            Ok(serde_json::json!({ "command": "bandwidth_reservation" }))
        }
        _ => Ok(serde_json::json!({"info": "Command parsing not implemented"})),
    }
}

fn parse_descriptor(br: &mut BitReader) -> Result<DescriptorInfo, String> {
    let tag = br.read_u8(8)?;
    let length = br.read_u8(8)? as usize;
    
    let tag_name = match tag {
        0x00 => "avail_descriptor",
        0x01 => "DTMF_descriptor",
        0x02 => "segmentation_descriptor",
        _ => "unknown",
    };
    
    let data = if tag == 0x02 && length >= 6 {
        let start_pos = br.bitpos;
        
        let identifier = br.read_u32(32)?;
        tracing::info!("Segmentation descriptor identifier: 0x{:08X}", identifier);
        
        let seg_event_id = br.read_u32(32)?;
        let seg_cancel = br.read_u8(1)?;
        br.skip_bits(7)?;
        
        if seg_cancel == 0 && length > 5 {
            let program_seg_flag = br.read_u8(1)?;
            let seg_duration_flag = br.read_u8(1)?;
            let delivery_not_restricted = br.read_u8(1)?;

            // Delivery restriction flags (surfaced below for blackout/regionalize).
            let (mut web_delivery, mut no_regional_blackout, mut archive_allowed, mut device_restrictions) =
                (true, true, true, 3u8);
            if delivery_not_restricted == 0 {
                web_delivery = br.read_u8(1)? == 1;
                no_regional_blackout = br.read_u8(1)? == 1;
                archive_allowed = br.read_u8(1)? == 1;
                device_restrictions = br.read_u8(2)?;
            } else {
                br.skip_bits(5)?;
            }
            
            if program_seg_flag == 0 {
                let component_count = br.read_u8(8)?;
                for _ in 0..component_count {
                    let _component_tag = br.read_u8(8)?;
                    br.skip_bits(7)?;
                    let _pts_offset = br.read_u64(33)?;
                }
            }
            
            let seg_duration = if seg_duration_flag == 1 {
                Some(br.read_u64(40)?)
            } else {
                None
            };
            
            let upid_type = br.read_u8(8)?;
            let upid_length = br.read_u8(8)? as usize;
            let mut upid_bytes = Vec::new();
            for _ in 0..upid_length {
                upid_bytes.push(br.read_u8(8)?);
            }
            
            let seg_type_id = br.read_u8(8)?;
            let segment_num = br.read_u8(8)?;
            let segments_expected = br.read_u8(8)?;
            
            let upid_display = format_upid(upid_type, &upid_bytes);
            let seg_type_name = format_segmentation_type(seg_type_id);
            
            let bytes_read = (br.bitpos - start_pos) / 8;
            if bytes_read < length {
                br.skip_bits(((length - bytes_read) * 8) as u32)?;
            }
            
            serde_json::json!({
                "identifier": format!("0x{:08X}", identifier),
                "segmentation_event_id": seg_event_id,
                "segmentation_type_id": format!("0x{:02X}", seg_type_id),
                "segmentation_type_name": seg_type_name,
                "segmentation_duration_ticks": seg_duration,
                "segmentation_duration_seconds": seg_duration.map(|d| d as f64 / 90000.0),
                "delivery_not_restricted": delivery_not_restricted == 1,
                "web_delivery_allowed": web_delivery,
                "no_regional_blackout": no_regional_blackout,
                "archive_allowed": archive_allowed,
                "device_restrictions": device_restrictions,
                "upid_type": format!("0x{:02X}", upid_type),
                "upid_type_name": format_upid_type(upid_type),
                "upid_value": upid_display,
                "segment_num": segment_num,
                "segments_expected": segments_expected
            })
        } else {
            let bytes_read = (br.bitpos - start_pos) / 8;
            if bytes_read < length {
                br.skip_bits(((length - bytes_read) * 8) as u32)?;
            }
            serde_json::json!({
                "identifier": format!("0x{:08X}", identifier),
                "segmentation_event_id": seg_event_id,
                "cancelled": true
            })
        }
    } else if tag == 0x00 && length >= 4 {
        let provider_avail_id = br.read_u32(32)?;
        let bytes_read = 4;
        if bytes_read < length {
            br.skip_bits(((length - bytes_read) * 8) as u32)?;
        }
        serde_json::json!({ "provider_avail_id": provider_avail_id })
    } else if tag == 0x01 && length >= 1 {
        let preroll = br.read_u8(8)?;
        let dtmf_count = br.read_u8(3)?;
        br.skip_bits(5)?;
        let mut dtmf_chars = String::new();
        for _ in 0..dtmf_count {
            let ch = br.read_u8(8)?;
            dtmf_chars.push(ch as char);
        }
        let bytes_read = 2 + dtmf_count as usize;
        if bytes_read < length {
            br.skip_bits(((length - bytes_read) * 8) as u32)?;
        }
        serde_json::json!({ "preroll": preroll, "dtmf_chars": dtmf_chars })
    } else {
        br.skip_bits((length * 8) as u32)?;
        serde_json::json!({})
    };
    
    Ok(DescriptorInfo {
        tag,
        tag_name: tag_name.to_string(),
        length,
        data,
    })
}

fn format_upid_type(upid_type: u8) -> &'static str {
    match upid_type {
        0x00 => "Not Used",
        0x01 => "User Defined",
        0x02 => "ISCI",
        0x03 => "Ad-ID",
        0x04 => "UMID",
        0x05 => "ISAN (deprecated)",
        0x06 => "ISAN",
        0x07 => "TID",
        0x08 => "TI",
        0x09 => "ADI",
        0x0A => "EIDR",
        0x0B => "ATSC Content ID",
        0x0C => "MPU",
        0x0D => "MID",
        0x0E => "ADS Info",
        0x0F => "URI",
        0x10 => "UUID",
        0x11 => "SCR",
        _ => "Unknown"
    }
}

fn format_upid(upid_type: u8, bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "(empty)".to_string();
    }
    match upid_type {
        0x01 | 0x02 | 0x03 | 0x0C | 0x0D => {
            if bytes.iter().all(|&b| (32..=126).contains(&b)) {
                String::from_utf8_lossy(bytes).to_string()
            } else {
                format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            }
        }
        0x0F => String::from_utf8_lossy(bytes).to_string(),
        0x10 => {
            if bytes.len() == 16 {
                format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                    bytes[0], bytes[1], bytes[2], bytes[3],
                    bytes[4], bytes[5], bytes[6], bytes[7],
                    bytes[8], bytes[9], bytes[10], bytes[11],
                    bytes[12], bytes[13], bytes[14], bytes[15])
            } else {
                format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            }
        }
        0x06 => {
            if bytes.len() == 12 {
                format!("ISAN:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            } else {
                format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            }
        }
        0x0A => {
            if bytes.len() == 12 {
                format!("EIDR:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            } else {
                format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            }
        }
        _ => format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>()),
    }
}

fn format_segmentation_type(type_id: u8) -> &'static str {
    match type_id {
        0x00 => "Not Indicated",
        0x01 => "Content Identification",
        0x10 => "Program Start",
        0x11 => "Program End",
        0x12 => "Program Early Termination",
        0x13 => "Program Breakaway",
        0x14 => "Program Resumption",
        0x15 => "Program Runover Planned",
        0x16 => "Program Runover Unplanned",
        0x17 => "Program Overlap Start",
        0x18 => "Program Blackout Override",
        0x19 => "Program Start - In Progress",
        0x20 => "Chapter Start",
        0x21 => "Chapter End",
        0x22 => "Break Start",
        0x23 => "Break End",
        0x24 => "Opening Credit Start",
        0x25 => "Opening Credit End",
        0x26 => "Closing Credit Start",
        0x27 => "Closing Credit End",
        0x30 => "Provider Advertisement Start",
        0x31 => "Provider Advertisement End",
        0x32 => "Distributor Advertisement Start",
        0x33 => "Distributor Advertisement End",
        0x34 => "Provider Placement Opportunity Start",
        0x35 => "Provider Placement Opportunity End",
        0x36 => "Distributor Placement Opportunity Start",
        0x37 => "Distributor Placement Opportunity End",
        0x38 => "Provider Overlay Placement Opportunity Start",
        0x39 => "Provider Overlay Placement Opportunity End",
        0x3A => "Distributor Overlay Placement Opportunity Start",
        0x3B => "Distributor Overlay Placement Opportunity End",
        0x40 => "Unscheduled Event Start",
        0x41 => "Unscheduled Event End",
        0x42 => "Alternate Content Opportunity Start",
        0x43 => "Alternate Content Opportunity End",
        0x44 => "Provider Ad Block Start",
        0x45 => "Provider Ad Block End",
        0x46 => "Distributor Ad Block Start",
        0x47 => "Distributor Ad Block End",
        0x50 => "Network Start",
        0x51 => "Network End",
        _ => "Unknown"
    }
}

fn validate_scte35_internal(input: &str) -> Result<String, String> {
    let bytes = scte35_input_to_bytes(input)?;

    if bytes.is_empty() {
        return Err("Empty data".to_string());
    }
    if bytes.len() < 14 {
        return Err(format!("Too short: {} bytes (minimum 14)", bytes.len()));
    }

    let table_id = bytes[0];
    if table_id != 0xFC {
        return Err(format!("Invalid table_id: 0x{:02X} (expected 0xFC)", table_id));
    }

    if bytes.len() < 4 {
        return Err("Message too short for CRC".to_string());
    }

    let calculated_crc = calculate_crc32(&bytes[..bytes.len() - 4]);
    let stored_crc = u32::from_be_bytes([
        bytes[bytes.len() - 4],
        bytes[bytes.len() - 3],
        bytes[bytes.len() - 2],
        bytes[bytes.len() - 1],
    ]);

    if calculated_crc != stored_crc {
        return Err(format!(
            "CRC mismatch: calculated 0x{:08X}, stored 0x{:08X}",
            calculated_crc, stored_crc
        ));
    }

    Ok(format!(
        "Valid SCTE-35 message ({} bytes, CRC: 0x{:08X})",
        bytes.len(),
        stored_crc
    ))
}

#[allow(dead_code)]
fn parse_hex_u8(s: &str) -> Option<u8> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u8::from_str_radix(s, 16).ok()
}

fn build_advanced_internal(req: &AdvancedBuildRequest) -> Result<String, String> {
    let seg_type = req.segmentation_type_id.as_deref().and_then(parse_hex_u8);
    let upid_type = req.segmentation_upid_type.as_deref().and_then(parse_hex_u8);
    
    match req.command.as_str() {
        "time_signal" | "time_signal_immediate" => {
            Ok(scte35::build_time_signal_advanced_b64(
                seg_type,
                upid_type,
                req.segmentation_upid.as_deref(),
            ))
        }
        "splice_insert_out" => {
            let dur = req.duration_seconds.unwrap_or(60);
            Ok(scte35::build_splice_insert_out_advanced_b64(
                dur,
                seg_type,
                upid_type,
                req.segmentation_upid.as_deref(),
            ))
        }
        _ => Err(format!("Unknown command: {}", req.command)),
    }
}

/// Parse the fixed header + splice_command and return byte offsets describing the
/// descriptor loop: (descriptor_loop_length word offset, loop_start, loop_end).
/// Returns None for non-SCTE-35, encrypted, or unparseable/misaligned input.
/// Shared by every in-place rewrite below.
fn locate_descriptor_loop(bytes: &[u8]) -> Option<(usize, usize, usize)> {
    if bytes.len() < 15 || bytes[0] != 0xFC || bytes[4] & 0x80 != 0 {
        return None;
    }
    let mut br = BitReader::new(bytes);
    br.read_u8(8).ok()?; // table_id
    br.read_u8(1).ok()?; // section_syntax_indicator
    br.read_u8(1).ok()?; // private_indicator
    br.read_u8(2).ok()?; // sap_type / reserved
    br.read_u16(12).ok()?; // section_length
    br.read_u8(8).ok()?; // protocol_version
    br.read_u8(1).ok()?; // encrypted_packet
    br.read_u8(6).ok()?; // encryption_algorithm
    br.read_u64(33).ok()?; // pts_adjustment
    br.read_u8(8).ok()?; // cw_index
    br.read_u16(12).ok()?; // tier
    br.read_u16(12).ok()?; // splice_command_length
    let command_type = br.read_u8(8).ok()?;
    parse_command_info(&mut br, command_type).ok()?;
    if !br.bitpos.is_multiple_of(8) {
        return None; // unexpected misalignment after the command
    }
    let dlw_off = br.bitpos / 8;
    if dlw_off + 2 > bytes.len() {
        return None;
    }
    let word = u16::from_be_bytes([bytes[dlw_off], bytes[dlw_off + 1]]);
    let loop_start = dlw_off + 2;
    let loop_end = loop_start + (word & 0x03FF) as usize;
    if loop_end + 4 > bytes.len() {
        return None;
    }
    Some((dlw_off, loop_start, loop_end))
}

/// Recompute the trailing CRC-32 in place for a message whose total length is
/// unchanged (delivery-flag / duration edits don't move any bytes).
fn refresh_crc_in_place(bytes: &mut [u8]) {
    let n = bytes.len();
    let crc = crate::scte35::compute_crc32(&bytes[..n - 4]);
    bytes[n - 4..].copy_from_slice(&crc.to_be_bytes());
}

/// Rewrite the segmentation_upid of every segmentation_descriptor in a SCTE-35
/// message, preserving every other byte. `new_type` optionally changes the
/// segmentation_upid_type (None keeps each descriptor's existing type).
///
/// Returns the re-encoded base64 (descriptor/section lengths + CRC-32 fixed), or
/// None when the input can't be parsed, is encrypted, or carries no segmentation
/// UPID — callers should then pass the original signal through unchanged.
pub fn rewrite_upid_b64(input: &str, new_type: Option<u8>, new_value: &str) -> Option<String> {
    let bytes = scte35_input_to_bytes(input).ok()?;
    let (dlw_off, loop_start, loop_end) = locate_descriptor_loop(&bytes)?;
    let orig_word = u16::from_be_bytes([bytes[dlw_off], bytes[dlw_off + 1]]);

    // Walk the descriptor loop, rewriting the UPID of each segmentation_descriptor.
    let mut new_loop: Vec<u8> = Vec::with_capacity((loop_end - loop_start) + 8);
    let mut cursor = loop_start;
    let mut changed = false;
    while cursor + 2 <= loop_end {
        let tag = bytes[cursor];
        let len = bytes[cursor + 1] as usize;
        let body_end = cursor + 2 + len;
        if body_end > loop_end {
            return None; // malformed descriptor loop
        }
        if tag == 0x02 {
            match rewrite_seg_descriptor_body(&bytes[cursor + 2..body_end], new_type, new_value) {
                Some(nb) if nb.len() <= 255 => {
                    new_loop.push(0x02);
                    new_loop.push(nb.len() as u8);
                    new_loop.extend_from_slice(&nb);
                    changed = true;
                }
                Some(_) => return None, // descriptor would exceed the u8 length field
                None => new_loop.extend_from_slice(&bytes[cursor..body_end]), // cancel / no UPID
            }
        } else {
            new_loop.extend_from_slice(&bytes[cursor..body_end]);
        }
        cursor = body_end;
    }
    if !changed || new_loop.len() > 0x03FF {
        return None;
    }

    // Reassemble: header+command unchanged, new loop, preserved trailing stuffing,
    // fixed descriptor_loop_length / section_length, recomputed CRC-32.
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() + new_loop.len());
    out.extend_from_slice(&bytes[..dlw_off]);
    let new_word = (orig_word & 0xFC00) | (new_loop.len() as u16 & 0x03FF);
    out.extend_from_slice(&new_word.to_be_bytes());
    out.extend_from_slice(&new_loop);
    out.extend_from_slice(&bytes[loop_end..bytes.len() - 4]); // alignment stuffing, if any

    // section_length covers everything after the 3-byte prefix, including the CRC.
    let section_length = (out.len() + 4 - 3) as u16;
    out[1] = (out[1] & 0xF0) | (((section_length >> 8) & 0x0F) as u8);
    out[2] = (section_length & 0xFF) as u8;

    let crc = crate::scte35::compute_crc32(&out);
    out.extend_from_slice(&crc.to_be_bytes());

    Some(B64.encode(out))
}

/// Set the delivery-restriction flags on every (non-cancelled) segmentation
/// descriptor: clears `delivery_not_restricted_flag` and writes the 5 restriction
/// bits. Same byte width → CRC-only refresh. Used by `blackout`/`regionalize`.
/// Returns None when there is no segmentation descriptor to mark.
pub fn rewrite_delivery_flags_b64(
    input: &str,
    web_delivery_allowed: bool,
    no_regional_blackout: bool,
    archive_allowed: bool,
    device_restrictions: u8,
) -> Option<String> {
    let mut bytes = scte35_input_to_bytes(input).ok()?;
    let (_dlw, loop_start, loop_end) = locate_descriptor_loop(&bytes)?;
    let mut cursor = loop_start;
    let mut changed = false;
    while cursor + 2 <= loop_end {
        let tag = bytes[cursor];
        let len = bytes[cursor + 1] as usize;
        let body_start = cursor + 2;
        let body_end = body_start + len;
        if body_end > loop_end {
            return None;
        }
        // tag 0x02, body has the flags byte (>=10), and not a cancel descriptor.
        if tag == 0x02 && len >= 10 && bytes[body_start + 8] & 0x80 == 0 {
            // flags byte: program_seg(1) | seg_dur(1) | delivery_not_restricted(1) | 5 bits.
            let top = bytes[body_start + 9] & 0xC0; // preserve program_seg + seg_dur
            let restr = ((web_delivery_allowed as u8) << 4)
                | ((no_regional_blackout as u8) << 3)
                | ((archive_allowed as u8) << 2)
                | (device_restrictions & 0x03);
            bytes[body_start + 9] = top | restr; // delivery_not_restricted (bit5) cleared
            changed = true;
        }
        cursor = body_end;
    }
    if !changed {
        return None;
    }
    refresh_crc_in_place(&mut bytes);
    Some(B64.encode(bytes))
}

/// Set the break duration (90 kHz ticks) of the avail in place: patches the
/// splice_insert `break_duration` and/or any `segmentation_duration`. Same byte
/// width → CRC-only refresh. Used by `shorten`/`extend`/`fill`. Returns None when
/// the signal carries no duration field to modify.
pub fn rewrite_break_duration_b64(input: &str, new_ticks: u64) -> Option<String> {
    let mut bytes = scte35_input_to_bytes(input).ok()?;
    let mut changed = false;

    // splice_insert break_duration (auto_return 1b + reserved 6b + duration 33b).
    if let Some(bd) = locate_break_duration(&bytes) {
        let t = new_ticks & 0x1_FFFF_FFFF; // 33-bit
        bytes[bd] = (bytes[bd] & 0xFE) | (((t >> 32) & 0x1) as u8); // keep auto_return + reserved
        bytes[bd + 1] = (t >> 24) as u8;
        bytes[bd + 2] = (t >> 16) as u8;
        bytes[bd + 3] = (t >> 8) as u8;
        bytes[bd + 4] = t as u8;
        changed = true;
    }

    // segmentation_duration (40b) in any segmentation descriptor (time_signal avails).
    if let Some((_dlw, loop_start, loop_end)) = locate_descriptor_loop(&bytes) {
        let mut cursor = loop_start;
        while cursor + 2 <= loop_end {
            let len = bytes[cursor + 1] as usize;
            let body_start = cursor + 2;
            let body_end = body_start + len;
            if body_end > loop_end {
                break;
            }
            if bytes[cursor] == 0x02 {
                if let Some(off) = seg_duration_offset(&bytes[body_start..body_end]) {
                    let abs = body_start + off;
                    let t = new_ticks & 0xFF_FFFF_FFFF; // 40-bit
                    bytes[abs] = (t >> 32) as u8;
                    bytes[abs + 1] = (t >> 24) as u8;
                    bytes[abs + 2] = (t >> 16) as u8;
                    bytes[abs + 3] = (t >> 8) as u8;
                    bytes[abs + 4] = t as u8;
                    changed = true;
                }
            }
            cursor = body_end;
        }
    }

    if !changed {
        return None;
    }
    refresh_crc_in_place(&mut bytes);
    Some(B64.encode(bytes))
}

/// Additive variant of `rewrite_break_duration_b64`: ADD `delta_ticks` (90 kHz,
/// may be negative) to the existing splice_insert `break_duration` and to any
/// `segmentation_duration`, flooring the result at 0. Used by `extend` so the
/// label "extend by Ns" lengthens the incoming break rather than overwriting it.
/// Returns None when the signal carries no duration field to modify.
pub fn adjust_break_duration_b64(input: &str, delta_ticks: i64) -> Option<String> {
    let mut bytes = scte35_input_to_bytes(input).ok()?;
    let mut changed = false;

    // splice_insert break_duration (33-bit), preserving auto_return + reserved.
    if let Some(bd) = locate_break_duration(&bytes) {
        let cur = (((bytes[bd] & 0x01) as u64) << 32)
            | ((bytes[bd + 1] as u64) << 24)
            | ((bytes[bd + 2] as u64) << 16)
            | ((bytes[bd + 3] as u64) << 8)
            | (bytes[bd + 4] as u64);
        let t = (cur as i64 + delta_ticks).max(0) as u64 & 0x1_FFFF_FFFF;
        bytes[bd] = (bytes[bd] & 0xFE) | (((t >> 32) & 0x1) as u8);
        bytes[bd + 1] = (t >> 24) as u8;
        bytes[bd + 2] = (t >> 16) as u8;
        bytes[bd + 3] = (t >> 8) as u8;
        bytes[bd + 4] = t as u8;
        changed = true;
    }

    // segmentation_duration (40-bit) in any segmentation descriptor.
    if let Some((_dlw, loop_start, loop_end)) = locate_descriptor_loop(&bytes) {
        let mut cursor = loop_start;
        while cursor + 2 <= loop_end {
            let len = bytes[cursor + 1] as usize;
            let body_start = cursor + 2;
            let body_end = body_start + len;
            if body_end > loop_end {
                break;
            }
            if bytes[cursor] == 0x02 {
                if let Some(off) = seg_duration_offset(&bytes[body_start..body_end]) {
                    let abs = body_start + off;
                    let cur = ((bytes[abs] as u64) << 32)
                        | ((bytes[abs + 1] as u64) << 24)
                        | ((bytes[abs + 2] as u64) << 16)
                        | ((bytes[abs + 3] as u64) << 8)
                        | (bytes[abs + 4] as u64);
                    let t = (cur as i64 + delta_ticks).max(0) as u64 & 0xFF_FFFF_FFFF;
                    bytes[abs] = (t >> 32) as u8;
                    bytes[abs + 1] = (t >> 24) as u8;
                    bytes[abs + 2] = (t >> 16) as u8;
                    bytes[abs + 3] = (t >> 8) as u8;
                    bytes[abs + 4] = t as u8;
                    changed = true;
                }
            }
            cursor = body_end;
        }
    }

    if !changed {
        return None;
    }
    refresh_crc_in_place(&mut bytes);
    Some(B64.encode(bytes))
}

/// Byte offset (from the start of `bytes`) of the splice_insert break_duration
/// field, navigating the flag-dependent layout. None unless this is a
/// splice_insert with `duration_flag` set. All preceding fields are whole bytes,
/// so the result is byte-aligned.
fn locate_break_duration(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 15 || bytes[0] != 0xFC || bytes[4] & 0x80 != 0 {
        return None;
    }
    let mut br = BitReader::new(bytes);
    br.read_u8(8).ok()?;
    br.read_u8(1).ok()?;
    br.read_u8(1).ok()?;
    br.read_u8(2).ok()?;
    br.read_u16(12).ok()?;
    br.read_u8(8).ok()?;
    br.read_u8(1).ok()?;
    br.read_u8(6).ok()?;
    br.read_u64(33).ok()?;
    br.read_u8(8).ok()?;
    br.read_u16(12).ok()?;
    br.read_u16(12).ok()?;
    if br.read_u8(8).ok()? != 0x05 {
        return None; // not splice_insert
    }
    br.read_u32(32).ok()?; // splice_event_id
    let cancel = br.read_u8(1).ok()? == 1;
    br.skip_bits(7).ok()?;
    if cancel {
        return None;
    }
    let _oon = br.read_u8(1).ok()?;
    let program_splice = br.read_u8(1).ok()? == 1;
    let duration_flag = br.read_u8(1).ok()? == 1;
    let splice_immediate = br.read_u8(1).ok()? == 1;
    br.skip_bits(4).ok()?;
    if !duration_flag {
        return None;
    }
    if program_splice && !splice_immediate {
        skip_splice_time(&mut br)?;
    } else if !program_splice {
        let cc = br.read_u8(8).ok()?;
        for _ in 0..cc {
            br.read_u8(8).ok()?; // component_tag
            if !splice_immediate {
                skip_splice_time(&mut br)?;
            }
        }
    }
    if !br.bitpos.is_multiple_of(8) {
        return None;
    }
    Some(br.bitpos / 8)
}

fn skip_splice_time(br: &mut BitReader) -> Option<()> {
    if br.read_u8(1).ok()? == 1 {
        br.skip_bits(6).ok()?; // reserved
        br.read_u64(33).ok()?; // pts_time
    } else {
        br.skip_bits(7).ok()?; // reserved
    }
    Some(())
}

/// Offset within a segmentation_descriptor body of the 40-bit segmentation_duration,
/// if `segmentation_duration_flag` is set. None otherwise.
fn seg_duration_offset(body: &[u8]) -> Option<usize> {
    if body.len() < 10 || body[8] & 0x80 != 0 {
        return None;
    }
    let flags = body[9];
    let program_segmentation_flag = flags & 0x80 != 0;
    let segmentation_duration_flag = flags & 0x40 != 0;
    if !segmentation_duration_flag {
        return None;
    }
    let mut idx = 10usize;
    if !program_segmentation_flag {
        let cc = *body.get(idx)? as usize;
        idx += 1;
        idx += cc.checked_mul(6)?;
    }
    if idx + 5 > body.len() {
        return None;
    }
    Some(idx)
}

/// Rewrite the segmentation_upid (type + value) inside one segmentation_descriptor
/// body (the bytes after tag+length). Returns None for a cancelled descriptor or
/// when the layout can't be walked.
fn rewrite_seg_descriptor_body(body: &[u8], new_type: Option<u8>, new_value: &str) -> Option<Vec<u8>> {
    // identifier(4) + segmentation_event_id(4) + cancel(1)/reserved(7)
    if body.len() < 9 {
        return None;
    }
    if body[8] & 0x80 != 0 {
        return None; // segmentation_event_cancel_indicator set: no UPID present
    }
    let flags = *body.get(9)?;
    let program_segmentation_flag = flags & 0x80 != 0;
    let segmentation_duration_flag = flags & 0x40 != 0;
    let mut idx = 10usize;
    if !program_segmentation_flag {
        // component_count(8), then component_count * (tag(8)+reserved(7)+pts_offset(33)) = 6 bytes each
        let cc = *body.get(idx)? as usize;
        idx += 1;
        idx += cc.checked_mul(6)?;
    }
    if segmentation_duration_flag {
        idx += 5; // segmentation_duration (40 bits)
    }
    // segmentation_upid_type(8), segmentation_upid_length(8), segmentation_upid[len]
    let upid_type = *body.get(idx)?;
    let upid_len = *body.get(idx + 1)? as usize;
    let upid_end = (idx + 2).checked_add(upid_len)?;
    if upid_end > body.len() {
        return None;
    }
    let tail = &body[upid_end..]; // segmentation_type_id, segment_num, segments_expected, [sub_*]

    let new_type_val = new_type.unwrap_or(upid_type);
    let new_upid = crate::scte35::encode_upid(new_type_val, new_value);
    if new_upid.len() > 255 {
        return None;
    }

    let mut nb: Vec<u8> = Vec::with_capacity(idx + 2 + new_upid.len() + tail.len());
    nb.extend_from_slice(&body[..idx]);
    nb.push(new_type_val);
    nb.push(new_upid.len() as u8);
    nb.extend_from_slice(&new_upid);
    nb.extend_from_slice(tail);
    Some(nb)
}

// ============================================================================
// BIT READER HELPER
// ============================================================================

struct BitReader<'a> {
    data: &'a [u8],
    bitpos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bitpos: 0 }
    }

    fn read_u8(&mut self, nbits: u32) -> Result<u8, String> {
        Ok(self.read_bits(nbits)? as u8)
    }

    fn read_u16(&mut self, nbits: u32) -> Result<u16, String> {
        Ok(self.read_bits(nbits)? as u16)
    }

    fn read_u32(&mut self, nbits: u32) -> Result<u32, String> {
        Ok(self.read_bits(nbits)? as u32)
    }

    fn read_u64(&mut self, nbits: u32) -> Result<u64, String> {
        self.read_bits(nbits)
    }

    fn read_bits(&mut self, nbits: u32) -> Result<u64, String> {
        let mut v = 0u64;
        for _ in 0..nbits {
            let idx = self.bitpos / 8;
            if idx >= self.data.len() {
                return Err("out of bounds".into());
            }
            let byte = self.data[idx];
            let bit = 7 - (self.bitpos % 8);
            v = (v << 1) | (((byte >> bit) & 1) as u64);
            self.bitpos += 1;
        }
        Ok(v)
    }

    fn skip_bits(&mut self, nbits: u32) -> Result<(), String> {
        self.bitpos = self
            .bitpos
            .checked_add(nbits as usize)
            .ok_or_else(|| "overflow".to_string())?;
        if self.bitpos / 8 > self.data.len() {
            return Err("out of bounds".into());
        }
        Ok(())
    }
}

// ============================================================================
// CRC-32 CALCULATION
// ============================================================================

fn calculate_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &byte in data {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ 0x04C11DB7;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}
#[cfg(test)]
mod input_format_tests {
    use super::*;

    // A real splice_insert sample (first byte 0xFC).
    const SAMPLE: &str = "/DAlAAAAAAAAAP/wFAUAAAABf+/+ANSrgP4AKTLgAAEBAQAArQrwxg==";

    #[test]
    fn accepts_base64_hex_binary_equivalently() {
        let bytes = scte35_input_to_bytes(SAMPLE).unwrap();
        assert_eq!(bytes[0], 0xFC);

        let hex_lower: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        let hex_spaced: String = bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
        let bin: String = bytes.iter().map(|b| format!("{:08b}", b)).collect();

        assert_eq!(scte35_input_to_bytes(&hex_lower).unwrap(), bytes, "plain hex");
        assert_eq!(scte35_input_to_bytes(&format!("0x{hex_lower}")).unwrap(), bytes, "0x hex");
        assert_eq!(scte35_input_to_bytes(&hex_spaced).unwrap(), bytes, "spaced hex");
        assert_eq!(scte35_input_to_bytes(&bin).unwrap(), bytes, "binary");
        assert_eq!(scte35_input_to_bytes(&format!("0b{bin}")).unwrap(), bytes, "0b binary");
    }

    #[test]
    fn full_decode_from_hex_matches_base64() {
        let from_b64 = decode_scte35_internal(SAMPLE).unwrap();
        let hex: String = B64.decode(SAMPLE).unwrap().iter().map(|b| format!("{:02X}", b)).collect();
        let from_hex = decode_scte35_internal(&hex).unwrap();
        assert_eq!(from_b64.command_type, from_hex.command_type);
    }

    #[test]
    fn rejects_garbage() {
        assert!(scte35_input_to_bytes("not valid !!!").is_err());
    }

    // ---- UPID rewrite (in-place) ----

    fn seg_descriptor(decoded: &DecodedScte35) -> serde_json::Value {
        decoded
            .descriptors
            .iter()
            .find(|d| d.tag == 0x02)
            .map(|d| d.data.clone())
            .expect("a segmentation_descriptor")
    }

    #[test]
    fn rewrite_changes_value_and_type_keeps_everything_else() {
        // Build a time_signal carrying seg type 0x34 and an ASCII (MID) UPID.
        let orig = crate::scte35::build_time_signal_advanced_b64(Some(0x34), Some(0x0C), Some("ORIGINAL"));
        let before = decode_scte35_internal(&orig).unwrap();

        // Rewrite to a URI-type (0x0F) UPID with a new value.
        let out = rewrite_upid_b64(&orig, Some(0x0F), "https://new.example/ad").expect("rewrite");
        let after = decode_scte35_internal(&out).expect("re-decodes (valid CRC + lengths)");

        let sd = seg_descriptor(&after);
        assert_eq!(sd["upid_type"], "0x0F");
        assert!(sd["upid_value"].as_str().unwrap().contains("new.example/ad"), "new UPID present");
        // Everything else preserved:
        assert_eq!(after.command_type, before.command_type, "command unchanged");
        assert_eq!(sd["segmentation_type_id"], "0x34", "seg type unchanged");
    }

    #[test]
    fn rewrite_keeps_existing_type_when_none() {
        let orig = crate::scte35::build_splice_insert_out_advanced_b64(30, Some(0x10), Some(0x0C), Some("OLD"));
        let out = rewrite_upid_b64(&orig, None, "NEWVALUE").expect("rewrite");
        let after = decode_scte35_internal(&out).expect("re-decodes");
        let sd = seg_descriptor(&after);
        assert_eq!(sd["upid_type"], "0x0C", "type kept");
        assert!(sd["upid_value"].as_str().unwrap().contains("NEWVALUE"));
    }

    #[test]
    fn rewrite_returns_none_without_segmentation_descriptor() {
        // A plain immediate time_signal has no segmentation_descriptor -> nothing to change.
        let plain = crate::scte35::build_time_signal_immediate_b64();
        assert!(rewrite_upid_b64(&plain, Some(0x0E), "x").is_none());
    }

    // ---- break-duration rewrite (shorten/extend/fill) ----

    #[test]
    fn rewrite_break_duration_changes_avail_length() {
        let orig = crate::scte35::build_splice_insert_out_b64(30);
        let before = decode_scte35_internal(&orig).unwrap();
        let out = rewrite_break_duration_b64(&orig, 15 * 90000).expect("rewrite");
        let after = decode_scte35_internal(&out).expect("re-decodes (valid CRC + lengths)");

        // splice_insert break_duration updated to 15s
        let bd = after.command_info["break_duration"]["duration_seconds"].as_f64().unwrap();
        assert!((bd - 15.0).abs() < 0.001, "break_duration={bd}");
        assert_eq!(after.command_type, before.command_type, "command unchanged");
        // the segmentation_duration in the descriptor is patched too
        let segdur = seg_descriptor(&after)["segmentation_duration_seconds"].as_f64().unwrap();
        assert!((segdur - 15.0).abs() < 0.001, "seg_duration={segdur}");
    }

    #[test]
    fn rewrite_break_duration_none_without_duration_field() {
        let plain = crate::scte35::build_time_signal_immediate_b64();
        assert!(rewrite_break_duration_b64(&plain, 90000).is_none());
    }

    #[test]
    fn adjust_break_duration_adds_delta() {
        // extend "by 30s": a 30s avail becomes 60s (break + segmentation durations).
        let orig = crate::scte35::build_splice_insert_out_b64(30);
        let out = adjust_break_duration_b64(&orig, 30 * 90000).expect("adjust");
        let after = decode_scte35_internal(&out).expect("re-decodes (valid CRC + lengths)");
        let bd = after.command_info["break_duration"]["duration_seconds"].as_f64().unwrap();
        assert!((bd - 60.0).abs() < 0.001, "break_duration={bd}");
        let segdur = seg_descriptor(&after)["segmentation_duration_seconds"].as_f64().unwrap();
        assert!((segdur - 60.0).abs() < 0.001, "seg_duration={segdur}");
    }

    #[test]
    fn adjust_break_duration_floors_at_zero() {
        // Subtracting more than the current duration floors at 0, never negative.
        let orig = crate::scte35::build_splice_insert_out_b64(10);
        let out = adjust_break_duration_b64(&orig, -30 * 90000).expect("adjust");
        let after = decode_scte35_internal(&out).expect("re-decodes");
        let bd = after.command_info["break_duration"]["duration_seconds"].as_f64().unwrap();
        assert!(bd.abs() < 0.001, "break_duration={bd}");
    }

    #[test]
    fn adjust_break_duration_none_without_duration_field() {
        let plain = crate::scte35::build_time_signal_immediate_b64();
        assert!(adjust_break_duration_b64(&plain, 90000).is_none());
    }

    // ---- delivery-flag rewrite (blackout/regionalize) ----

    #[test]
    fn rewrite_delivery_flags_marks_blackout() {
        let orig = crate::scte35::build_time_signal_advanced_b64(Some(0x34), Some(0x0C), Some("X"));
        assert_eq!(seg_descriptor(&decode_scte35_internal(&orig).unwrap())["delivery_not_restricted"], true);

        let out = rewrite_delivery_flags_b64(&orig, false, false, true, 0).expect("rewrite");
        let sd = seg_descriptor(&decode_scte35_internal(&out).expect("re-decodes"));
        assert_eq!(sd["delivery_not_restricted"], false);
        assert_eq!(sd["web_delivery_allowed"], false);
        assert_eq!(sd["no_regional_blackout"], false);
        assert_eq!(sd["archive_allowed"], true);
    }
}

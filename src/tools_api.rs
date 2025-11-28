// src/tools_api.rs
// Version: 4.1.0
// Created: 2024-11-17
// Updated: 2024-11-26
// 
// Enhanced SCTE-35 Tools API - Decoder, Validator, Test Sender, Unified Builder
//
// Changelog:
// v4.1.0 (2024-11-26): Builder consolidation - merged basic and advanced builders
//   - Consolidated BuildRequest to include all segmentation parameters
//   - Merged build_scte35 and build_advanced_scte35 into single endpoint
//   - Removed AdvancedBuildRequest struct (functionality merged into BuildRequest)
//   - Removed build_advanced_scte35 handler (functionality in build_scte35)
//   - Renamed build_advanced_internal to build_scte35_internal
//   - All segmentation parameters now optional in BuildRequest for backward compatibility
//   - Single /api/tools/scte35/build endpoint now handles both basic and advanced signals
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
//   - CRITICAL FIX: After parsing command info, now skips remaining command bytes
//   - This fixes "No descriptors found" bug when splice_command_length > parsed bytes
//   - splice_insert now fully parsed (flags, duration, program info)
//   - Proper handling of splice_command_length=0xFFF (unspecified length)
//   - NOTE: This introduced a bug that was fixed in v4.0.6
// v4.0.3 (2024-11-20): Fixed segmentation descriptor parsing
//   - CRITICAL FIX: Now properly reads 4-byte "CUEI" identifier before segmentation_event_id
//   - This was causing all segmentation descriptors to be parsed incorrectly
//   - Decoder now correctly displays seg type, UPID type, and UPID value
// v4.0.2 (2024-11-19): Enhanced logging for descriptor parsing debugging
//   - Added INFO level logs to track bit positions during decode
//   - Better error handling for descriptor parsing failures
// v4.0.1 (2024-11-19): Enhanced decoder for segmentation descriptors
//   - Decoder now extracts and displays segmentation_type_id with name
//   - Shows UPID type and formatted UPID value
//   - Displays segmentation duration in both ticks and seconds
//   - Smart UPID formatting (ASCII, UUID, hex) based on type
// v4.0.0 (2024-11-19): Full segmentation descriptor support
//   - Advanced builder now supports custom segmentation types and UPIDs
//   - Added hex parsing for segmentation_type_id and upid_type
//   - Integrated with new scte35::build_*_advanced_b64() functions
// v3.1.2 (2024-11-17): Quick Test now processes real ESAM signals
//   - test_send now builds proper ESAM XML and processes through pipeline
//   - Runs signals through channel rules for matching
//   - Creates real events in database visible in Event Monitora
//   - Marks test events with "QuickTest:{username}" as source IP
// v3.1.1 (2024-11-17): Fixed JWT authentication integration
//   - Added Extension(claims) parameter to all endpoints for JWT auth
//   - Added State(st) parameter for AppState access
//   - All endpoints now properly integrate with require_jwt_auth middleware
// v3.1.0 (2024-11-17): Initial release
//   - Added SCTE-35 decoder with full message parsing
//   - Added SCTE-35 validator with CRC-32 checking
//   - Added quick test sender for channel testing
//   - Added advanced builder foundation for segmentation descriptors
//   - All endpoints require JWT authentication

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
    // Segmentation descriptor parameters (optional - for advanced signals)
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

/// POST /api/tools/scte35/build - Basic SCTE-35 builder
/// POST /api/tools/scte35/build - Unified SCTE-35 builder with optional segmentation descriptors
/// Supports both basic signals (time_signal, splice_insert) and advanced signals with segmentation
pub async fn build_scte35(
    State(_st): State<std::sync::Arc<AppState>>,
    Extension(_claims): Extension<jwt_auth::Claims>,
    Json(req): Json<BuildRequest>,
) -> Response {
    match build_scte35_internal(&req) {
        Ok(b64) => Json(BuildResponse { base64: b64 }).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
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
        let resp = build_notification(&test_signal_id, &utc_point, &r.action, &params);
        (r.action.clone(), resp)
    } else {
        let resp = build_notification(&test_signal_id, &utc_point, "noop", &serde_json::json!({}));
        ("noop".to_string(), resp)
    };
    
    let duration = start.elapsed();
    
    // Log the event with special marker for Quick Test
    let client_info = ClientInfo {
        source_ip: Some(format!("QuickTest:{}", claims.username)),
        user_agent: Some("POIS Quick Test Tool".to_string()),
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

fn decode_scte35_internal(b64: &str) -> Result<DecodedScte35, String> {
    let bytes = B64
        .decode(b64)
        .map_err(|e| format!("Invalid Base64: {}", e))?;

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
    
    // Record position before parsing command (after command_type byte)
    let command_start_pos = br.bitpos;
    
    // Parse command-specific data
    let command_info = parse_command_info(&mut br, command_type)?;
    
    tracing::info!("After command parse, bitpos: {}", br.bitpos);
    
    // v4.0.6 FIX: REMOVED the v4.0.4 skip logic - it was causing wrong bitpos!
    // The command parsers already consume the correct number of bytes.
    // descriptor_loop_length immediately follows the command data.
    
    tracing::info!("Before reading descriptor_loop_length, bitpos: {}, total message bits: {}", 
                   br.bitpos, br.data.len() * 8);
    
    // v4.0.5 FIX: Parse descriptors with proper 10-bit length field
    // The descriptor_loop_length field is 10 bits, with 6 reserved bits before it
    let descriptor_word = br.read_u16(16)?;
    let descriptor_loop_length = (descriptor_word & 0x03FF) as usize;  // Mask to get lower 10 bits
    
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
                    // Skip remaining descriptor data
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
            // splice_null - no data
            Ok(serde_json::json!({
                "command": "splice_null"
            }))
        }
        0x05 => {
            // splice_insert - fully parse for display
            let event_id = br.read_u32(32)?;
            let event_cancel = br.read_u8(1)? == 1;
            br.skip_bits(7)?; // reserved
            
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
            br.skip_bits(4)?; // reserved
            
            let mut result = serde_json::json!({
                "command": "splice_insert",
                "splice_event_id": event_id,
                "out_of_network_indicator": out_of_network,
                "program_splice_flag": program_splice_flag,
                "duration_flag": duration_flag,
                "splice_immediate_flag": splice_immediate_flag
            });
            
            // Parse splice_time if program_splice and not immediate
            if program_splice_flag && !splice_immediate_flag {
                let time_specified = br.read_u8(1)? == 1;
                if time_specified {
                    br.skip_bits(6)?; // reserved
                    let pts_time = br.read_u64(33)?;
                    result["pts_time"] = serde_json::json!(pts_time);
                } else {
                    br.skip_bits(7)?; // reserved
                }
            }
            
            // Parse component mode if not program splice
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
            
            // Parse break_duration if duration_flag
            if duration_flag {
                let auto_return = br.read_u8(1)? == 1;
                br.skip_bits(6)?; // reserved
                let duration = br.read_u64(33)?;
                result["break_duration"] = serde_json::json!({
                    "auto_return": auto_return,
                    "duration_ticks": duration,
                    "duration_seconds": duration as f64 / 90000.0
                });
            }
            
            // unique_program_id, avail_num, avails_expected
            let unique_program_id = br.read_u16(16)?;
            let avail_num = br.read_u8(8)?;
            let avails_expected = br.read_u8(8)?;
            
            result["unique_program_id"] = serde_json::json!(unique_program_id);
            result["avail_num"] = serde_json::json!(avail_num);
            result["avails_expected"] = serde_json::json!(avails_expected);
            
            Ok(result)
        }
        0x06 => {
            // time_signal - parse splice_time()
            let time_specified = br.read_u8(1)? == 1;
            if time_specified {
                br.skip_bits(6)?; // reserved
                let pts_time = br.read_u64(33)?;
                Ok(serde_json::json!({
                    "command": "time_signal",
                    "time_specified": true,
                    "pts_time": pts_time
                }))
            } else {
                br.skip_bits(7)?; // reserved
                Ok(serde_json::json!({
                    "command": "time_signal",
                    "time_specified": false,
                    "immediate": true
                }))
            }
        }
        0x07 => {
            // bandwidth_reservation - no additional data
            Ok(serde_json::json!({
                "command": "bandwidth_reservation"
            }))
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
    
    // For segmentation descriptor, parse detailed fields
    let data = if tag == 0x02 && length >= 6 {
        let start_pos = br.bitpos;
        
        // Read identifier (should be "CUEI" = 0x43554549)
        let identifier = br.read_u32(32)?;
        tracing::info!("Segmentation descriptor identifier: 0x{:08X}", identifier);
        
        let seg_event_id = br.read_u32(32)?;
        let seg_cancel = br.read_u8(1)?;
        br.skip_bits(7)?; // reserved
        
        if seg_cancel == 0 && length > 5 {
            // Parse flags
            let program_seg_flag = br.read_u8(1)?;
            let seg_duration_flag = br.read_u8(1)?;
            let delivery_not_restricted = br.read_u8(1)?;
            
            if delivery_not_restricted == 0 {
                let _web_delivery = br.read_u8(1)?;
                let _no_regional_blackout = br.read_u8(1)?;
                let _archive_allowed = br.read_u8(1)?;
                let _device_restrictions = br.read_u8(2)?;
            } else {
                br.skip_bits(5)?; // reserved
            }
            
            // Parse components if not program segmentation
            if program_seg_flag == 0 {
                let component_count = br.read_u8(8)?;
                for _ in 0..component_count {
                    let _component_tag = br.read_u8(8)?;
                    br.skip_bits(7)?; // reserved
                    let _pts_offset = br.read_u64(33)?;
                }
            }
            
            // Segmentation duration
            let seg_duration = if seg_duration_flag == 1 {
                Some(br.read_u64(40)?)
            } else {
                None
            };
            
            // UPID
            let upid_type = br.read_u8(8)?;
            let upid_length = br.read_u8(8)? as usize;
            let mut upid_bytes = Vec::new();
            for _ in 0..upid_length {
                upid_bytes.push(br.read_u8(8)?);
            }
            
            // Segmentation type ID
            let seg_type_id = br.read_u8(8)?;
            
            // segment_num and segments_expected
            let segment_num = br.read_u8(8)?;
            let segments_expected = br.read_u8(8)?;
            
            // Format UPID based on type
            let upid_display = format_upid(upid_type, &upid_bytes);
            let seg_type_name = format_segmentation_type(seg_type_id);
            
            // Skip any remaining bytes (sub-segments for certain types)
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
                "upid_type": format!("0x{:02X}", upid_type),
                "upid_type_name": format_upid_type(upid_type),
                "upid_value": upid_display,
                "segment_num": segment_num,
                "segments_expected": segments_expected
            })
        } else {
            // Cancelled segmentation
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
        // avail_descriptor
        let provider_avail_id = br.read_u32(32)?;
        let bytes_read = 4;
        if bytes_read < length {
            br.skip_bits(((length - bytes_read) * 8) as u32)?;
        }
        serde_json::json!({
            "provider_avail_id": provider_avail_id
        })
    } else if tag == 0x01 && length >= 1 {
        // DTMF_descriptor
        let preroll = br.read_u8(8)?;
        let dtmf_count = br.read_u8(3)?;
        br.skip_bits(5)?; // reserved
        let mut dtmf_chars = String::new();
        for _ in 0..dtmf_count {
            let ch = br.read_u8(8)?;
            dtmf_chars.push(ch as char);
        }
        let bytes_read = 2 + dtmf_count as usize;
        if bytes_read < length {
            br.skip_bits(((length - bytes_read) * 8) as u32)?;
        }
        serde_json::json!({
            "preroll": preroll,
            "dtmf_chars": dtmf_chars
        })
    } else {
        // Skip unknown descriptor data
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
            // ASCII types: User Defined, ISCI, Ad-ID, MPU, MID
            if bytes.iter().all(|&b| (32..=126).contains(&b)) {
                String::from_utf8_lossy(bytes).to_string()
            } else {
                format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            }
        }
        0x0F => {
            // URI - always ASCII
            String::from_utf8_lossy(bytes).to_string()
        }
        0x10 => {
            // UUID - 16 bytes formatted
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
            // ISAN - 12 bytes
            if bytes.len() == 12 {
                format!("ISAN:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            } else {
                format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            }
        }
        0x0A => {
            // EIDR - typically 12 bytes
            if bytes.len() == 12 {
                // Format as 10.5240/XXXX-XXXX-XXXX-XXXX-XXXX-C
                format!("EIDR:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            } else {
                format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            }
        }
        _ => {
            // Hex for others
            format!("hex:{}", bytes.iter().map(|b| format!("{:02X}", b)).collect::<String>())
        }
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

fn validate_scte35_internal(b64: &str) -> Result<String, String> {
    let bytes = B64
        .decode(b64)
        .map_err(|e| format!("Invalid Base64: {}", e))?;

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

    // Check CRC-32 (last 4 bytes)
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

fn parse_hex_u8(s: &str) -> Option<u8> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u8::from_str_radix(s, 16).ok()
}

/// Internal builder function - handles both basic and advanced SCTE-35 signal generation
/// If segmentation parameters are provided, generates advanced signals with descriptors
/// If not provided, generates basic signals for backward compatibility
fn build_scte35_internal(req: &BuildRequest) -> Result<String, String> {
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
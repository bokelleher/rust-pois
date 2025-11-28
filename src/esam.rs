// src/esam.rs
// Version: 2.2.0
// Updated: 2024-11-24
//
// Changelog:
// v2.2.1 (2024-11-24): Fixed parse_splice_insert_pts missing final fields
// v2.2.0 (2024-11-24): Applied critical fixes from tools_api.rs v4.0.6
//   - CRITICAL FIX: Removed v2.1.0 skip logic that was causing wrong bitpos (v4.0.6 fix)
//   - CRITICAL FIX: Masked descriptor_loop_length to 10 bits (6 reserved bits) (v4.0.5 fix)
//   - Command parsers already consume correct bytes; descriptor_loop_length immediately follows
//   - Now Event Monitor will show segmentation descriptors correctly!
// v2.1.0 (2024-11-24): Fixed SCTE-35 decoder command byte skipping
//   - CRITICAL FIX: time_signal now properly tracks bits read before skipping
//   - CRITICAL FIX: splice_insert skip logic now correctly positioned
//   - CRITICAL FIX: delivery_not_restricted always skips 5 bits (was missing when true)
//   - Added handling for splice_command_length=0xFFF (unspecified)
//   - NOTE: v2.1.0 skip logic introduced a bug that was fixed in v2.2.0
//   - NOTE: v2.2.0 fixed descriptor_loop_length masking but parse_splice_insert_pts
//           was still missing final fields, causing bitpos to be 72 bits short. Fixed in v2.2.1
// v2.0.0: Enhanced UPID/Type ID decoding with comprehensive type support

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use quick_xml::{events::Event, Reader};
use serde_json::json;
use tracing::{debug, warn, error, info};

/// Extract minimal facts from an ESAM SignalProcessingEvent XML with enhanced UPID/Type ID decoding
pub fn extract_facts(esam_xml: &str) -> Result<serde_json::Value, String> {
    let mut reader = Reader::from_str(esam_xml);
    reader.trim_text(true);
    let mut buf = Vec::new();

    let mut acquisition_signal_id = String::new();
    let mut utc_point: Option<String> = None;
    let mut scte35_b64: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if local.ends_with("AcquiredSignal") {
                    for a in e.attributes().flatten() {
                        let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                        let v = a.unescape_value().map_err(|e| e.to_string())?.to_string();
                        if k.ends_with("acquisitionSignalID") {
                            acquisition_signal_id = v;
                        }
                    }
                }
                if local.ends_with("UTCPoint") {
                    for a in e.attributes().flatten() {
                        let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                        let v = a.unescape_value().map_err(|e| e.to_string())?.to_string();
                        if k.ends_with("utcPoint") {
                            utc_point = Some(v);
                        }
                    }
                }
                if local.ends_with("BinaryData") {
                    debug!("extract_facts: Found BinaryData element");
                    if let Ok(Event::Text(t)) = reader.read_event_into(&mut buf) {
                        let text = t.unescape().map_err(|e| e.to_string())?.to_string();
                        debug!("extract_facts: Read BinaryData text (length={})", text.len());
                        scte35_b64 = Some(text);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
        buf.clear();
    }

    if acquisition_signal_id.is_empty() {
        return Err("missing acquisitionSignalID".into());
    }

    // Decode SCTE-35 details if present
    let mut scte35_cmd = None;
    let mut seg_type_id_hex = None;
    let mut seg_type_name = None;
    let mut seg_upid_repr = None;
    let mut upid_type_name = None;
    let mut pts_time = None;
    
    if let Some(ref b64) = scte35_b64 {
        debug!("extract_facts: Found SCTE-35 base64 (length={}), calling decode_scte35_details", b64.len());
        match decode_scte35_details(b64) {
            Ok(info) => {
                scte35_cmd = info.command.clone();
                if let Some(ref cmd) = scte35_cmd {
                    debug!("extract_facts: ✅ SCTE-35 decoded successfully, command={}", cmd);
                } else {
                    warn!("extract_facts: SCTE-35 decoded but command is None (will become 'unknown')");
                }
                
                if let Some(type_id) = info.segmentation_type_id {
                    seg_type_id_hex = Some(format!("0x{type_id:02X}"));
                    seg_type_name = Some(decode_segmentation_type_name(type_id));
                    debug!("extract_facts: Extracted segmentation_type_id={}", seg_type_id_hex.as_ref().unwrap());
                }
                
                if let Some((upid_type, upid_bytes)) = info.segmentation_upid_with_type {
                    upid_type_name = Some(decode_upid_type_name(upid_type));
                    seg_upid_repr = Some(decode_upid_data(upid_type, &upid_bytes));
                    debug!("extract_facts: Extracted UPID type={}, repr={}", 
                           upid_type_name.as_ref().unwrap(), seg_upid_repr.as_ref().unwrap());
                }
                
                pts_time = info.pts_time;
                if let Some(pts) = pts_time {
                    debug!("extract_facts: Extracted PTS time={}", pts);
                }
            }
            Err(e) => {
                error!("extract_facts: ❌ SCTE-35 decoding FAILED: {}", e);
                error!("extract_facts: Failed base64 was: {}", b64);
                // scte35_cmd stays None, will become "unknown"
            }
        }
    }
    let scte35_cmd_str = scte35_cmd.unwrap_or_else(|| {
        if scte35_b64.is_some() {
            warn!("extract_facts: SCTE-35 present but command is None/unknown");
        }
        "unknown".into()
    });
    
    debug!("extract_facts: Final SCTE-35 command string: '{}'", scte35_cmd_str);

    let mut out = json!({
        "acquisitionSignalID": acquisition_signal_id,
        "utcPoint": utc_point.unwrap_or_else(|| "1970-01-01T00:00:00Z".into()),
        "scte35.command": scte35_cmd_str,
    });
    
    if let Some(b64) = scte35_b64 { out["scte35_b64"] = json!(b64); }
    if let Some(t) = seg_type_id_hex { out["scte35.segmentation_type_id"] = json!(t); }
    if let Some(name) = seg_type_name { out["scte35.segmentation_type_name"] = json!(name); }
    if let Some(u) = seg_upid_repr { out["scte35.segmentation_upid"] = json!(u); }
    if let Some(upid_name) = upid_type_name { out["scte35.upid_type_name"] = json!(upid_name); }
    if let Some(pts) = pts_time { out["scte35.pts_time"] = json!(pts); }
    
    Ok(out)
}

/// Build a minimal ESAM SignalProcessingNotification response.
pub fn build_notification(acq_id: &str, _utc: &str, action: &str, params: &serde_json::Value) -> String {
    let mut extra = String::new();
    // CRITICAL FIX: Handle both "replace" AND "noop" actions to pass through SCTE-35 payload
    if action.eq_ignore_ascii_case("replace") || action.eq_ignore_ascii_case("noop") {
        if let Some(b64) = params.get("scte35_b64").and_then(|v| v.as_str()) {
            extra = format!(r#"<sig:BinaryData signalType="SCTE35">{}</sig:BinaryData>"#, xml_escape(b64));
        }
    }
    
    // Use current UTC time plus 4 seconds instead of original UTC point
    let response_utc = chrono::Utc::now() + chrono::Duration::seconds(4);
    let utc_str = response_utc.to_rfc3339();
    
    format!(r#"
<SignalProcessingNotification
  xmlns="urn:cablelabs:iptvservices:esam:xsd:signal:1"
  xmlns:sig="urn:cablelabs:md:xsd:signaling:3.0"
  xmlns:core="urn:cablelabs:md:xsd:core:3.0"
  xmlns:common="urn:cablelabs:iptvservices:esam:xsd:common:1">
  <common:StatusCode classCode="0">
    <core:Note>{note}</core:Note>
  </common:StatusCode>
  <ResponseSignal action="{action}" acquisitionSignalID="{acq}" acquisitionPointIdentity="pois-techexlab">
    <sig:UTCPoint utcPoint="{utc}"/>
    {extra}
  </ResponseSignal>
</SignalProcessingNotification>
"#,
        note = if action == "delete" { "filtered signal" } else if action == "replace" { "replaced signal" } else { "pass-through" },
        action = xml_escape(action),
        acq = xml_escape(acq_id),
        utc = xml_escape(&utc_str),
        extra = extra
    ).trim().to_string()
}

/// Decode segmentation type ID to human-readable name
fn decode_segmentation_type_name(type_id: u8) -> String {
    match type_id {
        0x00 => "Not Indicated".to_string(),
        0x01 => "Content Identification".to_string(),
        0x10 => "Program Start".to_string(),
        0x11 => "Program End".to_string(), 
        0x12 => "Program Early Termination".to_string(),
        0x13 => "Program Breakaway".to_string(),
        0x14 => "Program Resumption".to_string(),
        0x15 => "Program Runover Planned".to_string(),
        0x16 => "Program Runover Unplanned".to_string(),
        0x17 => "Program Overlap Start".to_string(),
        0x18 => "Program Blackout Override".to_string(),
        0x19 => "Program Start - In Progress".to_string(),
        0x20 => "Chapter Start".to_string(),
        0x21 => "Chapter End".to_string(),
        0x22 => "Break Start".to_string(),
        0x23 => "Break End".to_string(),
        0x24 => "Opening Credit Start".to_string(),
        0x25 => "Opening Credit End".to_string(),
        0x26 => "Closing Credit Start".to_string(),
        0x27 => "Closing Credit End".to_string(),
        0x30 => "Provider Advertisement Start".to_string(),
        0x31 => "Provider Advertisement End".to_string(),
        0x32 => "Distributor Advertisement Start".to_string(),
        0x33 => "Distributor Advertisement End".to_string(),
        0x34 => "Provider Placement Opportunity Start".to_string(),
        0x35 => "Provider Placement Opportunity End".to_string(),
        0x36 => "Distributor Placement Opportunity Start".to_string(),
        0x37 => "Distributor Placement Opportunity End".to_string(),
        0x38 => "Provider Overlay Placement Opportunity Start".to_string(),
        0x39 => "Provider Overlay Placement Opportunity End".to_string(),
        0x3A => "Distributor Overlay Placement Opportunity Start".to_string(),
        0x3B => "Distributor Overlay Placement Opportunity End".to_string(),
        0x40 => "Unscheduled Event Start".to_string(),
        0x41 => "Unscheduled Event End".to_string(),
        0x50 => "Network Start".to_string(),
        0x51 => "Network End".to_string(),
        0x81 => "Custom/Vendor Specific (0x81)".to_string(),
        _ if type_id >= 0x80 => format!("Custom/Vendor Specific (0x{:02X})", type_id),
        _ => format!("Reserved (0x{:02X})", type_id),
    }
}

/// Decode UPID type to human-readable name
fn decode_upid_type_name(upid_type: u8) -> String {
    match upid_type {
        0x00 => "Not Used".to_string(),
        0x01 => "User Defined (Deprecated)".to_string(),
        0x02 => "ISCI (Deprecated)".to_string(), 
        0x03 => "Ad-ID".to_string(),
        0x04 => "UMID".to_string(),
        0x05 => "ISAN".to_string(),
        0x06 => "V-ISAN".to_string(),
        0x07 => "TI".to_string(),
        0x08 => "ADI".to_string(),
        0x09 => "EIDR".to_string(),
        0x0A => "ATSC Content Identifier".to_string(),
        0x0B => "MPU".to_string(),
        0x0C => "MID".to_string(),
        0x0D => "ADS Information".to_string(),
        0x0E => "URI".to_string(),
        0x0F => "UUID".to_string(),
        0x10 => "SCR".to_string(),
        _ => format!("Reserved/Unknown (0x{:02X})", upid_type),
    }
}

/// Decode UPID data based on type
fn decode_upid_data(upid_type: u8, data: &[u8]) -> String {
    match upid_type {
        0x00 => "Not Used".to_string(),
        0x03 => decode_ad_id(data),        // Ad-ID
        0x05 => decode_isan(data),         // ISAN
        0x07 => decode_ti(data),           // TI
        0x08 => decode_adi(data),          // ADI
        0x09 => decode_eidr(data),         // EIDR
        0x0C => decode_mid(data),          // MID
        0x0E => decode_uri(data),          // URI
        0x0F => decode_uuid(data),         // UUID
        _ => {
            // For unknown types, show both ASCII (if printable) and hex
            if is_ascii_printable(data) {
                format!("ASCII: {}", String::from_utf8_lossy(data))
            } else {
                format!("hex: {}", hex_encode(data))
            }
        }
    }
}

fn decode_ad_id(data: &[u8]) -> String {
    if data.len() == 12 && is_ascii_printable(data) {
        format!("Ad-ID: {}", String::from_utf8_lossy(data))
    } else {
        format!("Ad-ID (invalid): hex:{}", hex_encode(data))
    }
}

fn decode_isan(data: &[u8]) -> String {
    if data.len() >= 8 {
        format!("ISAN: hex:{}", hex_encode(data))
    } else {
        format!("ISAN (invalid): hex:{}", hex_encode(data))
    }
}

fn decode_ti(data: &[u8]) -> String {
    if data.len() == 8 {
        let ti = u64::from_be_bytes(data.try_into().unwrap_or([0; 8]));
        format!("TI: {}", ti)
    } else {
        format!("TI (invalid): hex:{}", hex_encode(data))
    }
}

fn decode_adi(data: &[u8]) -> String {
    if is_ascii_printable(data) {
        format!("ADI: {}", String::from_utf8_lossy(data))
    } else {
        format!("ADI: hex:{}", hex_encode(data))
    }
}

fn decode_eidr(data: &[u8]) -> String {
    if data.len() >= 12 {
        format!("EIDR: hex:{}", hex_encode(data))
    } else {
        format!("EIDR (invalid): hex:{}", hex_encode(data))
    }
}

fn decode_mid(data: &[u8]) -> String {
    if data.is_empty() {
        return "MID: (empty)".to_string();
    }
    
    let mut result = String::from("MID: ");
    let mut pos = 0;
    
    while pos < data.len() {
        if pos + 1 >= data.len() { break; }
        
        let sub_type = data[pos];
        let sub_len = data[pos + 1] as usize;
        pos += 2;
        
        if pos + sub_len > data.len() { break; }
        
        let sub_data = &data[pos..pos + sub_len];
        result.push_str(&format!("[Type 0x{:02X}: {}] ", sub_type, 
            decode_upid_data(sub_type, sub_data)));
        pos += sub_len;
    }
    
    result
}

fn decode_uri(data: &[u8]) -> String {
    if is_ascii_printable(data) {
        format!("URI: {}", String::from_utf8_lossy(data))
    } else {
        format!("URI (invalid): hex:{}", hex_encode(data))
    }
}

fn decode_uuid(data: &[u8]) -> String {
    if data.len() == 16 {
        format!("UUID: {:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            data[0], data[1], data[2], data[3],
            data[4], data[5], data[6], data[7],
            data[8], data[9], data[10], data[11],
            data[12], data[13], data[14], data[15])
    } else {
        format!("UUID (invalid): hex:{}", hex_encode(data))
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn is_ascii_printable(bytes: &[u8]) -> bool {
    bytes.iter().all(|&b| b == 0x09 || b == 0x0A || b == 0x0D || (0x20..=0x7E).contains(&b))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0F) as usize] as char);
    }
    s
}

#[derive(Default)]
pub struct Scte35Info {
    pub command: Option<String>,
    pub segmentation_type_id: Option<u8>,
    pub segmentation_upid_with_type: Option<(u8, Vec<u8>)>, // (type, data)
    pub pts_time: Option<u64>,
}

/// Minimal bit reader for SCTE-35 parsing.
struct BitReader<'a> {
    data: &'a [u8],
    bitpos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data, bitpos: 0 } }

    fn read_u8(&mut self, nbits: u32) -> Result<u8, String>  { Ok(self.read_bits(nbits)? as u8) }
    fn read_u16(&mut self, nbits: u32) -> Result<u16, String> { Ok(self.read_bits(nbits)? as u16) }
    fn read_u32(&mut self, nbits: u32) -> Result<u32, String> { Ok(self.read_bits(nbits)? as u32) }

    #[allow(dead_code)]
    fn read_u64(&mut self, nbits: u32) -> Result<u64, String> { self.read_bits(nbits) }

    fn read_bits(&mut self, nbits: u32) -> Result<u64, String> {
        let mut v = 0u64;
        for _ in 0..nbits {
            let idx = self.bitpos / 8;
            if idx >= self.data.len() { return Err("out of bounds".into()); }
            let byte = self.data[idx];
            let bit = 7 - (self.bitpos % 8);
            v = (v << 1) | (((byte >> bit) & 1) as u64);
            self.bitpos += 1;
        }
        Ok(v)
    }

    fn skip_bits(&mut self, nbits: u32) -> Result<(), String> {
        self.bitpos = self.bitpos.checked_add(nbits as usize).ok_or_else(|| "overflow".to_string())?;
        if self.bitpos / 8 > self.data.len() { return Err("out of bounds".into()); }
        Ok(())
    }
}

/// Return command + optional segmentation details + PTS from SCTE-35
/// v2.2.1: Fixed parse_splice_insert_pts to include missing final fields
/// v2.2.0: Fixed descriptor_loop_length parsing with proper bit masking
pub fn decode_scte35_details(b64: &str) -> Result<Scte35Info, String> {
    debug!("decode_scte35_details: Starting decode, base64 length={}", b64.len());
    
    let bytes = B64.decode(b64).map_err(|e| {
        let err_msg = format!("base64 decode error: {e}");
        error!("SCTE-35 DECODE FAILED: {}", err_msg);
        err_msg
    })?;
    
    debug!("decode_scte35_details: Decoded {} bytes total, first 4 bytes: {:02X?}", 
           bytes.len(), &bytes[..4.min(bytes.len())]);
    
    if bytes.first() != Some(&0xFC) {
        warn!("SCTE-35: Invalid first byte (expected 0xFC, got {:02X?}) - returning default", bytes.first());
        return Ok(Scte35Info::default());
    }
    
    let mut br = BitReader::new(&bytes);
    
    let table_id = br.read_u8(8).map_err(|e| {
        let err_msg = format!("Failed to read table_id: {e}");
        error!("SCTE-35 DECODE FAILED: {}", err_msg);
        err_msg
    })?;
    
    debug!("decode_scte35_details: table_id=0x{:02x}", table_id);
    
    if table_id != 0xFC { 
        warn!("SCTE-35: table_id mismatch (expected 0xFC, got 0x{:02x}) - returning default", table_id);
        return Ok(Scte35Info::default()); 
    }

    debug!("decode_scte35_details: Skipping section header (16 bits)...");
    // section_syntax_indicator, private_indicator, reserved(2), section_length(12)
    br.skip_bits(1 + 1 + 2 + 12).map_err(|e| {
        let err_msg = format!("Failed to skip section header: {e} at bitpos {}", br.bitpos);
        error!("SCTE-35 DECODE FAILED: {}", err_msg);
        err_msg
    })?;
    
    debug!("decode_scte35_details: After section header, bitpos={}", br.bitpos);
    
    debug!("decode_scte35_details: Skipping SCTE-35 header fields (68 bits)...");
    // protocol_version(8), encrypted(1), encryption_algorithm(6), pts_adjustment(33), cw_index(8), tier(12)
    br.skip_bits(8 + 1 + 6 + 33 + 8 + 12).map_err(|e| {
        let err_msg = format!("Failed to skip SCTE-35 header: {e} at bitpos {} (byte {})", 
                             br.bitpos, br.bitpos / 8);
        error!("SCTE-35 DECODE FAILED: {}", err_msg);
        err_msg
    })?;
    
    debug!("decode_scte35_details: After header fields, bitpos={} (byte {})", 
           br.bitpos, br.bitpos / 8);

    debug!("decode_scte35_details: Reading splice_command_length (12 bits)...");
    let splice_command_length = br.read_u16(12).map_err(|e| {
        let err_msg = format!("Failed to read splice_command_length: {e} at bitpos {} (byte {})", 
                             br.bitpos, br.bitpos / 8);
        error!("SCTE-35 DECODE FAILED: {}", err_msg);
        err_msg
    })? as usize;
    
    debug!("decode_scte35_details: Reading splice_command_type (8 bits)...");
    let splice_command_type = br.read_u8(8).map_err(|e| {
        let err_msg = format!("Failed to read splice_command_type: {e} at bitpos {} (byte {})", 
                             br.bitpos, br.bitpos / 8);
        error!("SCTE-35 DECODE FAILED: {}", err_msg);
        err_msg
    })?;
    
    debug!("decode_scte35_details: splice_command_length={}, splice_command_type=0x{:02x}", 
           splice_command_length, splice_command_type);
    
    let command_name = match splice_command_type {
        0x00 => "splice_null",
        0x04 => "splice_schedule", 
        0x05 => "splice_insert",
        0x06 => "time_signal",
        0x07 => "bandwidth_reservation",
        0xFF => "private_command",
        _ => {
            warn!("SCTE-35: Unrecognized splice_command_type 0x{:02x}, treating as 'unknown'", 
                  splice_command_type);
            "unknown"
        }
    };
    
    debug!("decode_scte35_details: ✅ Successfully decoded command: {}", command_name);
    
    let mut info = Scte35Info {
        command: Some(command_name.into()),
        segmentation_type_id: None,
        segmentation_upid_with_type: None,
        pts_time: None,
    };

    // Parse command-specific data to extract PTS times
    match splice_command_type {
        0x05 => {
            debug!("decode_scte35_details: Parsing splice_insert for PTS...");
            // splice_insert() command - parse for PTS time
            if let Ok(pts) = parse_splice_insert_pts(&mut br) {
                info.pts_time = pts;
                debug!("decode_scte35_details: Extracted PTS: {:?}", pts);
            }
        },
        0x06 => {
            debug!("decode_scte35_details: Parsing time_signal for PTS...");
            // time_signal() command - parse splice_time()
            if let Ok(pts) = parse_splice_time(&mut br) {
                info.pts_time = pts;
                debug!("decode_scte35_details: Extracted PTS: {:?}", pts);
            }
        },
        _ => {
            debug!("decode_scte35_details: Command type 0x{:02x} - no PTS extraction needed", splice_command_type);
        }
    }

    // v2.2.1 FIX: parse_splice_insert_pts now includes all final fields (break_duration, unique_program_id, avail_num, avails_expected)
    // v2.2.0 FIX: REMOVED the v2.1.0 skip logic - it was causing wrong bitpos!
    // The command parsers already consume the correct number of bytes.
    // descriptor_loop_length immediately follows the command data.

    info!("decode_scte35_details: After command, bitpos={} (byte {})", br.bitpos, br.bitpos / 8);

    // v2.2.0 FIX: descriptor_loop_length is 10 bits with 6 reserved bits
    debug!("decode_scte35_details: Reading descriptor_loop_length (16 bits with masking)...");
    let descriptor_word = br.read_u16(16).map_err(|e| {
        let err_msg = format!("Failed to read descriptor_loop_length: {e} at bitpos {} (byte {})", 
                             br.bitpos, br.bitpos / 8);
        warn!("SCTE-35 WARNING: {}", err_msg);
        err_msg
    })?;
    let descriptor_loop_length = (descriptor_word & 0x03FF) as usize;  // Mask to get lower 10 bits
    
    info!("decode_scte35_details: descriptor_loop_word=0x{:04X}, masked_length={} bytes", 
          descriptor_word, descriptor_loop_length);
    
    let loop_end = br.bitpos + descriptor_loop_length * 8;
    while br.bitpos + 16 <= loop_end {
        let tag = br.read_u8(8)?;
        let len = br.read_u8(8)? as usize;
        let desc_end = br.bitpos + len * 8;

        if tag == 0x02 {
            debug!("decode_scte35_details: Found segmentation_descriptor (tag=0x02, length={})", len);
            // segmentation_descriptor
            let _cuei = br.read_u32(32)?;
            let _event_id = br.read_u32(32)?;
            let cancel = br.read_u8(1)? == 1;
            br.skip_bits(7)?;
            if !cancel {
                let program_flag = br.read_u8(1)? == 1;
                let duration_flag = br.read_u8(1)? == 1;
                let delivery_not_restricted = br.read_u8(1)? == 1;
                
                // v2.1.0 FIX: Always skip 5 bits (either reserved or restriction flags)
                br.skip_bits(5)?;
                
                if !program_flag {
                    let count = br.read_u8(8)? as usize;
                    for _ in 0..count {
                        br.skip_bits(8 + 7 + 33)?; // component_tag + reserved + pts_offset
                    }
                }
                if duration_flag { br.skip_bits(40)?; } // segmentation_duration
                
                // Extract UPID type and data
                let upid_type = br.read_u8(8)?;
                let upid_len  = br.read_u8(8)? as usize;
                if upid_len > 0 {
                    let mut upid = Vec::with_capacity(upid_len);
                    for _ in 0..upid_len { upid.push(br.read_u8(8)?); }
                    info.segmentation_upid_with_type = Some((upid_type, upid));
                    debug!("decode_scte35_details: Extracted UPID (type=0x{:02x}, {} bytes)", 
                           upid_type, upid_len);
                }
                
                if br.bitpos + 24 <= desc_end {
                    let seg_type_id = br.read_u8(8)?;
                    info.segmentation_type_id = Some(seg_type_id);
                    info!("decode_scte35_details: Extracted segmentation_type_id=0x{:02x}", seg_type_id);
                    br.skip_bits(16)?; // segment_num + segments_expected
                }
                
                // Log delivery_not_restricted for debugging
                debug!("decode_scte35_details: delivery_not_restricted={}", delivery_not_restricted);
            }
        } else {
            debug!("decode_scte35_details: Skipping descriptor tag=0x{:02x}, length={}", tag, len);
        }

        if br.bitpos < desc_end {
            br.skip_bits((desc_end - br.bitpos) as u32)?;
        }
    }

    info!("decode_scte35_details: ✅ DECODE COMPLETE - command={}, type_id={:?}, has_upid={}", 
           command_name, info.segmentation_type_id, info.segmentation_upid_with_type.is_some());
    
    Ok(info)
}

/// Parse splice_time() structure from time_signal command
fn parse_splice_time(br: &mut BitReader) -> Result<Option<u64>, String> {
    let time_specified_flag = br.read_u8(1)? == 1;
    if time_specified_flag {
        // reserved 6 bits, then 33-bit pts_time
        br.skip_bits(6)?;
        let pts_time = br.read_bits(33)?;
        Ok(Some(pts_time))
    } else {
        // reserved 7 bits, no pts_time
        br.skip_bits(7)?;
        Ok(None)
    }
}

/// Parse splice_insert() command for PTS time
fn parse_splice_insert_pts(br: &mut BitReader) -> Result<Option<u64>, String> {
    let _splice_event_id = br.read_u32(32)?;
    let splice_event_cancel_indicator = br.read_u8(1)? == 1;
    br.skip_bits(7)?; // reserved
    
    if !splice_event_cancel_indicator {
        let _out_of_network_indicator = br.read_u8(1)?;
        let program_splice_flag = br.read_u8(1)? == 1;
        let duration_flag = br.read_u8(1)? == 1;
        let splice_immediate_flag = br.read_u8(1)? == 1;
        br.skip_bits(4)?; // reserved
        
        let mut pts_result = None;
        
        if program_splice_flag && !splice_immediate_flag {
            // Parse splice_time() for program splice
            pts_result = parse_splice_time(br)?;
        } else if !program_splice_flag {
            let component_count = br.read_u8(8)? as usize;
            for _ in 0..component_count {
                let _component_tag = br.read_u8(8)?;
                if !splice_immediate_flag {
                    // Each component has a splice_time()
                    if let Ok(Some(pts)) = parse_splice_time(br) {
                        if pts_result.is_none() {
                            pts_result = Some(pts);
                        }
                    }
                }
            }
        }
        
        // v2.2.1 CRITICAL FIX: Parse remaining splice_insert fields
        // These fields must be consumed to advance bitpos correctly for descriptor_loop_length
        // Without this, bitpos stops at byte 20 instead of byte 29 (72 bits short)
        
        // Parse break_duration if duration_flag is set (40 bits)
        if duration_flag {
            let _auto_return = br.read_u8(1)?;
            br.skip_bits(6)?; // reserved
            let _duration_ticks = br.read_u64(33)?;
        }
        
        // Always present at end of splice_insert: unique_program_id (16), avail_num (8), avails_expected (8)
        let _unique_program_id = br.read_u16(16)?;
        let _avail_num = br.read_u8(8)?;
        let _avails_expected = br.read_u8(8)?;
        
        return Ok(pts_result);
    }
    
    Ok(None)
}
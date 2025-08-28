use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use quick_xml::{events::Event, Reader};
use serde_json::json;

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
                    if let Ok(Event::Text(t)) = reader.read_event_into(&mut buf) {
                        scte35_b64 = Some(t.unescape().map_err(|e| e.to_string())?.to_string());
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
        if let Ok(info) = decode_scte35_details(b64) {
            scte35_cmd = info.command;
            
            if let Some(type_id) = info.segmentation_type_id {
                seg_type_id_hex = Some(format!("0x{type_id:02X}"));
                seg_type_name = Some(decode_segmentation_type_name(type_id));
            }
            
            if let Some((upid_type, upid_bytes)) = info.segmentation_upid_with_type {
                upid_type_name = Some(decode_upid_type_name(upid_type));
                seg_upid_repr = Some(decode_upid_data(upid_type, &upid_bytes));
            }
            
            pts_time = info.pts_time;
        }
    }
    let scte35_cmd_str = scte35_cmd.unwrap_or_else(|| "unknown".into());

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
    if action.eq_ignore_ascii_case("replace") {
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
struct Scte35Info {
    command: Option<String>,
    segmentation_type_id: Option<u8>,
    segmentation_upid_with_type: Option<(u8, Vec<u8>)>, // (type, data)
    pts_time: Option<u64>,
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
fn decode_scte35_details(b64: &str) -> Result<Scte35Info, String> {
    let bytes = B64.decode(b64).map_err(|e| format!("base64 error: {e}"))?;
    if bytes.first() != Some(&0xFC) {
        return Ok(Scte35Info::default());
    }
    let mut br = BitReader::new(&bytes);
    let table_id = br.read_u8(8)?;
    if table_id != 0xFC { return Ok(Scte35Info::default()); }

    // section_syntax_indicator, private_indicator, reserved(2), section_length(12)
    br.skip_bits(1 + 1 + 2 + 12)?;
    // protocol_version(8), encrypted(1), encryption_algorithm(6), pts_adjustment(33), cw_index(8), tier(12)
    br.skip_bits(8 + 1 + 6 + 33 + 8 + 12)?;

    let splice_command_length = br.read_u16(12)? as usize;
    let splice_command_type = br.read_u8(8)?;
    let mut info = Scte35Info {
        command: Some(match splice_command_type {
            0x00 => "splice_null",
            0x04 => "splice_schedule", 
            0x05 => "splice_insert",
            0x06 => "time_signal",
            0x07 => "bandwidth_reservation",
            0xFF => "private_command",
            _ => "unknown",
        }.into()),
        segmentation_type_id: None,
        segmentation_upid_with_type: None,
        pts_time: None,
    };

    // Parse command-specific data to extract PTS times
    match splice_command_type {
        0x05 => {
            // splice_insert() command - parse for PTS time
            if splice_command_length > 0 {
                if let Ok(pts) = parse_splice_insert_pts(&mut br) {
                    info.pts_time = pts;
                }
                // Skip any remaining command bytes
                let current_pos = br.bitpos;
                let expected_end = current_pos + (splice_command_length * 8);
                if br.bitpos < expected_end {
                    let remaining_bits = expected_end - br.bitpos;
                    if remaining_bits > 0 {
                        br.skip_bits(remaining_bits as u32).ok();
                    }
                }
            }
        },
        0x06 => {
            // time_signal() command - parse splice_time()
            if splice_command_length > 0 {
                if let Ok(pts) = parse_splice_time(&mut br) {
                    info.pts_time = pts;
                }
                // Skip any remaining command bytes
                let remaining_bits = (splice_command_length * 8).saturating_sub(5);
                if remaining_bits > 0 {
                    br.skip_bits(remaining_bits as u32).ok();
                }
            }
        },
        _ => {
            // Skip command bytes for other command types
            br.skip_bits((splice_command_length * 8) as u32)?;
        }
    }

    // descriptor_loop_length
    let descriptor_loop_length = br.read_u16(16)? as usize;
    let loop_end = br.bitpos + descriptor_loop_length * 8;
    while br.bitpos + 16 <= loop_end {
        let tag = br.read_u8(8)?;
        let len = br.read_u8(8)? as usize;
        let desc_end = br.bitpos + len * 8;

        if tag == 0x02 {
            // segmentation_descriptor
            let _cuei = br.read_u32(32)?;
            let _event_id = br.read_u32(32)?;
            let cancel = br.read_u8(1)? == 1;
            br.skip_bits(7)?;
            if !cancel {
                let program_flag = br.read_u8(1)? == 1;
                let duration_flag = br.read_u8(1)? == 1;
                let delivery_not_restricted = br.read_u8(1)? == 1;
                if !delivery_not_restricted { br.skip_bits(5)?; }
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
                }
                
                if br.bitpos + 24 <= desc_end {
                    let seg_type_id = br.read_u8(8)?;
                    info.segmentation_type_id = Some(seg_type_id);
                    br.skip_bits(16)?; // segment_num + segments_expected
                }
            }
        }

        if br.bitpos < desc_end {
            br.skip_bits((desc_end - br.bitpos) as u32)?;
        }
    }

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
        let _duration_flag = br.read_u8(1)?;
        let splice_immediate_flag = br.read_u8(1)? == 1;
        br.skip_bits(4)?; // reserved
        
        if program_splice_flag && !splice_immediate_flag {
            // Parse splice_time() for program splice
            return parse_splice_time(br);
        } else if !program_splice_flag {
            let component_count = br.read_u8(8)? as usize;
            for _ in 0..component_count {
                let _component_tag = br.read_u8(8)?;
                if !splice_immediate_flag {
                    // Each component has a splice_time()
                    if let Ok(Some(pts)) = parse_splice_time(br) {
                        // Return the first PTS time found
                        return Ok(Some(pts));
                    }
                }
            }
        }
    }
    
    Ok(None)
}
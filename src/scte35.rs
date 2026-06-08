// src/scte35.rs
// Version: 2.1.0 - splice_command_length now excludes the splice_command_type byte
// Updated: 2026-06-08
// (Previously off by one: it counted the type byte, so a time_signal immediate
//  emitted scl=2 over a 1-byte body. Spec-strict decoders like scte35-reader use
//  splice_command_length to delimit the command, so they dropped the section. Now
//  scl measures the command body only, after the type byte; CRC recomputed.)

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// Public API: return base64 SCTE-35 payloads.
pub fn build_time_signal_immediate_b64() -> String {
    let sec = build_time_signal_immediate_section();
    B64.encode(sec)
}

pub fn build_splice_insert_out_b64(duration_s: u32) -> String {
    let sec = build_splice_insert_out_section(duration_s, None, None, None);
    B64.encode(sec)
}

/// NEW: Advanced builder with custom segmentation descriptor
#[allow(dead_code)]
pub fn build_splice_insert_out_advanced_b64(
    duration_s: u32,
    seg_type_id: Option<u8>,
    upid_type: Option<u8>,
    upid_value: Option<&str>,
) -> String {
    let sec = build_splice_insert_out_section(duration_s, seg_type_id, upid_type, upid_value);
    B64.encode(sec)
}

/// NEW: Time signal with segmentation descriptor
#[allow(dead_code)]
pub fn build_time_signal_advanced_b64(
    seg_type_id: Option<u8>,
    upid_type: Option<u8>,
    upid_value: Option<&str>,
) -> String {
    let sec = build_time_signal_section(seg_type_id, upid_type, upid_value);
    B64.encode(sec)
}

#[allow(dead_code)]
pub fn build_splice_insert_in_b64() -> String {
    let sec = build_splice_insert_in_section();
    B64.encode(sec)
}

#[allow(dead_code)]
pub fn build_splice_insert_in_with_pts_b64(pts_time: u64) -> String {
    let sec = build_splice_insert_in_with_pts_section(pts_time);
    B64.encode(sec)
}

// ---- Internal: section builders (binary) ----

fn build_time_signal_immediate_section() -> Vec<u8> {
    build_time_signal_section(None, None, None)
}

fn build_time_signal_section(
    seg_type_id: Option<u8>,
    upid_type: Option<u8>,
    upid_value: Option<&str>,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u8(0xFC);
    w.u1(0);
    w.u1(0);
    w.u2(3);
    let section_length_pos = w.reserve_u12();

    w.u8(0);      // protocol_version
    w.u1(0);      // encrypted_packet
    w.u6(0);      // encryption_algorithm
    w.u33(0);     // pts_adjustment
    w.u8(0);      // cw_index
    w.u12(0x0FFF);// tier

    let splice_cmd_len_pos = w.bitpos();
    w.u12(0);     // splice_command_length (patch later)
    w.u8(0x06);   // splice_command_type = time_signal (NOT part of splice_command_length)
    // splice_command_length measures the splice_command() body only, after the
    // type byte (per spec; e.g. splice_null has length 0). A strict decoder uses
    // it to delimit the command, so it must exclude the type byte.
    let splice_cmd_start = w.bitpos();
    w.u1(0);      // time_specified_flag = 0 (immediate)
    w.u7(0);      // reserved

    let splice_cmd_bits = w.bitpos() - splice_cmd_start;
    w.patch_u12(splice_cmd_len_pos, (splice_cmd_bits/8) as u16);

    // Add segmentation descriptor if params provided
    if seg_type_id.is_some() || upid_type.is_some() {
        add_segmentation_descriptor(&mut w, None, seg_type_id, upid_type, upid_value);
    } else {
        w.u16(0); // descriptor_loop_length = 0
    }

    finalize_with_crc32(&mut w, section_length_pos)
}

fn build_splice_insert_out_section(
    duration_s: u32,
    seg_type_id: Option<u8>,
    upid_type: Option<u8>,
    upid_value: Option<&str>,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u8(0xFC);
    w.u1(0);
    w.u1(0);
    w.u2(3);
    let section_length_pos = w.reserve_u12();

    w.u8(0);      // protocol_version
    w.u1(0);      // encrypted_packet
    w.u6(0);      // encryption_algorithm
    w.u33(0);     // pts_adjustment
    w.u8(0);      // cw_index
    w.u12(0x0FFF);// tier

    let splice_cmd_len_pos = w.bitpos();
    w.u12(0);           // splice_command_length (patch later)
    w.u8(0x05);         // splice_command_type = splice_insert (NOT part of splice_command_length)
    let splice_cmd_start = w.bitpos();
    w.u32(1);           // splice_event_id
    w.u1(0);            // splice_event_cancel_indicator
    w.u7(0);            // reserved
    
    // flags
    w.u1(1); // out_of_network_indicator = 1 (OUT)
    w.u1(1); // program_splice_flag = 1
    w.u1(1); // duration_flag = 1
    w.u1(1); // splice_immediate_flag = 1
    w.u4(0); // reserved

    // break_duration
    w.u1(1); // auto_return
    w.u6(0); // reserved
    let dur90k = duration_s as u64 * 90000;
    w.u33(dur90k);

    // unique_program_id, avail_num, avails_expected
    w.u16(1);
    w.u8(0);
    w.u8(0);

    let splice_cmd_bits = w.bitpos() - splice_cmd_start;
    w.patch_u12(splice_cmd_len_pos, (splice_cmd_bits/8) as u16);

    // Add segmentation descriptor
    add_segmentation_descriptor(&mut w, Some(dur90k), seg_type_id, upid_type, upid_value);

    finalize_with_crc32(&mut w, section_length_pos)
}

/// NEW: Helper to add segmentation descriptor with custom parameters
fn add_segmentation_descriptor(
    w: &mut BitWriter,
    duration_90k: Option<u64>,
    seg_type_id: Option<u8>,
    upid_type: Option<u8>,
    upid_value: Option<&str>,
) {
    let desc_loop_start = w.bitpos();
    w.u16(0); // descriptor_loop_length placeholder

    // segmentation_descriptor (tag 0x02)
    w.u8(0x02);
    let seg_len_pos = w.reserve_u8();
    w.u32(0x43554549); // "CUEI"
    w.u32(1); // segmentation_event_id
    w.u1(0);  // segmentation_event_cancel_indicator
    w.u7(0);  // reserved

    // flags
    w.u1(1); // program_segmentation_flag
    w.u1(if duration_90k.is_some() { 1 } else { 0 }); // segmentation_duration_flag
    w.u1(1); // delivery_not_restricted_flag
    w.u5(0); // reserved (no restriction flags)

    // no components (program_segmentation_flag=1)

    // segmentation_duration (if provided)
    if let Some(dur) = duration_90k {
        w.u40(dur);
    }

    // UPID
    let upid_type_val = upid_type.unwrap_or(0x0C); // Default to MID
    w.u8(upid_type_val);
    
    let upid_bytes = if let Some(val) = upid_value {
        encode_upid(upid_type_val, val)
    } else {
        // Default UPID value
        b"POIS-OUT".to_vec()
    };
    
    w.u8(upid_bytes.len() as u8);
    for b in &upid_bytes {
        w.u8(*b);
    }

    // segmentation_type_id
    let seg_type = seg_type_id.unwrap_or(0x10); // Default to Program Start
    w.u8(seg_type);
    
    // segment_num, segments_expected
    w.u8(0);
    w.u8(0);

    // Handle sub-segment fields for certain types
    if matches!(seg_type, 0x34 | 0x36 | 0x38 | 0x3A) {
        // Distributor types that may have sub-segments
        w.u8(0); // sub_segment_num
        w.u8(0); // sub_segments_expected
    }

    // patch segmentation_descriptor length
    let seg_bits = w.bitpos() - (seg_len_pos + 8);
    w.patch_u8(seg_len_pos, (seg_bits / 8) as u8);

    // patch descriptor_loop_length
    let loop_bits = w.bitpos() - (desc_loop_start + 16);
    w.patch_u16(desc_loop_start, (loop_bits / 8) as u16);
}

/// NEW: Encode UPID value based on type
pub(crate) fn encode_upid(upid_type: u8, value: &str) -> Vec<u8> {
    match upid_type {
        0x01 | 0x02 | 0x03 | 0x0C => {
            // User Defined, ISCI, Ad-ID, MID - treat as ASCII
            value.as_bytes().to_vec()
        }
        0x08 => {
            // TI (Turner ID) - 8 bytes hex
            hex_decode(value).unwrap_or_else(|| value.as_bytes().to_vec())
        }
        0x09 => {
            // ADI - variable length binary
            hex_decode(value).unwrap_or_else(|| value.as_bytes().to_vec())
        }
        0x0B => {
            // UUID - 16 bytes
            parse_uuid(value).unwrap_or_else(|| value.as_bytes().to_vec())
        }
        _ => {
            // Default to ASCII
            value.as_bytes().to_vec()
        }
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.replace(['-', ' ', ':'], "");
    if !s.len().is_multiple_of(2) {
        return None;
    }
    
    let mut bytes = Vec::new();
    for i in (0..s.len()).step_by(2) {
        if let Ok(b) = u8::from_str_radix(&s[i..i+2], 16) {
            bytes.push(b);
        } else {
            return None;
        }
    }
    Some(bytes)
}

fn parse_uuid(s: &str) -> Option<Vec<u8>> {
    let s = s.replace(['-', ' '], "");
    if s.len() != 32 {
        return None;
    }
    hex_decode(&s)
}

#[allow(dead_code)]
fn build_splice_insert_in_section() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u8(0xFC);
    w.u1(0);
    w.u1(0);
    w.u2(3);
    let section_length_pos = w.reserve_u12();

    w.u8(0);      // protocol_version
    w.u1(0);      // encrypted_packet
    w.u6(0);      // encryption_algorithm
    w.u33(0);     // pts_adjustment
    w.u8(0);      // cw_index
    w.u12(0x0FFF);// tier

    let splice_cmd_len_pos = w.bitpos();
    w.u12(0);           // splice_command_length (patch later)
    w.u8(0x05);         // splice_command_type = splice_insert (NOT part of splice_command_length)
    let splice_cmd_start = w.bitpos();
    w.u32(2);           // splice_event_id
    w.u1(0);            // splice_event_cancel_indicator
    w.u7(0);            // reserved
    
    // flags
    w.u1(0); // out_of_network_indicator = 0 (IN)
    w.u1(1); // program_splice_flag = 1
    w.u1(0); // duration_flag = 0
    w.u1(1); // splice_immediate_flag = 1
    w.u4(0); // reserved

    // unique_program_id, avail_num, avails_expected
    w.u16(1);
    w.u8(0);
    w.u8(0);

    let splice_cmd_bits = w.bitpos() - splice_cmd_start;
    w.patch_u12(splice_cmd_len_pos, (splice_cmd_bits/8) as u16);

    // descriptor_loop_length = 0
    w.u16(0);

    finalize_with_crc32(&mut w, section_length_pos)
}

#[allow(dead_code)]
fn build_splice_insert_in_with_pts_section(pts_time: u64) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.u8(0xFC);
    w.u1(0);
    w.u1(0);
    w.u2(3);
    let section_length_pos = w.reserve_u12();

    w.u8(0);      // protocol_version
    w.u1(0);      // encrypted_packet
    w.u6(0);      // encryption_algorithm
    w.u33(0);     // pts_adjustment
    w.u8(0);      // cw_index
    w.u12(0x0FFF);// tier

    let splice_cmd_len_pos = w.bitpos();
    w.u12(0);           // splice_command_length (patch later)
    w.u8(0x05);         // splice_command_type = splice_insert (NOT part of splice_command_length)
    let splice_cmd_start = w.bitpos();
    w.u32(2);           // splice_event_id
    w.u1(0);            // splice_event_cancel_indicator
    w.u7(0);            // reserved
    
    // flags
    w.u1(0); // out_of_network_indicator = 0 (IN)
    w.u1(1); // program_splice_flag = 1
    w.u1(0); // duration_flag = 0
    w.u1(0); // splice_immediate_flag = 0 (using PTS)
    w.u4(0); // reserved

    // splice_time() with PTS
    w.u1(1); // time_specified_flag = 1
    w.u6(0); // reserved
    w.u33(pts_time);

    // unique_program_id, avail_num, avails_expected
    w.u16(1);
    w.u8(0);
    w.u8(0);

    let splice_cmd_bits = w.bitpos() - splice_cmd_start;
    w.patch_u12(splice_cmd_len_pos, (splice_cmd_bits/8) as u16);

    // descriptor_loop_length = 0
    w.u16(0);

    finalize_with_crc32(&mut w, section_length_pos)
}

// ---- BitWriter helper ----

struct BitWriter {
    bytes: Vec<u8>,
    bitpos: usize,
}

impl BitWriter {
    fn new() -> Self { Self { bytes: Vec::new(), bitpos: 0 } }
    
    fn u1(&mut self, v: u8) { self.write_bits(v as u64, 1); }
    fn u2(&mut self, v: u8) { self.write_bits(v as u64, 2); }
    fn u4(&mut self, v: u8) { self.write_bits(v as u64, 4); }
    fn u5(&mut self, v: u8) { self.write_bits(v as u64, 5); }
    fn u6(&mut self, v: u8) { self.write_bits(v as u64, 6); }
    fn u7(&mut self, v: u8) { self.write_bits(v as u64, 7); }
    fn u8(&mut self, v: u8) { self.write_bits(v as u64, 8); }
    fn u12(&mut self, v: u16) { self.write_bits(v as u64, 12); }
    fn u16(&mut self, v: u16) { self.write_bits(v as u64, 16); }
    fn u32(&mut self, v: u32) { self.write_bits(v as u64, 32); }
    fn u33(&mut self, v: u64) { self.write_bits(v, 33); }
    fn u40(&mut self, v: u64) { self.write_bits(v, 40); }

    fn write_bits(&mut self, val: u64, nbits: usize) {
        for i in (0..nbits).rev() {
            let bit = ((val >> i) & 1) as u8;
            let byte_idx = self.bitpos / 8;
            let bit_idx = 7 - (self.bitpos % 8);
            
            if byte_idx >= self.bytes.len() {
                self.bytes.push(0);
            }
            self.bytes[byte_idx] |= bit << bit_idx;
            self.bitpos += 1;
        }
    }

    fn reserve_u8(&mut self) -> usize {
        let pos = self.bitpos();
        self.u8(0);
        pos
    }

    fn reserve_u12(&mut self) -> usize {
        let pos = self.bitpos();
        self.u12(0);
        pos
    }

    fn patch_u1(&mut self, bitpos: usize, val: u8) {
        let byte_idx = bitpos / 8;
        let bit_idx = 7 - (bitpos % 8);
        if byte_idx < self.bytes.len() {
            self.bytes[byte_idx] &= !(1 << bit_idx);
            self.bytes[byte_idx] |= (val & 1) << bit_idx;
        }
    }

    fn patch_u8(&mut self, bitpos: usize, val: u8) {
        for i in 0..8 {
            let bit = (val >> (7 - i)) & 1;
            self.patch_u1(bitpos + i, bit);
        }
    }

    fn patch_u12(&mut self, bitpos: usize, val: u16) {
        for i in 0..12 {
            let bit = ((val >> (11 - i)) & 1) as u8;
            self.patch_u1(bitpos + i, bit);
        }
    }

    fn patch_u16(&mut self, bitpos: usize, val: u16) {
        for i in 0..16 {
            let bit = ((val >> (15 - i)) & 1) as u8;
            self.patch_u1(bitpos + i, bit);
        }
    }

    fn bitpos(&self) -> usize { self.bitpos }
}

fn finalize_with_crc32(w: &mut BitWriter, section_length_pos: usize) -> Vec<u8> {
    // Align to byte
    while !w.bitpos.is_multiple_of(8) {
        w.u1(1);
    }

    // Compute section_length (from after section_length field to end, including 4-byte CRC)
    let section_start_byte = (section_length_pos + 12) / 8;
    let section_length = (w.bytes.len() - section_start_byte) + 4;
    w.patch_u12(section_length_pos, section_length as u16);

    // Compute CRC-32 over entire section (from table_id to end before CRC)
    let crc = compute_crc32(&w.bytes);
    w.u32(crc);

    w.bytes.clone()
}

pub(crate) fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFF_u32;
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
mod splice_command_length_tests {
    //! `splice_command_length` must equal the serialized length of the
    //! splice_command() body AFTER the splice_command_type byte (per spec; a
    //! spec-strict decoder such as `scte35-reader` uses it to delimit the command,
    //! and our own decoder is lenient so it can't catch a wrong value). These tests
    //! parse the field directly and confirm the descriptor loop lands exactly where
    //! `splice_command_length` says the command ends.
    use super::*;
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    fn decode(b64: &str) -> Vec<u8> {
        B64.decode(b64).unwrap()
    }

    // Fixed header is 13 bytes; tier(12) + splice_command_length(12) sit in bytes
    // 10..13, so scl is the low nibble of byte 11 plus all of byte 12.
    fn scl(b: &[u8]) -> usize {
        (((b[11] & 0x0F) as usize) << 8) | b[12] as usize
    }
    fn cmd_type(b: &[u8]) -> u8 {
        b[13]
    }

    // Strict parse: the command body is exactly `scl` bytes after the type byte,
    // immediately followed by descriptor_loop_length, the descriptors, and the
    // 4-byte CRC-32. With the old off-by-one, scl over-counts and this fails.
    fn descriptor_loop_well_formed(b: &[u8]) -> bool {
        let dll_pos = 14 + scl(b); // type byte at 13, command body at 14..14+scl
        if dll_pos + 2 > b.len() {
            return false;
        }
        let dll = ((b[dll_pos] as usize) << 8) | b[dll_pos + 1] as usize;
        dll_pos + 2 + dll + 4 == b.len()
    }

    #[test]
    fn time_signal_immediate_scl_is_one() {
        let b = decode(&build_time_signal_immediate_b64());
        assert_eq!(cmd_type(&b), 0x06);
        assert_eq!(scl(&b), 1, "time_signal (time_specified_flag=0) body is 1 byte");
        assert!(descriptor_loop_well_formed(&b));
    }

    #[test]
    fn time_signal_with_segmentation_strict_parse() {
        // The ba-wafb-live / report repro: time_signal + seg 0x34 + ADI UPID 0x09.
        let b = decode(&build_time_signal_advanced_b64(
            Some(0x34),
            Some(0x09),
            Some("umc.cse.21xuvhesfjb2vw9ldryme46a5/break110"),
        ));
        assert_eq!(cmd_type(&b), 0x06);
        assert_eq!(scl(&b), 1);
        assert!(
            descriptor_loop_well_formed(&b),
            "descriptor loop must start right after the 1-byte time_signal"
        );
    }

    #[test]
    fn splice_insert_out_scl_matches_body() {
        let b = decode(&build_splice_insert_out_b64(30));
        assert_eq!(cmd_type(&b), 0x05);
        // event_id(4) + cancel/res(1) + flags(1) + break_duration(5) + uniq(2) + avail(1) + avails(1)
        assert_eq!(scl(&b), 15);
        assert!(descriptor_loop_well_formed(&b));
    }

    #[test]
    fn splice_insert_out_advanced_strict_parse() {
        let b = decode(&build_splice_insert_out_advanced_b64(
            60,
            Some(0x34),
            Some(0x09),
            Some("test-upid"),
        ));
        assert_eq!(scl(&b), 15);
        assert!(descriptor_loop_well_formed(&b));
    }

    #[test]
    fn splice_insert_in_scl_matches_body() {
        let b = decode(&build_splice_insert_in_b64());
        assert_eq!(cmd_type(&b), 0x05);
        // event_id(4) + cancel/res(1) + flags(1) + uniq(2) + avail(1) + avails(1)
        assert_eq!(scl(&b), 10);
        assert!(descriptor_loop_well_formed(&b));
    }

    #[test]
    fn splice_insert_in_with_pts_scl_matches_body() {
        let b = decode(&build_splice_insert_in_with_pts_b64(0x1_2345_6789));
        // adds splice_time() PTS (5 bytes) vs the immediate IN variant
        assert_eq!(scl(&b), 15);
        assert!(descriptor_loop_well_formed(&b));
    }
}
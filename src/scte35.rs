// src/scte35.rs
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// Public API: return base64 SCTE-35 payloads.
pub fn build_time_signal_immediate_b64() -> String {
    let sec = build_time_signal_immediate_section();
    B64.encode(sec)
}

pub fn build_splice_insert_out_b64(duration_s: u32) -> String {
    let sec = build_splice_insert_out_section(duration_s);
    B64.encode(sec)
}

pub fn build_splice_insert_in_b64() -> String {
    let sec = build_splice_insert_in_section();
    B64.encode(sec)
}

pub fn build_splice_insert_in_with_pts_b64(pts_time: u64) -> String {
    let sec = build_splice_insert_in_with_pts_section(pts_time);
    B64.encode(sec)
}

// ---- Internal: section builders (binary) ----

fn build_time_signal_immediate_section() -> Vec<u8> {
    // table_id (0xFC) + header fields per SCTE-35
    let mut w = BitWriter::new();
    w.u8(0xFC);               // table_id
    w.u1(0);                  // section_syntax_indicator
    w.u1(0);                  // private_indicator
    w.u2(3);                  // reserved
    let section_length_pos = w.reserve_u12(); // to be backfilled

    w.u8(0);                  // protocol_version
    w.u1(0);                  // encrypted_packet
    w.u6(0);                  // encryption_algorithm
    w.u33(0);                 // pts_adjustment
    w.u8(0);                  // cw_index
    w.u12(0x0FFF);            // tier

    // splice_command_length + type
    let splice_cmd_start = w.bitpos();
    w.u12(0);                 // splice_command_length (patch later)
    w.u8(0x06);               // splice_command_type = time_signal
    // time_signal() : splice_time()
    w.u1(0);                  // time_specified_flag = 0 (immediate)
    w.u7(0);                  // reserved

    let splice_cmd_bits = w.bitpos() - splice_cmd_start;
    w.patch_u12(splice_cmd_start, (splice_cmd_bits/8) as u16);

    // descriptor_loop_length = 0
    w.u16(0);

    // CRC will be appended after we serialize bytes
    finalize_with_crc32(&mut w, section_length_pos)
}

fn build_splice_insert_out_section(duration_s: u32) -> Vec<u8> {
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

    let splice_cmd_start = w.bitpos();
    w.u12(0);           // splice_command_length (patch later)
    w.u8(0x05);         // splice_insert
    // splice_insert() fields (minimal, immediate OUT)
    w.u32(1);           // splice_event_id
    w.u1(0);            // splice_event_cancel_indicator
    w.u7(0);            // reserved
    // flags
    w.u1(1); // out_of_network_indicator = 1 (OUT)
    w.u1(1); // program_splice_flag = 1
    w.u1(0); // duration_flag (patched below)
    w.u1(0); // splice_immediate_flag (patched below)
    w.u4(0); // reserved

    // splice_immediate = 1, program_splice=1 => no splice_time() field
    // duration_flag = 1 -> break_duration
    w.patch_u1(w.bitpos()-3, 1); // duration_flag = 1
    w.patch_u1(w.bitpos()-2, 1); // splice_immediate_flag = 1

    // break_duration (duration_flag=1)
    w.u1(1); // auto_return
    w.u6(0); // reserved
    let dur90k = duration_s as u64 * 90000;
    w.u33(dur90k);

    // unique_program_id, avail_num, avails_expected
    w.u16(1);
    w.u8(0);
    w.u8(0);

    let splice_cmd_bits = w.bitpos() - splice_cmd_start;
    w.patch_u12(splice_cmd_start, (splice_cmd_bits/8) as u16);

    // Add a segmentation_descriptor to mark OUT (0x10 Program Start commonly used for CUE-OUT)
    let desc_loop_start = w.bitpos();
    w.u16(0); // descriptor_loop_length placeholder

    // segmentation_descriptor (tag 0x02)
    w.u8(0x02);
    let seg_len_pos = w.reserve_u8();
    w.u32(0x43554549); // "CUEI"
    w.u32(2); // segmentation_event_id
    w.u1(0);  // cancellation
    w.u7(0);

    // flags
    w.u1(1); // program_segmentation_flag
    w.u1(1); // segmentation_duration_flag
    w.u1(1); // delivery_not_restricted
    // no restriction bits

    // no components (program_flag=1)

    // segmentation_duration
    w.u40(dur90k);

    // UPID: type 0x0C MID with ASCII "POIS-OUT"
    w.u8(0x0C);
    let upid = b"POIS-OUT";
    w.u8(upid.len() as u8);
    for b in upid { w.u8(*b); }

    // segmentation_type_id + segment numbers
    w.u8(0x10); // Program Start (OUT)
    w.u8(0);
    w.u8(0);

    // patch segmentation_descriptor length
    let seg_bits = w.bitpos() - (seg_len_pos+8);
    w.patch_u8(seg_len_pos, (seg_bits/8) as u8);

    // patch descriptor_loop_length
    let loop_bits = w.bitpos() - (desc_loop_start+16);
    w.patch_u16(desc_loop_start, (loop_bits/8) as u16);

    finalize_with_crc32(&mut w, section_length_pos)
}

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

    let splice_cmd_start = w.bitpos();
    w.u12(0);           // splice_command_length (patch later)
    w.u8(0x05);         // splice_insert
    // splice_insert() fields (minimal, immediate IN)
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
    w.patch_u12(splice_cmd_start, (splice_cmd_bits/8) as u16);

    // descriptor_loop_length = 0
    w.u16(0);

    finalize_with_crc32(&mut w, section_length_pos)
}

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

    let splice_cmd_start = w.bitpos();
    w.u12(0);           // splice_command_length (patch later)
    w.u8(0x05);         // splice_insert
    // splice_insert() fields
    w.u32(2);           // splice_event_id
    w.u1(0);            // splice_event_cancel_indicator
    w.u7(0);            // reserved
    // flags
    w.u1(0); // out_of_network_indicator = 0 (IN)
    w.u1(1); // program_splice_flag = 1
    w.u1(0); // duration_flag = 0
    w.u1(0); // splice_immediate_flag = 0 (using PTS)
    w.u4(0); // reserved

    // splice_time() - program_splice_flag=1, splice_immediate_flag=0
    w.u1(1); // time_specified_flag = 1
    w.u6(0); // reserved
    w.u33(pts_time); // pts_time

    // unique_program_id, avail_num, avails_expected
    w.u16(1);
    w.u8(0);
    w.u8(0);

    let splice_cmd_bits = w.bitpos() - splice_cmd_start;
    w.patch_u12(splice_cmd_start, (splice_cmd_bits/8) as u16);

    // descriptor_loop_length = 0
    w.u16(0);

    finalize_with_crc32(&mut w, section_length_pos)
}

// -------- bit writer + crc ----------

struct BitWriter {
    bytes: Vec<u8>,
    bitpos: usize,
}
impl BitWriter {
    fn new() -> Self { Self { bytes: Vec::with_capacity(256), bitpos: 0 } }
    fn bitpos(&self) -> usize { self.bitpos }
    fn ensure(&mut self, bits: usize) {
        let need = (self.bitpos + bits + 7)/8;
        if need > self.bytes.len() { self.bytes.resize(need, 0); }
    }
    fn put_bit(&mut self, bit: u8) {
        self.ensure(1);
        let byte_idx = self.bitpos / 8;
        let shift = 7 - (self.bitpos % 8);
        self.bytes[byte_idx] |= (bit & 1) << shift;
        self.bitpos += 1;
    }
    fn u1(&mut self, v: u8) { self.put_bit(v & 1); }
    fn u2(&mut self, v: u8) { for i in (0..2).rev() { self.put_bit((v >> i) & 1); } }
    fn u4(&mut self, v: u8) { for i in (0..4).rev() { self.put_bit((v >> i) & 1); } }
    fn u6(&mut self, v: u8) { for i in (0..6).rev() { self.put_bit((v >> i) & 1); } }
    fn u7(&mut self, v: u8) { for i in (0..7).rev() { self.put_bit((v >> i) & 1); } }
    fn u8(&mut self, v: u8) { for i in (0..8).rev() { self.put_bit((v >> i) & 1); } }
    fn u12(&mut self, v: u16) { for i in (0..12).rev() { self.put_bit(((v>>i)&1) as u8); } }
    fn u16(&mut self, v: u16) { for i in (0..16).rev() { self.put_bit(((v>>i)&1) as u8); } }
    fn u32(&mut self, v: u32) { for i in (0..32).rev() { self.put_bit(((v>>i)&1) as u8); } }
    fn u33(&mut self, v: u64) { for i in (0..33).rev() { self.put_bit(((v>>i)&1) as u8); } }
    fn u40(&mut self, v: u64) { for i in (0..40).rev() { self.put_bit(((v>>i)&1) as u8); } }

    fn reserve_u12(&mut self) -> usize { let pos = self.bitpos; self.u12(0); pos }
    fn reserve_u8(&mut self) -> usize { let pos = self.bitpos; self.u8(0); pos }

    fn patch_u12(&mut self, bitpos: usize, v: u16) {
        let cur = self.bitpos; self.bitpos = bitpos; self.u12(v); self.bitpos = cur;
    }
    fn patch_u8(&mut self, bitpos: usize, v: u8) {
        let cur = self.bitpos; self.bitpos = bitpos; self.u8(v); self.bitpos = cur;
    }
    fn patch_u16(&mut self, bitpos: usize, v: u16) {
        let cur = self.bitpos; self.bitpos = bitpos; self.u16(v); self.bitpos = cur;
    }
    fn patch_u1(&mut self, bitpos: usize, v: u8) {
        let cur = self.bitpos; self.bitpos = bitpos; self.u1(v); self.bitpos = cur;
    }
}

fn finalize_with_crc32(w: &mut BitWriter, section_length_pos: usize) -> Vec<u8> {
    // section_length counts bytes from after that field up to and including CRC
    let len_without_crc = (w.bytes.len() - 3) + 4; // -3 for header bytes (id + 2 length), +4 for CRC
    w.patch_u12(section_length_pos, len_without_crc as u16);
    // compute CRC over entire section starting at table_id
    let crc = crc32_mpeg2(&w.bytes);
    let mut out = w.bytes.clone();
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

// ISO/IEC-13818-1 MPEG-2 CRC32 (poly 0x04C11DB7, init=0xFFFFFFFF, no final xor)
fn crc32_mpeg2(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            if (crc & 0x80000000) != 0 { crc = (crc << 1) ^ 0x04C11DB7; } else { crc <<= 1; }
        }
    }
    crc
}
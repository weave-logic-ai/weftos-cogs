//! Incremental reassembler for the HLK-LD2450 UART stream (Hi-Link "Serial Communication
//! Protocol" V1.03, 2023-10-17). Two frame kinds share the line, both little-endian:
//!
//! ```text
//! report: AA FF 03 00 | 3 x { x:u16, y:u16, speed:u16, resolution:u16 } | 55 CC   (30 bytes)
//! ACK:    FD FC FB FA | len:u16 | ack_word:u16 status:u16 value[len-4] | 04 03 02 01
//! ```
//!
//! Neither kind carries a checksum: the fixed header, the fixed tail and (for reports) the
//! fixed length are the only integrity checks. On any mismatch the framer drops one byte
//! and rescans, the same resync rule as ld6002-radar.

/// Report frame header (§2.3, Table 9).
pub const REPORT_HEADER: [u8; 4] = [0xAA, 0xFF, 0x03, 0x00];
/// Report frame tail.
pub const REPORT_TAIL: [u8; 2] = [0x55, 0xCC];
/// Header + 3 targets x 8 bytes + tail.
pub const REPORT_LEN: usize = 30;
/// Command and ACK frame header (§2.1.2, Tables 2 and 4).
pub const CMD_HEADER: [u8; 4] = [0xFD, 0xFC, 0xFB, 0xFA];
/// Command and ACK frame tail.
pub const CMD_TAIL: [u8; 4] = [0x04, 0x03, 0x02, 0x01];
/// Longest ACK payload accepted. The radar reports a 0x40-byte buffer in its
/// enable-config ACK; the longest documented ACK (zone filter query) is 0x1E bytes.
pub const MAX_ACK_DATA: usize = 64;
/// An ACK's command word is the sent command word with this bit set (`0x00FF` -> `0x01FF`).
pub const ACK_BIT: u16 = 0x0100;

/// Signed-magnitude decode used by x, y and speed: the high bit set means positive,
/// clear means negative, and the low 15 bits are the magnitude. `0x0000` and `0x8000`
/// are both zero. Protocol §2.3 Table 10 and its worked example.
pub fn sign_magnitude(raw: u16) -> i32 {
    let mag = i32::from(raw & 0x7FFF);
    if raw & 0x8000 != 0 {
        mag
    } else {
        -mag
    }
}

/// Inverse of [`sign_magnitude`]; magnitudes beyond 15 bits saturate.
pub fn to_sign_magnitude(v: i32) -> u16 {
    let mag = v.unsigned_abs().min(0x7FFF) as u16;
    if v > 0 {
        0x8000 | mag
    } else {
        mag
    }
}

/// One 8-byte target slot, decoded to integers in the radar's units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Slot {
    pub x_mm: i32,
    pub y_mm: i32,
    pub speed_cm_s: i32,
    /// "Distance resolution": the size of one distance gate in mm, as reported.
    pub resolution_mm: u16,
}

impl Slot {
    fn decode(b: &[u8]) -> Slot {
        let w = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        Slot {
            x_mm: sign_magnitude(w(0)),
            y_mm: sign_magnitude(w(2)),
            speed_cm_s: sign_magnitude(w(4)),
            resolution_mm: w(6),
        }
    }

    /// An unused slot. The protocol sends absent targets as all-zero bytes; a slot whose
    /// x and y both decode to 0 is a point on the radar itself, so it is treated as
    /// empty whatever its speed and resolution words hold.
    pub fn is_empty(&self) -> bool {
        self.x_mm == 0 && self.y_mm == 0
    }
}

/// A decoded command acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ack {
    /// The command word this ACK answers (the ACK word with [`ACK_BIT`] cleared).
    pub command: u16,
    /// 0 success, 1 failure.
    pub status: u16,
    /// Return value after the status word.
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Report([Slot; 3]),
    Ack(Ack),
}

/// Cumulative framer counters. Take differences between two snapshots for a window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FramerStats {
    pub reports: u64,
    pub acks: u64,
    /// Bytes that belonged to valid frames.
    pub frame_bytes: u64,
    /// A report or ACK header whose tail did not match.
    pub bad_tail: u64,
    /// An ACK header declaring a payload longer than [`MAX_ACK_DATA`] or shorter than
    /// its 4-byte command word and status.
    pub bad_ack_len: u64,
    /// Bytes dropped while searching for the next frame.
    pub resync_bytes: u64,
}

impl FramerStats {
    pub fn since(&self, earlier: &FramerStats) -> FramerStats {
        FramerStats {
            reports: self.reports - earlier.reports,
            acks: self.acks - earlier.acks,
            frame_bytes: self.frame_bytes - earlier.frame_bytes,
            bad_tail: self.bad_tail - earlier.bad_tail,
            bad_ack_len: self.bad_ack_len - earlier.bad_ack_len,
            resync_bytes: self.resync_bytes - earlier.resync_bytes,
        }
    }

    pub fn parse_errors(&self) -> u64 {
        self.bad_tail + self.bad_ack_len
    }
}

/// Feed raw UART bytes, get whole frames back. Holds at most one partial frame.
#[derive(Debug, Default)]
pub struct Framer {
    buf: Vec<u8>,
    stats: FramerStats,
}

/// What the bytes at an offset can still become.
enum Head {
    Report,
    Cmd,
    /// A strict prefix of a header: wait for more bytes.
    Partial,
    None,
}

fn head(rest: &[u8]) -> Head {
    for (hdr, kind) in [(REPORT_HEADER, Head::Report), (CMD_HEADER, Head::Cmd)] {
        if rest.len() >= 4 {
            if rest[..4] == hdr {
                return kind;
            }
        } else if hdr.starts_with(rest) {
            return Head::Partial;
        }
    }
    Head::None
}

impl Framer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> FramerStats {
        self.stats
    }

    fn skip(&mut self, off: &mut usize) {
        *off += 1;
        self.stats.resync_bytes += 1;
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(chunk);
        let buf = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        let n = buf.len();
        let mut off = 0;
        while off < n {
            match head(&buf[off..]) {
                Head::None => self.skip(&mut off),
                Head::Partial => break,
                Head::Report => {
                    if n - off < REPORT_LEN {
                        break;
                    }
                    let f = &buf[off..off + REPORT_LEN];
                    if f[28..] != REPORT_TAIL {
                        self.stats.bad_tail += 1;
                        self.skip(&mut off);
                        continue;
                    }
                    let slots = [
                        Slot::decode(&f[4..12]),
                        Slot::decode(&f[12..20]),
                        Slot::decode(&f[20..28]),
                    ];
                    out.push(Frame::Report(slots));
                    self.stats.reports += 1;
                    self.stats.frame_bytes += REPORT_LEN as u64;
                    off += REPORT_LEN;
                }
                Head::Cmd => {
                    if n - off < 6 {
                        break;
                    }
                    let len = usize::from(u16::from_le_bytes([buf[off + 4], buf[off + 5]]));
                    if !(4..=MAX_ACK_DATA).contains(&len) {
                        self.stats.bad_ack_len += 1;
                        self.skip(&mut off);
                        continue;
                    }
                    let total = 6 + len + 4;
                    if n - off < total {
                        break;
                    }
                    let f = &buf[off..off + total];
                    if f[6 + len..] != CMD_TAIL {
                        self.stats.bad_tail += 1;
                        self.skip(&mut off);
                        continue;
                    }
                    let d = &f[6..6 + len];
                    out.push(Frame::Ack(Ack {
                        command: u16::from_le_bytes([d[0], d[1]]) & !ACK_BIT,
                        status: u16::from_le_bytes([d[2], d[3]]),
                        value: d[4..].to_vec(),
                    }));
                    self.stats.acks += 1;
                    self.stats.frame_bytes += total as u64;
                    off += total;
                }
            }
        }
        self.buf = buf;
        self.buf.drain(..off);
        out
    }
}

/// Command words used by this cog (protocol §2.2).
pub mod cmd {
    pub const ENABLE_CONFIG: u16 = 0x00FF;
    pub const END_CONFIG: u16 = 0x00FE;
    pub const SINGLE_TARGET: u16 = 0x0080;
    pub const MULTI_TARGET: u16 = 0x0090;
    pub const QUERY_TRACKING_MODE: u16 = 0x0091;
    pub const READ_FIRMWARE: u16 = 0x00A0;
}

/// Build a command frame: header, length, command word, value, tail.
pub fn encode_command(word: u16, value: &[u8]) -> Vec<u8> {
    let mut f = CMD_HEADER.to_vec();
    f.extend_from_slice(&((2 + value.len()) as u16).to_le_bytes());
    f.extend_from_slice(&word.to_le_bytes());
    f.extend_from_slice(value);
    f.extend_from_slice(&CMD_TAIL);
    f
}

/// The enable-configuration command carries the value 0x0001.
pub fn enable_config() -> Vec<u8> {
    encode_command(cmd::ENABLE_CONFIG, &1u16.to_le_bytes())
}

/// Build an ACK frame. Used by the simulator and tests.
pub fn encode_ack(command: u16, status: u16, value: &[u8]) -> Vec<u8> {
    let mut f = CMD_HEADER.to_vec();
    f.extend_from_slice(&((4 + value.len()) as u16).to_le_bytes());
    f.extend_from_slice(&(command | ACK_BIT).to_le_bytes());
    f.extend_from_slice(&status.to_le_bytes());
    f.extend_from_slice(value);
    f.extend_from_slice(&CMD_TAIL);
    f
}

/// Build a report frame from three slots. Used by the simulator and tests.
pub fn encode_report(slots: &[Slot; 3]) -> Vec<u8> {
    let mut f = REPORT_HEADER.to_vec();
    for s in slots {
        f.extend_from_slice(&to_sign_magnitude(s.x_mm).to_le_bytes());
        f.extend_from_slice(&to_sign_magnitude(s.y_mm).to_le_bytes());
        f.extend_from_slice(&to_sign_magnitude(s.speed_cm_s).to_le_bytes());
        f.extend_from_slice(&s.resolution_mm.to_le_bytes());
    }
    f.extend_from_slice(&REPORT_TAIL);
    f
}

/// Firmware version from a read-firmware ACK value: firmware type u16, major u16, minor
/// u32. The protocol's example (`00 00 02 01 16 24 06 22`) reads "V1.02.22062416": the
/// major word's bytes and the minor word's hex digits are printed as they are.
pub fn firmware_version(value: &[u8]) -> Option<String> {
    let v = value.get(..8)?;
    let major = u16::from_le_bytes([v[2], v[3]]);
    let minor = u32::from_le_bytes([v[4], v[5], v[6], v[7]]);
    Some(format!(
        "V{:x}.{:02x}.{:08x}",
        major >> 8,
        major & 0xFF,
        minor
    ))
}

/// Tracking mode from a query-tracking-mode ACK value.
pub fn tracking_mode(value: &[u8]) -> Option<&'static str> {
    match u16::from_le_bytes(value.get(..2)?.try_into().ok()?) {
        1 => Some("single"),
        2 => Some("multi"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    /// The report example from protocol V1.03 §2.3 (also instruction manual V1.00 §6).
    const DOC_REPORT: &str = "AA FF 03 00 0E 03 B1 86 10 00 40 01 00 00 00 00 00 00 00 00 \
                              00 00 00 00 00 00 00 00 55 CC";

    fn slot(x: i32, y: i32, v: i32, r: u16) -> Slot {
        Slot {
            x_mm: x,
            y_mm: y,
            speed_cm_s: v,
            resolution_mm: r,
        }
    }

    #[test]
    fn decodes_the_protocol_document_example() {
        let bytes = hex(DOC_REPORT);
        assert_eq!(bytes.len(), REPORT_LEN);
        let mut f = Framer::new();
        let got = f.feed(&bytes);
        let Frame::Report(s) = &got[0] else {
            panic!("{got:?}")
        };
        // The document's worked conversion: x -782 mm, y 1713 mm, -16 cm/s, 320 mm.
        assert_eq!(s[0], slot(-782, 1713, -16, 320));
        assert!(s[1].is_empty() && s[2].is_empty());
        assert_eq!(f.stats().reports, 1);
        assert_eq!(f.stats().frame_bytes, 30);
    }

    #[test]
    fn decodes_the_user_guide_examples() {
        // Hi-Link "HLK-LD2450 User Guide" §6 captures, with the guide's own results.
        for (frame, want) in [
            (
                "AA FF 03 00 10 01 52 83 00 00 68 01",
                slot(-272, 850, 0, 360),
            ),
            (
                "AA FF 03 00 E4 00 87 83 11 80 68 01",
                slot(-228, 903, 17, 360),
            ),
            (
                "AA FF 03 00 BE 8A 47 8E 11 00 68 01",
                slot(2750, 3655, -17, 360),
            ),
        ] {
            let mut bytes = hex(frame);
            bytes.extend([0u8; 16]);
            bytes.extend(REPORT_TAIL);
            let got = Framer::new().feed(&bytes);
            assert_eq!(
                got,
                vec![Frame::Report([want, Slot::default(), Slot::default()])]
            );
        }
    }

    #[test]
    fn sign_magnitude_high_bit_is_positive() {
        assert_eq!(sign_magnitude(0x8000 | 1713), 1713);
        assert_eq!(sign_magnitude(782), -782);
        assert_eq!(sign_magnitude(0x0000), 0);
        assert_eq!(sign_magnitude(0x8000), 0);
        assert_eq!(sign_magnitude(0xFFFF), 32767);
        assert_eq!(sign_magnitude(0x7FFF), -32767);
        for v in [-32767, -1713, -1, 0, 1, 782, 6000, 32767] {
            assert_eq!(sign_magnitude(to_sign_magnitude(v)), v, "{v}");
        }
        assert_eq!(to_sign_magnitude(-40000), 0x7FFF);
    }

    #[test]
    fn all_zero_frame_has_no_targets() {
        let mut bytes = REPORT_HEADER.to_vec();
        bytes.extend([0u8; 24]);
        bytes.extend(REPORT_TAIL);
        let got = Framer::new().feed(&bytes);
        let Frame::Report(s) = &got[0] else { panic!() };
        assert!(s.iter().all(Slot::is_empty));
    }

    #[test]
    fn three_targets_round_trip_in_slot_order() {
        let slots = [
            slot(-1200, 800, 35, 360),
            slot(0, 2500, 0, 360),
            slot(1500, 5900, -120, 360),
        ];
        let got = Framer::new().feed(&encode_report(&slots));
        assert_eq!(got, vec![Frame::Report(slots)]);
    }

    #[test]
    fn resyncs_after_garbage_including_partial_headers() {
        let mut bytes = hex("00 AA FF 13 FD FC 55 CC AA FF 03");
        let garbage = bytes.len() as u64;
        bytes.extend(hex(DOC_REPORT));
        let mut f = Framer::new();
        let got = f.feed(&bytes);
        assert_eq!(got.len(), 1);
        assert_eq!(f.stats().resync_bytes, garbage);
    }

    #[test]
    fn bad_tail_drops_frame_and_recovers() {
        let mut bad = hex(DOC_REPORT);
        bad[29] = 0x00;
        bad.extend(hex(DOC_REPORT));
        let mut f = Framer::new();
        let got = f.feed(&bad);
        assert_eq!(got.len(), 1);
        assert_eq!(f.stats().bad_tail, 1);
        assert_eq!(f.stats().resync_bytes, 30);
    }

    #[test]
    fn truncated_frame_then_full_frame() {
        // A frame cut short (bytes lost on the line) is followed by a full one: the
        // framer must not splice them, and must still find the second.
        let doc = hex(DOC_REPORT);
        let mut bytes = doc[..20].to_vec();
        bytes.extend(&doc);
        bytes.extend(&doc);
        let mut f = Framer::new();
        let got = f.feed(&bytes);
        assert_eq!(got.len(), 2, "{:?}", f.stats());
        assert!(f.stats().bad_tail >= 1);
    }

    #[test]
    fn frame_split_across_every_boundary() {
        let bytes = [
            hex(DOC_REPORT),
            encode_ack(cmd::ENABLE_CONFIG, 0, &[1, 0, 0x40, 0]),
            hex(DOC_REPORT),
        ]
        .concat();
        for cut in 0..=bytes.len() {
            let mut f = Framer::new();
            let mut got = f.feed(&bytes[..cut]);
            got.extend(f.feed(&bytes[cut..]));
            assert_eq!(got.len(), 3, "split at {cut}");
            assert_eq!(f.stats().resync_bytes, 0, "split at {cut}");
        }
    }

    #[test]
    fn byte_at_a_time_matches_bulk() {
        let bytes = [
            hex("01 02 AA"),
            hex(DOC_REPORT),
            encode_ack(0xA0, 0, &[0; 8]),
        ]
        .concat();
        let bulk = Framer::new().feed(&bytes);
        let mut f = Framer::new();
        let trickle: Vec<Frame> = bytes.iter().flat_map(|b| f.feed(&[*b])).collect();
        assert_eq!(bulk, trickle);
        assert_eq!(bulk.len(), 2);
    }

    #[test]
    fn buffer_does_not_grow_on_noise() {
        let mut f = Framer::new();
        for _ in 0..1000 {
            f.feed(&[0x5Au8; 64]);
        }
        assert!(f.buf.len() < 4);
        assert_eq!(f.stats().reports, 0);
    }

    #[test]
    fn commands_match_the_protocol_document() {
        // §2.2.1 to §2.2.6 "Send data" lines.
        assert_eq!(
            enable_config(),
            hex("FD FC FB FA 04 00 FF 00 01 00 04 03 02 01")
        );
        for (word, doc) in [
            (cmd::END_CONFIG, "FD FC FB FA 02 00 FE 00 04 03 02 01"),
            (cmd::SINGLE_TARGET, "FD FC FB FA 02 00 80 00 04 03 02 01"),
            (cmd::MULTI_TARGET, "FD FC FB FA 02 00 90 00 04 03 02 01"),
            (
                cmd::QUERY_TRACKING_MODE,
                "FD FC FB FA 02 00 91 00 04 03 02 01",
            ),
            (cmd::READ_FIRMWARE, "FD FC FB FA 02 00 A0 00 04 03 02 01"),
        ] {
            assert_eq!(encode_command(word, &[]), hex(doc), "{word:#06x}");
        }
    }

    #[test]
    fn decodes_the_protocol_document_acks() {
        let enable = hex("FD FC FB FA 08 00 FF 01 00 00 01 00 40 00 04 03 02 01");
        let fw = hex("FD FC FB FA 0C 00 A0 01 00 00 00 00 02 01 16 24 06 22 04 03 02 01");
        let mode = hex("FD FC FB FA 06 00 91 01 00 00 02 00 04 03 02 01");
        let end = hex("FD FC FB FA 04 00 FE 01 00 00 04 03 02 01");
        let got = Framer::new().feed(&[enable, fw, mode, end].concat());
        let acks: Vec<Ack> = got
            .into_iter()
            .map(|f| match f {
                Frame::Ack(a) => a,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(acks.len(), 4);
        assert_eq!(
            acks[0],
            Ack {
                command: cmd::ENABLE_CONFIG,
                status: 0,
                value: vec![1, 0, 0x40, 0]
            }
        );
        assert_eq!(acks[1].command, cmd::READ_FIRMWARE);
        assert_eq!(
            firmware_version(&acks[1].value).as_deref(),
            Some("V1.02.22062416")
        );
        assert_eq!(tracking_mode(&acks[2].value), Some("multi"));
        assert_eq!((acks[3].command, acks[3].status), (cmd::END_CONFIG, 0));
    }

    #[test]
    fn rejects_bad_ack_lengths() {
        let mut short = CMD_HEADER.to_vec();
        short.extend([2, 0, 0xFE, 0x01]); // len 2 < word + status
        short.extend(CMD_TAIL);
        let mut huge = CMD_HEADER.to_vec();
        huge.extend([0xFF, 0x00]);
        let mut f = Framer::new();
        let got = f.feed(&[short, huge, hex(DOC_REPORT)].concat());
        assert_eq!(got.len(), 1);
        assert_eq!(f.stats().bad_ack_len, 2);
    }
}

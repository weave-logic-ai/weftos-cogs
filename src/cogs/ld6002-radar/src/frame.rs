//! Incremental reassembler for the LD6002 UART frame format (TinyFrame-style).
//!
//! ```text
//! 0x01 | id:u16be | len:u16be | type:u16be | hcksum | payload[len] | dcksum
//! ```
//!
//! `cksum = !(XOR of the covered bytes)`. The header checksum covers the first seven
//! bytes; the data checksum covers the payload. A zero-length frame carries no data
//! checksum. Ported from the host framer bench-confirmed against this radar at
//! 115200 8N1, including its resync rule: on any mismatch, drop one byte and rescan.

/// Start-of-frame byte.
pub const SOF: u8 = 0x01;
/// Longer payloads are treated as desync, matching the reference framer.
pub const MAX_PAYLOAD: usize = 640;
const HEADER_LEN: usize = 8;

/// `!(XOR of bytes)`.
pub fn cksum(buf: &[u8]) -> u8 {
    !buf.iter().fold(0u8, |acc, b| acc ^ b)
}

/// One checksum-valid frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub id: u16,
    pub ty: u16,
    pub payload: Vec<u8>,
}

/// Cumulative framer counters. Take differences between two snapshots for a window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FramerStats {
    pub frames: u64,
    /// Bytes that belonged to valid frames.
    pub frame_bytes: u64,
    /// A SOF byte whose header checksum did not match.
    pub header_checksum_errors: u64,
    /// A valid header whose payload checksum did not match.
    pub data_checksum_errors: u64,
    /// A valid header declaring a payload longer than [`MAX_PAYLOAD`].
    pub oversize_frames: u64,
    /// Bytes dropped while searching for the next frame.
    pub resync_bytes: u64,
}

impl FramerStats {
    pub fn since(&self, earlier: &FramerStats) -> FramerStats {
        FramerStats {
            frames: self.frames - earlier.frames,
            frame_bytes: self.frame_bytes - earlier.frame_bytes,
            header_checksum_errors: self.header_checksum_errors - earlier.header_checksum_errors,
            data_checksum_errors: self.data_checksum_errors - earlier.data_checksum_errors,
            oversize_frames: self.oversize_frames - earlier.oversize_frames,
            resync_bytes: self.resync_bytes - earlier.resync_bytes,
        }
    }

    pub fn checksum_errors(&self) -> u64 {
        self.header_checksum_errors + self.data_checksum_errors
    }
}

/// Feed raw UART bytes, get whole frames back. Holds at most one partial frame.
#[derive(Debug, Default)]
pub struct Framer {
    buf: Vec<u8>,
    stats: FramerStats,
}

impl Framer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stats(&self) -> FramerStats {
        self.stats
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        let buf = &self.buf;
        let n = buf.len();
        let mut off = 0;
        while n - off >= HEADER_LEN {
            if buf[off] != SOF {
                off += 1;
                self.stats.resync_bytes += 1;
                continue;
            }
            if cksum(&buf[off..off + 7]) != buf[off + 7] {
                off += 1;
                self.stats.header_checksum_errors += 1;
                self.stats.resync_bytes += 1;
                continue;
            }
            let id = u16::from_be_bytes([buf[off + 1], buf[off + 2]]);
            let len = u16::from_be_bytes([buf[off + 3], buf[off + 4]]) as usize;
            let ty = u16::from_be_bytes([buf[off + 5], buf[off + 6]]);
            if len > MAX_PAYLOAD {
                off += 1;
                self.stats.oversize_frames += 1;
                self.stats.resync_bytes += 1;
                continue;
            }
            let total = HEADER_LEN + len + usize::from(len > 0);
            if n - off < total {
                break; // partial frame: wait for more bytes
            }
            let payload = &buf[off + HEADER_LEN..off + HEADER_LEN + len];
            if len > 0 && cksum(payload) != buf[off + HEADER_LEN + len] {
                off += 1;
                self.stats.data_checksum_errors += 1;
                self.stats.resync_bytes += 1;
                continue;
            }
            out.push(Frame {
                id,
                ty,
                payload: payload.to_vec(),
            });
            self.stats.frames += 1;
            self.stats.frame_bytes += total as u64;
            off += total;
        }
        self.buf.drain(..off);
        out
    }
}

/// Build a well-formed frame. Used by tests to synthesize radar traffic.
#[cfg(test)]
pub fn encode(id: u16, ty: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![SOF];
    f.extend_from_slice(&id.to_be_bytes());
    f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    f.extend_from_slice(&ty.to_be_bytes());
    f.push(cksum(&f));
    if !payload.is_empty() {
        f.extend_from_slice(payload);
        f.push(cksum(payload));
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hr(bpm: f32) -> Vec<u8> {
        encode(7, 0x0A15, &bpm.to_le_bytes())
    }

    #[test]
    fn checksum_is_inverted_xor() {
        assert_eq!(cksum(&[]), 0xFF);
        assert_eq!(cksum(&[0x01, 0x02]), !0x03);
        assert_eq!(cksum(&[0xAA, 0xAA]), 0xFF);
    }

    #[test]
    fn header_fields_are_big_endian() {
        let bytes = encode(0x1234, 0x0A16, &[1, 2, 3]);
        assert_eq!(&bytes[..7], &[0x01, 0x12, 0x34, 0x00, 0x03, 0x0A, 0x16]);
        let mut f = Framer::new();
        let got = f.feed(&bytes);
        assert_eq!(
            got,
            vec![Frame {
                id: 0x1234,
                ty: 0x0A16,
                payload: vec![1, 2, 3]
            }]
        );
    }

    #[test]
    fn decodes_back_to_back_frames() {
        let mut bytes = hr(72.0);
        bytes.extend(encode(8, 0x0A14, &15.0f32.to_le_bytes()));
        let mut f = Framer::new();
        let got = f.feed(&bytes);
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].ty, 0x0A14);
        assert_eq!(
            f.stats(),
            FramerStats {
                frames: 2,
                frame_bytes: bytes.len() as u64,
                ..Default::default()
            }
        );
    }

    #[test]
    fn zero_length_frame_has_no_data_checksum() {
        let mut bytes = encode(1, 0x0A18, &[]);
        assert_eq!(bytes.len(), 8);
        bytes.extend(hr(60.0));
        let got = Framer::new().feed(&bytes);
        assert_eq!(
            got.iter().map(|f| f.ty).collect::<Vec<_>>(),
            vec![0x0A18, 0x0A15]
        );
        assert!(got[0].payload.is_empty());
    }

    #[test]
    fn resyncs_after_leading_garbage() {
        // Garbage that includes a stray SOF, so the header checksum path is exercised.
        let mut bytes = vec![0xFF, 0x00, 0x01, 0x42, 0x13, 0x37, 0x99, 0x01, 0x02];
        bytes.extend(hr(65.5));
        let mut f = Framer::new();
        let got = f.feed(&bytes);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].payload, 65.5f32.to_le_bytes());
        let s = f.stats();
        assert_eq!(s.resync_bytes, 9);
        assert!(s.header_checksum_errors >= 1);
    }

    #[test]
    fn bad_data_checksum_drops_frame_and_recovers() {
        let mut bad = hr(70.0);
        let last = bad.len() - 1;
        bad[last] ^= 0x5A;
        bad.extend(hr(71.0));
        let mut f = Framer::new();
        let got = f.feed(&bad);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].payload, 71.0f32.to_le_bytes());
        assert_eq!(f.stats().data_checksum_errors, 1);
    }

    #[test]
    fn corrupted_header_is_skipped() {
        let mut bad = hr(70.0);
        bad[4] ^= 0x01; // length byte: header checksum no longer matches
        bad.extend(hr(71.0));
        let mut f = Framer::new();
        let got = f.feed(&bad);
        assert_eq!(got.len(), 1);
        assert!(f.stats().header_checksum_errors >= 1);
    }

    #[test]
    fn oversize_length_is_desync() {
        let mut hdr = vec![SOF, 0, 0, 0x03, 0x00, 0x0A, 0x15]; // len 768 > MAX_PAYLOAD
        hdr.push(cksum(&hdr));
        hdr.extend(hr(64.0));
        let mut f = Framer::new();
        let got = f.feed(&hdr);
        assert_eq!(got.len(), 1);
        assert_eq!(f.stats().oversize_frames, 1);
    }

    #[test]
    fn frame_split_across_every_boundary() {
        let bytes = [hr(80.0), encode(2, 0x0F09, &[1, 0]), encode(3, 0x0A18, &[])].concat();
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
        let bytes = [vec![0x55, 0x01], hr(90.0), encode(4, 0x0A13, &[0u8; 12])].concat();
        let bulk = Framer::new().feed(&bytes);
        let mut f = Framer::new();
        let trickle: Vec<Frame> = bytes.iter().flat_map(|b| f.feed(&[*b])).collect();
        assert_eq!(bulk, trickle);
    }

    #[test]
    fn unknown_types_pass_through() {
        let got = Framer::new().feed(&encode(9, 0xBEEF, &[1, 2, 3, 4]));
        assert_eq!(got[0].ty, 0xBEEF);
    }

    #[test]
    fn buffer_does_not_grow_on_noise() {
        let mut f = Framer::new();
        for _ in 0..1000 {
            f.feed(&[0xFFu8; 64]);
        }
        assert!(f.buf.len() < HEADER_LEN);
        assert_eq!(f.stats().frames, 0);
    }
}

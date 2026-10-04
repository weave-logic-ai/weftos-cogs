//! Where bytes come from: the radar's UART, or a simulator that produces the same wire
//! bytes so `--simulate` exercises the real framer and decoder.

use std::f64::consts::TAU;
use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant};

use crate::frame::{cmd, encode_ack, encode_report, Slot, CMD_HEADER};

pub const READ_TIMEOUT: Duration = Duration::from_millis(50);
/// The radar reports at 10 Hz (instruction manual V1.00 Table 4).
pub const FRAME_PERIOD: Duration = Duration::from_millis(100);

pub trait ByteSource {
    /// Read what is available, waiting at most about [`READ_TIMEOUT`]. `Ok(0)` means
    /// nothing arrived yet.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()>;
}

impl ByteSource for Box<dyn serialport::SerialPort> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match Read::read(self, buf) {
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::Interrupted) => Ok(0),
            other => other,
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        Write::write_all(self, bytes)?;
        self.flush()
    }
}

/// Synthetic radar: one person walking a slow figure in front of it, and a second one
/// standing to the left for part of each 30 s cycle. Positions are deterministic in time.
/// It answers the commands this cog sends and, like the radar, stops reporting between
/// enable-config and end-config.
pub struct Simulator {
    start: Instant,
    sent: u64,
    configuring: bool,
    mode: u16,
    pending: Vec<u8>,
}

impl Default for Simulator {
    fn default() -> Self {
        Self::new()
    }
}

impl Simulator {
    pub fn new() -> Self {
        Simulator {
            start: Instant::now(),
            sent: 0,
            configuring: false,
            mode: 2,
            pending: Vec::new(),
        }
    }

    fn walker(t: f64) -> (f64, f64) {
        (
            1.5 * (TAU * t / 20.0).sin(),
            3.0 + 1.5 * (TAU * t / 13.0).sin(),
        )
    }

    /// The three slots of frame `k` (at `k * 100 ms`).
    pub fn frame(k: u64, multi: bool) -> [Slot; 3] {
        let t = k as f64 * FRAME_PERIOD.as_secs_f64();
        let range = |t: f64| {
            let (x, y) = Self::walker(t);
            x.hypot(y)
        };
        let (x, y) = Self::walker(t);
        // Radial speed, positive moving away: a simulator choice, the radar's own sign
        // convention for speed is not stated in the protocol document.
        let v = (range(t + 0.05) - range(t - 0.05)) / 0.1;
        let mm = |m: f64| (m * 1000.0).round() as i32;
        let mut slots = [Slot::default(); 3];
        slots[0] = Slot {
            x_mm: mm(x),
            y_mm: mm(y),
            speed_cm_s: (v * 100.0).round() as i32,
            resolution_mm: 360,
        };
        if multi && (10.0..25.0).contains(&(t % 30.0)) {
            slots[1] = Slot {
                x_mm: mm(-1.0 + 0.05 * (TAU * t / 4.0).sin()),
                y_mm: 4200,
                speed_cm_s: 0,
                resolution_mm: 360,
            };
        }
        slots
    }
}

impl ByteSource for Simulator {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pending.is_empty() {
            let elapsed = self.start.elapsed();
            let due = (elapsed.as_millis() / FRAME_PERIOD.as_millis()) as u64 + 1;
            if due <= self.sent {
                let next = FRAME_PERIOD * (self.sent as u32);
                std::thread::sleep(next.saturating_sub(elapsed).min(READ_TIMEOUT));
                return Ok(0);
            }
            for k in self.sent..due {
                if !self.configuring {
                    self.pending
                        .extend(encode_report(&Self::frame(k, self.mode == 2)));
                }
            }
            self.sent = due;
        }
        let n = buf.len().min(self.pending.len());
        buf[..n].copy_from_slice(&self.pending[..n]);
        self.pending.drain(..n);
        Ok(n)
    }

    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        if bytes.len() < 12 || bytes[..4] != CMD_HEADER {
            return Ok(()); // the radar ignores what it cannot parse
        }
        let word = u16::from_le_bytes([bytes[6], bytes[7]]);
        let enabled = self.configuring || word == cmd::ENABLE_CONFIG;
        let (status, value): (u16, Vec<u8>) = match word {
            cmd::ENABLE_CONFIG => {
                self.configuring = true;
                (0, vec![1, 0, 0x40, 0])
            }
            _ if !enabled => return Ok(()), // commands outside config mode are invalid
            cmd::END_CONFIG => {
                self.configuring = false;
                (0, vec![])
            }
            cmd::SINGLE_TARGET => {
                self.mode = 1;
                (0, vec![])
            }
            cmd::MULTI_TARGET => {
                self.mode = 2;
                (0, vec![])
            }
            cmd::QUERY_TRACKING_MODE => (0, self.mode.to_le_bytes().to_vec()),
            cmd::READ_FIRMWARE => (0, vec![0; 8]),
            _ => (1, vec![]),
        };
        self.pending.extend(encode_ack(word, status, &value));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Frame, Framer};

    #[test]
    fn walker_stays_in_the_field_of_view() {
        for k in 0..600 {
            let s = Simulator::frame(k, true);
            let (x, y) = (f64::from(s[0].x_mm), f64::from(s[0].y_mm));
            assert!((1000.0..=6000.0).contains(&y), "frame {k}: y {y}");
            // +-60 deg azimuth: |x| <= y * tan(60 deg)
            assert!(x.abs() <= y * 3f64.sqrt(), "frame {k}");
            assert!(s[0].speed_cm_s.abs() < 200);
        }
        assert!(Simulator::frame(150, true)[1].y_mm > 0); // t = 15 s: second person
        assert!(Simulator::frame(50, true)[1].is_empty());
        assert!(Simulator::frame(150, false)[1].is_empty());
    }

    #[test]
    fn answers_commands_and_pauses_reports_while_configuring() {
        let mut sim = Simulator::new();
        sim.write_all(&crate::frame::enable_config()).unwrap();
        sim.write_all(&crate::frame::encode_command(cmd::READ_FIRMWARE, &[]))
            .unwrap();
        let mut buf = [0u8; 256];
        let mut f = Framer::new();
        let mut got = Vec::new();
        for _ in 0..10 {
            let n = sim.read(&mut buf).unwrap();
            got.extend(f.feed(&buf[..n]));
        }
        assert!(got.iter().all(|fr| matches!(fr, Frame::Ack(_))), "{got:?}");
        assert_eq!(got.len(), 2);
    }
}

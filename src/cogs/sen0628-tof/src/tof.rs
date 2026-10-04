//! DFRobot SEN0628 driver over Linux i2c-dev. The board's RP2040 speaks a small command
//! protocol (ported from DFRobot_MatrixLidar.cpp, MIT, 2025-04-03):
//!
//!   request:  0x55, argsH, argsL, cmd, args...    (args count = len(args) + 1)
//!   response: status (0x53 ok / 0x63 failed / 0xFF not ready), cmd, lenL, lenH, data[len]
//!
//! CMD_SETMODE (1) with args [0, 0, 0, 4|8] selects 4x4 or 8x8 (the sensor then needs about
//! 5 s); CMD_ALLDATA (2) returns `zones` little-endian u16 distances in mm, row-major with
//! X left->right and Y top->bottom (DFRobot wiki). Reads are chunked to 32 bytes.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::time::{Duration, Instant};

const I2C_SLAVE: libc::c_ulong = 0x0703;
pub const CMD_SETMODE: u8 = 1;
pub const CMD_ALLDATA: u8 = 2;
pub const STATUS_OK: u8 = 0x53;
pub const STATUS_FAILED: u8 = 0x63;
const NOT_READY: u8 = 0xFF;
const CHUNK: usize = 32;

/// Builds a request packet.
pub fn request(cmd: u8, args: &[u8]) -> Vec<u8> {
    let n = args.len() + 1;
    let mut p = vec![0x55, ((n >> 8) & 0xFF) as u8, (n & 0xFF) as u8, cmd];
    p.extend_from_slice(args);
    p
}

/// Little-endian u16 distances from a CMD_ALLDATA payload.
pub fn decode_frame(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// A source of depth frames: the real sensor or the simulator.
pub trait FrameSource: Send {
    /// One frame of `side * side` distances in mm.
    fn frame(&mut self) -> Result<Vec<u16>, String>;
    fn side(&self) -> usize;
    fn describe(&self) -> String;
}

pub struct Sen0628 {
    dev: File,
    bus: u8,
    addr: u8,
    side: usize,
}

impl Sen0628 {
    /// Opens the bus, checks the address answers, and sets the matrix mode (4 or 8).
    pub fn open(bus: u8, addr: u8, side: usize) -> Result<Self, String> {
        if !(0x30..=0x33).contains(&addr) {
            return Err(format!(
                "address 0x{addr:02x} is not a SEN0628 address (0x30-0x33)"
            ));
        }
        if side != 4 && side != 8 {
            return Err(format!("mode {side} must be 4 (4x4) or 8 (8x8)"));
        }
        let path = format!("/dev/i2c-{bus}");
        let dev = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("open {path}: {e} (is I2C enabled? dtparam=i2c_arm=on)"))?;
        // SAFETY: plain ioctl on an owned, open fd; the argument is the 7-bit address.
        let rc = unsafe { libc::ioctl(dev.as_raw_fd(), I2C_SLAVE as _, libc::c_ulong::from(addr)) };
        if rc < 0 {
            return Err(format!(
                "ioctl I2C_SLAVE 0x{addr:02x}: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut s = Self {
            dev,
            bus,
            addr,
            side,
        };
        let mut probe = [0u8; 1];
        s.dev
            .read_exact(&mut probe)
            .map_err(|e| format!("nothing answered at 0x{addr:02x} on {path}: {e} (check wiring, the I2C/UART switch and the address switch; power-cycle the board after changing either)"))?;
        s.command(CMD_SETMODE, &[0, 0, 0, side as u8], Duration::from_secs(8))
            .map_err(|e| format!("set {side}x{side} mode: {e}"))?;
        // DFRobot's driver waits 5 s for the VL53L7CX to restart ranging after a mode change.
        std::thread::sleep(Duration::from_secs(5));
        Ok(s)
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<(), String> {
        for chunk in buf.chunks_mut(CHUNK) {
            self.dev
                .read_exact(chunk)
                .map_err(|e| format!("i2c read: {e}"))?;
        }
        Ok(())
    }

    /// Sends a command and returns the response payload.
    pub fn command(&mut self, cmd: u8, args: &[u8], timeout: Duration) -> Result<Vec<u8>, String> {
        self.dev
            .write_all(&request(cmd, args))
            .map_err(|e| format!("i2c write cmd {cmd}: {e}"))?;
        let start = Instant::now();
        loop {
            let mut status = [0u8; 1];
            self.read(&mut status)?;
            match status[0] {
                STATUS_OK | STATUS_FAILED => {
                    let mut head = [0u8; 3];
                    self.read(&mut head)?;
                    if head[0] != cmd {
                        return Err(format!(
                            "response for cmd {} while waiting for {cmd}",
                            head[0]
                        ));
                    }
                    let len = usize::from(u16::from_le_bytes([head[1], head[2]]));
                    if len > 1000 {
                        return Err(format!("response length {len} is implausible"));
                    }
                    let mut data = vec![0u8; len];
                    self.read(&mut data)?;
                    if status[0] == STATUS_FAILED {
                        return Err(format!(
                            "sensor reported failure (code {})",
                            data.first().copied().unwrap_or(0)
                        ));
                    }
                    return Ok(data);
                }
                NOT_READY => {}
                other => {
                    // Anything else is line noise or a stale byte; keep polling.
                    let _ = other;
                }
            }
            if start.elapsed() > timeout {
                return Err(format!(
                    "no response to cmd {cmd} within {} ms",
                    timeout.as_millis()
                ));
            }
            std::thread::sleep(Duration::from_millis(17));
        }
    }
}

impl FrameSource for Sen0628 {
    fn frame(&mut self) -> Result<Vec<u16>, String> {
        let data = self.command(CMD_ALLDATA, &[], Duration::from_millis(1500))?;
        let zones = self.side * self.side;
        if data.len() < zones * 2 {
            return Err(format!(
                "frame has {} bytes, want {}",
                data.len(),
                zones * 2
            ));
        }
        Ok(decode_frame(&data[..zones * 2]))
    }

    fn side(&self) -> usize {
        self.side
    }

    fn describe(&self) -> String {
        format!(
            "sen0628@/dev/i2c-{}:0x{:02x}/{}x{}",
            self.bus, self.addr, self.side, self.side
        )
    }
}

/// Synthetic room for testing without hardware: a back wall that tilts away toward the top,
/// a person-sized blob walking left and right, millimetre noise and a few dropouts.
pub struct Simulator {
    side: usize,
    n: u64,
    fps: f64,
    rng: u64,
}

impl Simulator {
    pub fn new(side: usize, fps: f64) -> Self {
        Self {
            side,
            n: 0,
            fps,
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }

    fn noise(&mut self) -> f64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let x = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (x >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }

    /// Column (0..side) where the blob is centred at time t.
    pub fn blob_x(&self, t: f64) -> f64 {
        let s = self.side as f64;
        (s - 1.0) / 2.0 + (s / 2.5) * (2.0 * std::f64::consts::PI * t / 8.0).sin()
    }
}

impl FrameSource for Simulator {
    fn frame(&mut self) -> Result<Vec<u16>, String> {
        let t = self.n as f64 / self.fps;
        self.n += 1;
        let s = self.side;
        let bx = self.blob_x(t);
        let mut out = Vec::with_capacity(s * s);
        for y in 0..s {
            for x in 0..s {
                let wall = 2400.0 + 600.0 * (1.0 - y as f64 / (s as f64 - 1.0));
                let in_blob =
                    (x as f64 - bx).abs() <= s as f64 / 8.0 + 0.5 && y as f64 >= s as f64 * 0.25;
                let d = if in_blob { 1200.0 } else { wall } + 8.0 * self.noise();
                let dropout = self.noise() > 0.985;
                out.push(if dropout { 0 } else { d.round() as u16 });
            }
        }
        Ok(out)
    }

    fn side(&self) -> usize {
        self.side
    }

    fn describe(&self) -> String {
        format!("simulator/{}x{}", self.side, self.side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_packets_match_dfrobots_layout() {
        assert_eq!(request(CMD_ALLDATA, &[]), vec![0x55, 0, 1, 2]);
        assert_eq!(
            request(CMD_SETMODE, &[0, 0, 0, 8]),
            vec![0x55, 0, 5, 1, 0, 0, 0, 8]
        );
    }

    #[test]
    fn frames_decode_little_endian_millimetres() {
        assert_eq!(
            decode_frame(&[0xE8, 0x03, 0xAC, 0x0D, 0x01]),
            vec![1000, 3500]
        );
    }

    #[test]
    fn open_rejects_bad_address_and_mode_before_touching_the_bus() {
        assert!(Sen0628::open(1, 0x48, 8)
            .err()
            .unwrap()
            .contains("not a SEN0628"));
        assert!(Sen0628::open(1, 0x33, 5)
            .err()
            .unwrap()
            .contains("must be 4"));
    }

    #[test]
    fn simulator_produces_a_walking_blob_in_front_of_a_wall() {
        let mut s = Simulator::new(8, 10.0);
        let f = s.frame().unwrap();
        assert_eq!(f.len(), 64);
        let near = f.iter().filter(|&&d| d > 0 && d < 1500).count();
        let far = f.iter().filter(|&&d| d > 2000).count();
        assert!(near >= 6 && far >= 30, "near {near} far {far}");
    }
}

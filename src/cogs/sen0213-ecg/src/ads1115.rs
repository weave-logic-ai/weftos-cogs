//! ADS1115 16-bit ADC over Linux i2c-dev (`/dev/i2c-N`).
//!
//! The ADC runs in continuous-conversion mode at 860 SPS on one single-ended input with the
//! +/-4.096 V range (125 uV per LSB), so every read at up to 500 Hz returns a fresh
//! conversion. The pointer register is left on the conversion register after setup, so each
//! sample is a single 2-byte I2C read.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;

/// `I2C_SLAVE` from linux/i2c-dev.h.
const I2C_SLAVE: libc::c_ulong = 0x0703;

const REG_CONVERSION: u8 = 0x00;
const REG_CONFIG: u8 = 0x01;

/// Volts per LSB at PGA +/-4.096 V.
pub const VOLTS_PER_LSB: f64 = 4.096 / 32768.0;

/// A source of ECG voltage samples. The real ADC and the simulator both implement it, so the
/// sampling loop and the tests share one path.
pub trait EcgSource: Send {
    /// One sample in volts at the ADC pin.
    fn read_volts(&mut self) -> Result<f64, String>;
    /// Human-readable description for reports.
    fn describe(&self) -> String;
}

/// Config register value for continuous mode on `channel` (0-3, single-ended against GND),
/// PGA +/-4.096 V, 860 SPS, comparator disabled.
pub fn config_word(channel: u8) -> u16 {
    let mux: u16 = 0b100 | (u16::from(channel) & 0b11); // AINx vs GND
    (mux << 12) // MUX[14:12]
        | (0b001 << 9) // PGA +/-4.096 V
        // MODE bit 8 = 0: continuous conversion
        | (0b111 << 5) // DR 860 SPS
        | 0b11 // COMP_QUE: comparator disabled
}

/// Signed 16-bit conversion result (big-endian on the wire) to volts.
pub fn raw_to_volts(hi: u8, lo: u8) -> f64 {
    f64::from(i16::from_be_bytes([hi, lo])) * VOLTS_PER_LSB
}

pub struct Ads1115 {
    dev: File,
    bus: u8,
    addr: u8,
    channel: u8,
}

impl Ads1115 {
    /// Opens the bus, configures the ADC and verifies it by reading the config back.
    pub fn open(bus: u8, addr: u8, channel: u8) -> Result<Self, String> {
        if !(0x48..=0x4B).contains(&addr) {
            return Err(format!(
                "address 0x{addr:02x} is not an ADS1115 address (0x48-0x4b)"
            ));
        }
        if channel > 3 {
            return Err(format!("channel {channel} out of range 0-3"));
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
        let mut adc = Self {
            dev,
            bus,
            addr,
            channel,
        };
        let want = config_word(channel);
        adc.write_reg(REG_CONFIG, want)?;
        let got = adc.read_reg(REG_CONFIG)?;
        // Bit 15 (OS) reads back as conversion status, so compare the rest.
        if got & 0x7FFF != want & 0x7FFF {
            return Err(format!(
                "no ADS1115 at 0x{addr:02x} on {path}: config read back 0x{got:04x}, wrote 0x{want:04x}"
            ));
        }
        // Leave the pointer on the conversion register: each sample is then one 2-byte read.
        adc.dev
            .write_all(&[REG_CONVERSION])
            .map_err(|e| format!("set pointer: {e}"))?;
        Ok(adc)
    }

    fn write_reg(&mut self, reg: u8, value: u16) -> Result<(), String> {
        let [hi, lo] = value.to_be_bytes();
        self.dev
            .write_all(&[reg, hi, lo])
            .map_err(|e| format!("i2c write reg 0x{reg:02x} at 0x{:02x}: {e}", self.addr))
    }

    fn read_reg(&mut self, reg: u8) -> Result<u16, String> {
        self.dev
            .write_all(&[reg])
            .map_err(|e| format!("i2c pointer 0x{reg:02x}: {e}"))?;
        let mut b = [0u8; 2];
        self.dev
            .read_exact(&mut b)
            .map_err(|e| format!("i2c read 0x{reg:02x}: {e}"))?;
        Ok(u16::from_be_bytes(b))
    }
}

impl EcgSource for Ads1115 {
    fn read_volts(&mut self) -> Result<f64, String> {
        let mut b = [0u8; 2];
        self.dev
            .read_exact(&mut b)
            .map_err(|e| format!("i2c read: {e}"))?;
        Ok(raw_to_volts(b[0], b[1]))
    }

    fn describe(&self) -> String {
        format!(
            "ads1115@/dev/i2c-{}:0x{:02x}/A{}",
            self.bus, self.addr, self.channel
        )
    }
}

/// Synthetic single-lead ECG for testing without hardware: a P-QRS-T complex at `bpm`
/// around the AD8232's ~1.5 V reference, with mains hum, baseline wander and noise.
pub struct Simulator {
    fs: f64,
    n: u64,
    bpm: f64,
    mains_hz: f64,
    rng: u64,
}

impl Simulator {
    pub fn new(fs: f64, bpm: f64, mains_hz: f64) -> Self {
        Self {
            fs,
            n: 0,
            bpm,
            mains_hz,
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn noise(&mut self) -> f64 {
        // xorshift64*, mapped to [-1, 1).
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let x = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (x >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }

    /// The clean waveform in volts relative to the reference, at time `t` seconds.
    pub fn clean(t: f64, bpm: f64) -> f64 {
        let period = 60.0 / bpm;
        let ph = t % period;
        let g = |c: f64, w: f64, a: f64| a * (-((ph - c) * (ph - c)) / (2.0 * w * w)).exp();
        g(0.16, 0.025, 0.08) // P
            + g(0.28, 0.008, -0.10) // Q
            + g(0.30, 0.010, 1.00) // R
            + g(0.32, 0.008, -0.20) // S
            + g(0.52, 0.040, 0.25) // T
    }
}

impl EcgSource for Simulator {
    fn read_volts(&mut self) -> Result<f64, String> {
        let t = self.n as f64 / self.fs;
        self.n += 1;
        let hum = if self.mains_hz > 0.0 {
            0.03 * (2.0 * std::f64::consts::PI * self.mains_hz * t).sin()
        } else {
            0.0
        };
        let wander = 0.10 * (2.0 * std::f64::consts::PI * 0.25 * t).sin();
        Ok(1.5 + 0.5 * Self::clean(t, self.bpm) + hum + wander + 0.01 * self.noise())
    }

    fn describe(&self) -> String {
        format!("simulator@{}bpm", self.bpm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_word_for_a0_is_datasheet_value() {
        // MUX 100 (A0/GND), PGA 001, continuous, 860 SPS, comparator off.
        assert_eq!(config_word(0), 0x42E3);
        assert_eq!(config_word(3), 0x72E3);
    }

    #[test]
    fn conversion_is_signed_big_endian() {
        assert!((raw_to_volts(0x7F, 0xFF) - 4.095875).abs() < 1e-6);
        assert!((raw_to_volts(0x80, 0x00) + 4.096).abs() < 1e-9);
        assert!((raw_to_volts(0x2E, 0xE0) - 1.5).abs() < 1e-3);
    }

    #[test]
    fn open_rejects_bad_address_and_channel_before_touching_the_bus() {
        assert!(Ads1115::open(1, 0x20, 0)
            .err()
            .unwrap()
            .contains("not an ADS1115"));
        assert!(Ads1115::open(1, 0x48, 4)
            .err()
            .unwrap()
            .contains("out of range"));
    }
}

//! ADS1115 16-bit ADC over Linux i2c-dev (`/dev/i2c-N`) — the same proven driver the sen0213-ecg
//! cog uses, generalized to an `AdcSource`. The sound sensor's OUT wire lands on one single-ended
//! ADC input (default A1, since the ECG uses A0), read in continuous mode at 860 SPS.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;

/// `I2C_SLAVE` from linux/i2c-dev.h.
const I2C_SLAVE: libc::c_ulong = 0x0703;

const REG_CONVERSION: u8 = 0x00;
const REG_CONFIG: u8 = 0x01;

/// Volts per LSB at PGA +/-4.096 V.
pub const VOLTS_PER_LSB: f64 = 4.096 / 32768.0;

/// A source of voltage samples. The real ADC and the simulator both implement it.
pub trait AdcSource: Send {
    fn read_volts(&mut self) -> Result<f64, String>;
    fn describe(&self) -> String;
}

/// Config register value for continuous mode on `channel` (0-3, single-ended vs GND), PGA
/// +/-4.096 V, 860 SPS, comparator disabled.
pub fn config_word(channel: u8) -> u16 {
    let mux: u16 = 0b100 | (u16::from(channel) & 0b11);
    (mux << 12) | (0b001 << 9) | (0b111 << 5) | 0b11
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
        if got & 0x7FFF != want & 0x7FFF {
            return Err(format!(
                "no ADS1115 at 0x{addr:02x} on {path}: config read back 0x{got:04x}, wrote 0x{want:04x}"
            ));
        }
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

impl AdcSource for Ads1115 {
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

/// Synthetic sound sensor for testing without hardware: an active-high digital OUT that sits near
/// 0 V when quiet and bursts toward ~3.3 V a few times a second (claps / noise), plus a little
/// noise. Produces detectable events so `--simulate` exercises the whole pipeline.
pub struct SoundSim {
    fs: f64,
    n: u64,
    rng: u64,
}

impl SoundSim {
    pub fn new(fs: f64) -> Self {
        Self {
            fs,
            n: 0,
            rng: 0x1234_5678_9ABC_DEF0,
        }
    }
    fn noise(&mut self) -> f64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let x = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (x >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

impl AdcSource for SoundSim {
    fn read_volts(&mut self) -> Result<f64, String> {
        let t = self.n as f64 / self.fs;
        self.n += 1;
        // A burst every ~0.7 s lasting ~80 ms -> a few events/second.
        let ph = t % 0.7;
        let burst = if ph < 0.08 { 3.3 } else { 0.0 };
        Ok((burst + 0.05 * self.noise()).clamp(0.0, 3.3))
    }
    fn describe(&self) -> String {
        "simulator@sound".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_word_matches_datasheet() {
        assert_eq!(config_word(0), 0x42E3);
        assert_eq!(config_word(1), 0x52E3);
    }

    #[test]
    fn conversion_is_signed_big_endian() {
        assert!((raw_to_volts(0x7F, 0xFF) - 4.095875).abs() < 1e-6);
        assert!((raw_to_volts(0x80, 0x00) + 4.096).abs() < 1e-9);
    }

    #[test]
    fn open_rejects_bad_address_and_channel() {
        assert!(Ads1115::open(1, 0x20, 1)
            .err()
            .unwrap()
            .contains("not an ADS1115"));
        assert!(Ads1115::open(1, 0x48, 4)
            .err()
            .unwrap()
            .contains("out of range"));
    }

    #[test]
    fn sim_produces_bursts() {
        let mut s = SoundSim::new(1000.0);
        let mut hi = 0;
        for _ in 0..1000 {
            if s.read_volts().unwrap() > 1.5 {
                hi += 1;
            }
        }
        assert!(hi > 10, "expected some high samples, got {hi}");
    }
}

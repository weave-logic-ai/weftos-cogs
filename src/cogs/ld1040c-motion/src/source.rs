//! The OUT-line source: the module's digital motion pin, read as a GPIO input over the Linux
//! character device (`/dev/gpiochip0`), plus a hardware-free simulator. Both implement [`OutLine`],
//! so the exact same sampling loop runs in `--simulate` as against a real radar.
//!
//! The OUT pin is HIGH while motion is asserted. The module holds it HIGH for ~5 s after the last
//! trigger and blocks re-triggering for ~2 s after it falls; the simulator honours the 5 s hold.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::export::{Sample, Shared};

const TAG: &str = "[cog-ld1040c-motion]";
/// The module's output hold after the last trigger (datasheet ~5 s).
pub const HOLD_SECS: f64 = 5.0;

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// A source of the OUT logic level. Returns `true` when motion is asserted.
pub trait OutLine: Send {
    fn level(&mut self) -> Result<bool, String>;
    fn describe(&self) -> String;
}

/// A real OUT pin read over the Linux GPIO character device.
#[cfg(target_os = "linux")]
pub struct GpioLine {
    handle: gpio_cdev::LineHandle,
    desc: String,
}

#[cfg(target_os = "linux")]
impl GpioLine {
    /// `chip_path` is e.g. `/dev/gpiochip0`; `offset` is the BCM line number on that chip.
    pub fn open(chip_path: &str, offset: u32) -> Result<Self, String> {
        use gpio_cdev::{Chip, LineRequestFlags};
        let mut chip = Chip::new(chip_path).map_err(|e| format!("open {chip_path}: {e}"))?;
        let line = chip
            .get_line(offset)
            .map_err(|e| format!("get line {offset} on {chip_path}: {e}"))?;
        let handle = line
            .request(LineRequestFlags::INPUT, 0, "cog-ld1040c-motion")
            .map_err(|e| format!("request line {offset} as input: {e}"))?;
        Ok(Self {
            handle,
            desc: format!("OUT on {chip_path} line {offset}"),
        })
    }
}

#[cfg(target_os = "linux")]
impl OutLine for GpioLine {
    fn level(&mut self) -> Result<bool, String> {
        self.handle
            .get_value()
            .map(|v| v != 0)
            .map_err(|e| format!("read: {e}"))
    }
    fn describe(&self) -> String {
        self.desc.clone()
    }
}

/// GPIO lines are a Linux-only facility; on other hosts the cog compiles but has no OUT line.
#[cfg(not(target_os = "linux"))]
pub struct GpioLine;

#[cfg(not(target_os = "linux"))]
impl GpioLine {
    pub fn open(_chip_path: &str, _offset: u32) -> Result<Self, String> {
        Err("the OUT GPIO line is only available on Linux (use --simulate here)".into())
    }
}

#[cfg(not(target_os = "linux"))]
impl OutLine for GpioLine {
    fn level(&mut self) -> Result<bool, String> {
        Err("no GPIO on this host".into())
    }
    fn describe(&self) -> String {
        "no GPIO (non-Linux host)".into()
    }
}

/// A deterministic walking-person OUT pattern: bursts of motion separated by quiet gaps, with the
/// module's 5 s output hold applied so a single burst reads as one sustained HIGH.
pub struct Simulator {
    start: Instant,
}

impl Simulator {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// OUT level at `elapsed_s` seconds since start. A person walks through the beam from
    /// WALK_START to WALK_END of a 12 s cycle; the 5 s hold keeps OUT high for a moment after.
    pub fn level_at(elapsed_s: f64) -> bool {
        const CYCLE: f64 = 12.0;
        const WALK_START: f64 = 1.0;
        const WALK_END: f64 = 6.0;
        let base = (elapsed_s / CYCLE).floor() * CYCLE;
        elapsed_s >= base + WALK_START && elapsed_s < base + WALK_END + HOLD_SECS
    }
}

impl OutLine for Simulator {
    fn level(&mut self) -> Result<bool, String> {
        Ok(Self::level_at(self.start.elapsed().as_secs_f64()))
    }
    fn describe(&self) -> String {
        "simulated OUT (walking-person, 12 s cycle)".to_string()
    }
}

/// Sample the OUT line at a fixed rate and feed shared state: level, rising edges and a trace.
pub fn read_loop(mut line: Box<dyn OutLine>, shared: Shared, sample_ms: u64) {
    let mut prev = false;
    let mut errors = 0u64;
    loop {
        let t = unix_ms();
        match line.level() {
            Ok(level) => {
                if let Ok(mut st) = shared.lock() {
                    if level && !prev {
                        st.mark_event(t);
                    }
                    st.push(Sample { t_ms: t, high: level });
                    st.last_level = level;
                    st.samples_total += 1;
                }
                prev = level;
            }
            Err(e) => {
                errors += 1;
                if errors % 200 == 1 {
                    eprintln!("{TAG} OUT read error: {e}");
                }
            }
        }
        std::thread::sleep(Duration::from_millis(sample_ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulator_is_high_during_the_walk_and_hold_then_clear() {
        // Cycle 0: walk 1..6 s, hold to 11 s, clear 11..12 s.
        assert!(!Simulator::level_at(0.5));
        assert!(Simulator::level_at(2.0));
        assert!(Simulator::level_at(6.5)); // within the 5 s hold
        assert!(Simulator::level_at(10.9));
        assert!(!Simulator::level_at(11.5)); // hold expired, quiet gap
        // Cycle 1 repeats the pattern.
        assert!(Simulator::level_at(14.0));
    }
}

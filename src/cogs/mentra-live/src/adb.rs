//! ADB telemetry source for the Mentra Live glasses.
//!
//! The glasses are an Android device (the MentraOS ASG client). We never run on them; we reach them
//! from a companion over ADB — USB, or ADB-over-TCP (`adb connect ip:port`). Each poll runs two
//! short, read-only commands (`get-state` for presence + link RTT, `dumpsys battery` for the power
//! picture) and returns a [`Telemetry`] snapshot. Fail-honest: a missing `adb` binary or no device
//! is an error / an offline snapshot, never faked liveness.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How the glasses are drawing power.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plug {
    None,
    Ac,
    Usb,
    Wireless,
    Other,
}

impl Plug {
    pub fn from_code(c: i64) -> Self {
        match c {
            0 => Plug::None,
            1 => Plug::Ac,
            2 => Plug::Usb,
            4 => Plug::Wireless,
            _ => Plug::Other,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Plug::None => "none",
            Plug::Ac => "ac",
            Plug::Usb => "usb",
            Plug::Wireless => "wireless",
            Plug::Other => "other",
        }
    }
    /// 1.0 when on external power, else 0.0 — the normalized "on_power" store feature.
    pub fn on_power(self) -> f64 {
        if matches!(self, Plug::None) {
            0.0
        } else {
            1.0
        }
    }
}

/// One liveness snapshot of the glasses.
#[derive(Clone, Debug)]
pub struct Telemetry {
    pub online: bool,
    pub serial: String,
    pub model: String,
    pub fw: String,
    pub ip: Option<String>,
    pub battery_pct: Option<u8>,
    pub charging: bool,
    pub plugged: Plug,
    pub temp_c: Option<f64>,
    pub voltage_v: Option<f64>,
    pub rtt_ms: Option<f64>,
}

impl Telemetry {
    fn offline(serial: &str, model: &str) -> Self {
        Self {
            online: false,
            serial: serial.to_string(),
            model: model.to_string(),
            fw: String::new(),
            ip: None,
            battery_pct: None,
            charging: false,
            plugged: Plug::None,
            temp_c: None,
            voltage_v: None,
            rtt_ms: None,
        }
    }
}

/// A source of glasses telemetry (ADB, or the simulator) — both drive the identical report path.
pub trait TeleSource {
    fn poll(&mut self) -> Result<Telemetry, String>;
    fn describe(&self) -> String;
}

/// Resolve the `adb` binary: explicit `--adb`, then `$ADB`, then any known absolute install, then
/// bare `adb` on `$PATH`.
pub fn resolve_adb(explicit: &Option<String>) -> Result<PathBuf, String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(p) = explicit.as_ref().filter(|s| !s.is_empty()) {
        candidates.push(p.clone());
    }
    if let Ok(p) = std::env::var("ADB") {
        if !p.is_empty() {
            candidates.push(p);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(format!("{home}/Library/Android/sdk/platform-tools/adb"));
        candidates.push(format!("{home}/Android/Sdk/platform-tools/adb"));
    }
    for p in [
        "/usr/bin/adb",
        "/usr/lib/android-sdk/platform-tools/adb",
        "/opt/homebrew/bin/adb",
        "/usr/local/bin/adb",
    ] {
        candidates.push(p.to_string());
    }
    for c in &candidates {
        if c.contains('/') && std::path::Path::new(c).exists() {
            return Ok(PathBuf::from(c));
        }
    }
    // Fall back to PATH resolution; spawn will fail honestly if it is not there.
    Ok(PathBuf::from("adb"))
}

/// Run an adb command with a hard timeout, returning stdout. Outputs are small (<4 KB), so reading
/// the pipe after the process exits cannot deadlock.
fn run(adb: &PathBuf, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(adb)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn adb: {e}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("adb {} timed out", args.join(" ")));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("adb wait: {e}")),
        }
    }
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    Ok(out)
}

/// The live ADB telemetry source.
pub struct AdbSource {
    adb: PathBuf,
    target: String,
    model: String,
    fw: String,
    ip: Option<String>,
}

impl AdbSource {
    /// Connect to the glasses: `--addr` (ADB-over-TCP) wins, else `--serial`, else the first
    /// attached device. Errors only when adb is unusable or no device can be found.
    pub fn connect(
        adb: &Option<String>,
        serial: &Option<String>,
        addr: &Option<String>,
    ) -> Result<Self, String> {
        let adb = resolve_adb(adb)?;
        let timeout = Duration::from_secs(5);
        let (target, ip) = if let Some(a) = addr.as_ref().filter(|s| !s.is_empty()) {
            let _ = run(&adb, &["connect", a], timeout); // best effort; get-state verifies
            (a.clone(), a.rsplit_once(':').map(|(h, _)| h.to_string()))
        } else if let Some(s) = serial.as_ref().filter(|s| !s.is_empty()) {
            (s.clone(), None)
        } else {
            let listed = run(&adb, &["devices"], timeout)?;
            let first = listed
                .lines()
                .skip(1)
                .filter_map(|l| {
                    let mut it = l.split_whitespace();
                    match (it.next(), it.next()) {
                        (Some(id), Some("device")) => Some(id.to_string()),
                        _ => None,
                    }
                })
                .next();
            match first {
                Some(id) => (id, None),
                None => {
                    return Err(
                        "no adb device attached (plug in the glasses, or pass --addr/--serial)"
                            .into(),
                    )
                }
            }
        };
        let mut src = Self {
            adb,
            target,
            model: String::new(),
            fw: String::new(),
            ip,
        };
        // Prime the cached identity; harmless if the device is momentarily offline.
        src.refresh_identity(timeout);
        Ok(src)
    }

    fn refresh_identity(&mut self, timeout: Duration) {
        if self.model.is_empty() {
            if let Ok(m) = run(
                &self.adb,
                &["-s", &self.target, "shell", "getprop", "ro.product.model"],
                timeout,
            ) {
                let m = m.trim();
                if !m.is_empty() {
                    self.model = m.to_string();
                }
            }
        }
        if self.fw.is_empty() {
            if let Ok(f) = run(
                &self.adb,
                &[
                    "-s",
                    &self.target,
                    "shell",
                    "getprop",
                    "ro.build.display.id",
                ],
                timeout,
            ) {
                let f = f.trim();
                if !f.is_empty() {
                    self.fw = f.to_string();
                }
            }
        }
    }
}

impl TeleSource for AdbSource {
    fn poll(&mut self) -> Result<Telemetry, String> {
        let timeout = Duration::from_secs(5);
        let t0 = Instant::now();
        let state = run(&self.adb, &["-s", &self.target, "get-state"], timeout)?;
        let rtt_ms = t0.elapsed().as_secs_f64() * 1000.0;
        if state.trim() != "device" {
            return Ok(Telemetry::offline(&self.target, &self.model));
        }
        self.refresh_identity(timeout);
        let batt = run(
            &self.adb,
            &["-s", &self.target, "shell", "dumpsys", "battery"],
            timeout,
        )
        .unwrap_or_default();
        let b = parse_battery(&batt);
        Ok(Telemetry {
            online: true,
            serial: self.target.clone(),
            model: if self.model.is_empty() {
                "Mentra Live".into()
            } else {
                self.model.clone()
            },
            fw: self.fw.clone(),
            ip: self.ip.clone(),
            battery_pct: b.level,
            charging: b.charging,
            plugged: b.plugged,
            temp_c: b.temp_c,
            voltage_v: b.voltage_v,
            rtt_ms: Some(rtt_ms),
        })
    }
    fn describe(&self) -> String {
        format!("Mentra Live over adb ({})", self.target)
    }
}

/// Parsed fields from `dumpsys battery`.
pub struct Battery {
    pub level: Option<u8>,
    pub charging: bool,
    pub plugged: Plug,
    pub temp_c: Option<f64>,
    pub voltage_v: Option<f64>,
}

/// Parse `dumpsys battery`'s `key: value` lines. `status` 2/5 = charging/full; `temperature` is in
/// tenths of a degree C; `voltage` is in millivolts.
pub fn parse_battery(s: &str) -> Battery {
    let mut level = None;
    let mut scale: f64 = 100.0;
    let mut status: i64 = 0;
    let mut plugged = Plug::None;
    let mut temp_c = None;
    let mut voltage_v = None;
    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "level" => level = v.parse::<f64>().ok(),
            "scale" => {
                if let Ok(x) = v.parse::<f64>() {
                    if x > 0.0 {
                        scale = x;
                    }
                }
            }
            "status" => status = v.parse().unwrap_or(0),
            "plugged" => plugged = Plug::from_code(v.parse().unwrap_or(0)),
            "temperature" => temp_c = v.parse::<f64>().ok().map(|t| t / 10.0),
            "voltage" => voltage_v = v.parse::<f64>().ok().map(|mv| mv / 1000.0),
            _ => {}
        }
    }
    let level = level.map(|l| ((l / scale) * 100.0).round().clamp(0.0, 100.0) as u8);
    Battery {
        level,
        charging: status == 2 || status == 5,
        plugged,
        temp_c,
        voltage_v,
    }
}

/// Synthetic glasses: online, battery drains then recharges, plug toggles — so `--simulate` exercises
/// the whole report + heartbeat path with no hardware.
pub struct Sim {
    tick: u64,
}

impl Sim {
    pub fn new() -> Self {
        Self { tick: 0 }
    }
}

impl TeleSource for Sim {
    fn poll(&mut self) -> Result<Telemetry, String> {
        self.tick += 1;
        // 100 -> 20 over 80 ticks, then charge back up; charging in the bottom third of the cycle.
        let cycle = self.tick % 120;
        let charging = cycle >= 80;
        let pct = if charging {
            20 + ((cycle - 80) * 2) as u8
        } else {
            (100 - cycle) as u8
        };
        Ok(Telemetry {
            online: true,
            serial: "SIMGLASSES01".into(),
            model: "Mentra Live (sim)".into(),
            fw: "sim-1".into(),
            ip: Some("127.0.0.1".into()),
            battery_pct: Some(pct.clamp(0, 100)),
            charging,
            plugged: if charging { Plug::Usb } else { Plug::None },
            temp_c: Some(30.0 + (cycle as f64) * 0.05),
            voltage_v: Some(3.7 + (pct as f64) / 1000.0),
            rtt_ms: Some(12.0 + (cycle % 7) as f64),
        })
    }
    fn describe(&self) -> String {
        "simulated Mentra Live".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dumpsys_battery() {
        let s = "Current Battery Service state:\n  AC powered: false\n  USB powered: true\n  level: 72\n  scale: 100\n  status: 2\n  plugged: 2\n  temperature: 284\n  voltage: 4123\n  present: true\n";
        let b = parse_battery(s);
        assert_eq!(b.level, Some(72));
        assert!(b.charging);
        assert_eq!(b.plugged, Plug::Usb);
        assert_eq!(b.temp_c, Some(28.4));
        assert_eq!(b.voltage_v, Some(4.123));
    }

    #[test]
    fn non_100_scale_normalizes_level() {
        let b = parse_battery("  level: 50\n  scale: 200\n  status: 3\n  plugged: 0\n");
        assert_eq!(b.level, Some(25));
        assert!(!b.charging);
        assert_eq!(b.plugged.on_power(), 0.0);
    }

    #[test]
    fn plug_codes() {
        assert_eq!(Plug::from_code(1), Plug::Ac);
        assert_eq!(Plug::from_code(4), Plug::Wireless);
        assert_eq!(Plug::from_code(9), Plug::Other);
        assert_eq!(Plug::Ac.on_power(), 1.0);
    }

    #[test]
    fn simulator_is_bounded_and_online() {
        let mut sim = Sim::new();
        for _ in 0..300 {
            let t = sim.poll().unwrap();
            assert!(t.online);
            assert!(t.battery_pct.unwrap() <= 100);
        }
    }
}

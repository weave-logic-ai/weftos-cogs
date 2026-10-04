//! Cognitum Cog: hlk-as201
//!
//! A Hi-Link HLK-AS201 9/10-axis attitude/IMU sensor (3-axis accel + 3-axis gyro + 3-axis
//! magnetometer + barometer; 32-bit DSP) read over a 3.3 V TTL UART at 115200 8N1. It streams
//! orientation, acceleration, angular rate, magnetic field, quaternion, temperature, pressure and
//! height.
//!
//! PROTOCOL = **Hi-Link AS201, CONFIRMED against the datasheet (V1.1, 2025-08-01).** Frame:
//!   `FA FB · len · cmd · data · SUM · FC FD`   (little-endian; len = cmd..check byte count;
//!   SUM = (cmd + data bytes) & 0xFF). The sensor report is `cmd 0x00` with a 1-byte sensor-type
//!   then a 42-byte ten-axis payload: accel(6) gyro(6) euler(6) mag(6) quaternion(8) temp(2)
//!   pressure(4) height(4). Scaling per the datasheet — accel ×16/32768 g, gyro ×0.0625 °/s, euler
//!   ×180/32768 °, mag ×0.006103515625 µT, quaternion ×1/32768, temp ×0.01 °C, pressure
//!   ×0.0002384185791 Pa, height ×0.0010728836 m. Implements ADR-001.
//!
//! Usage:
//!   cog-hlk-as201 --once                     # read until one report, emit it, exit
//!   cog-hlk-as201 --interval 1               # continuous, one report per second
//!   cog-hlk-as201 --once --simulate          # synthetic AS201 frames, no hardware needed
//!   cog-hlk-as201 --port /dev/ttyUSB0        # serial device (default autodetects USB-serial)

mod export;
mod guide;
mod serial;

use export::{round, Sample, Shared, State};
use serial::Serial;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TAG: &str = "[cog-hlk-as201]";
/// Per-cog id in the store vector tuple (21=ecg, 22=tof, 23=sound, 24=radar, 25=hlk-as201 IMU).
const STORE_ID: u32 = 25;
/// Default UART baud for the AS201 report stream.
const BAUD: u32 = 115_200;
/// Angular-rate magnitude (deg/s) above which the sensor is counted as "moving" (a motion onset).
const MOTION_DEG_S: f64 = 25.0;

// Datasheet scaling coefficients (raw i16/i32 -> physical units).
const ACC_G: f64 = 16.0 / 32768.0;
const GYRO_DPS: f64 = 0.0625;
const EULER_DEG: f64 = 0.005_493_164_062_5;
const MAG_UT: f64 = 0.006_103_515_625;
const QUAT: f64 = 1.0 / 32768.0;
const TEMP_C: f64 = 0.01;
const PRESSURE_PA: f64 = 0.000_238_418_579_1;
const HEIGHT_M: f64 = 0.001_072_883_6;

struct Opts {
    once: bool,
    interval: u64,
    window: u64,
    port: String,
    baud: u32,
    motion: f64,
    api_bind: String,
    simulate: bool,
}

fn arg<'a>(a: &'a [String], f: &str) -> Option<&'a str> {
    a.iter()
        .position(|x| x == f)
        .and_then(|i| a.get(i + 1))
        .map(String::as_str)
}

fn num<T: std::str::FromStr + PartialOrd + Copy>(a: &[String], f: &str, d: T, lo: T, hi: T) -> T {
    match arg(a, f).and_then(|v| v.parse::<T>().ok()) {
        Some(v) if v >= lo && v <= hi => v,
        Some(_) => {
            eprintln!("{TAG} {f} out of range, using default");
            d
        }
        None => d,
    }
}

fn default_port() -> String {
    const CANDIDATES: [&str; 5] = [
        "/dev/ttyUSB0",
        "/dev/ttyACM0",
        "/dev/tty.usbserial-0001",
        "/dev/tty.usbserial",
        "/dev/tty.SLAB_USBtoUART",
    ];
    for c in CANDIDATES {
        if std::path::Path::new(c).exists() {
            return c.to_string();
        }
    }
    CANDIDATES[0].to_string()
}

fn parse_opts(a: &[String]) -> Opts {
    Opts {
        once: a.iter().any(|x| x == "--once"),
        interval: num(a, "--interval", 1, 1, 60),
        window: num(a, "--window", 2, 1, 30),
        port: arg(a, "--port")
            .map(str::to_string)
            .unwrap_or_else(default_port),
        baud: num(a, "--baud", BAUD, 4800, 921_600),
        motion: num(a, "--motion", MOTION_DEG_S, 1.0, 2000.0),
        api_bind: arg(a, "--api-bind").unwrap_or("0.0.0.0:8051").to_string(),
        simulate: a.iter().any(|x| x == "--simulate"),
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// One decoded AS201 sensor report (cmd 0x00).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Report {
    accel: [f64; 3], // g
    gyro: [f64; 3],  // deg/s
    euler: [f64; 3], // roll, pitch, yaw (deg)
    mag: [f64; 3],   // µT
    quat: [f64; 4],  // w, x, y, z
    temp_c: f64,
    pressure_hpa: f64,
    height_m: f64,
    mag_accuracy: u8,
}

fn i16le(b: &[u8], o: usize) -> f64 {
    f64::from(i16::from_le_bytes([b[o], b[o + 1]]))
}
fn i32le(b: &[u8], o: usize) -> f64 {
    f64::from(i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]))
}

/// Decode a sensor-report `data` field: `type(1) + 42-byte payload`.
fn decode_report(data: &[u8]) -> Option<Report> {
    if data.len() < 43 {
        return None;
    }
    let s = &data[1..43]; // the 42-byte payload
    Some(Report {
        accel: [
            i16le(s, 0) * ACC_G,
            i16le(s, 2) * ACC_G,
            i16le(s, 4) * ACC_G,
        ],
        gyro: [
            i16le(s, 6) * GYRO_DPS,
            i16le(s, 8) * GYRO_DPS,
            i16le(s, 10) * GYRO_DPS,
        ],
        euler: [
            i16le(s, 12) * EULER_DEG,
            i16le(s, 14) * EULER_DEG,
            i16le(s, 16) * EULER_DEG,
        ],
        mag: [
            i16le(s, 18) * MAG_UT,
            i16le(s, 20) * MAG_UT,
            i16le(s, 22) * MAG_UT,
        ],
        quat: [
            i16le(s, 24) * QUAT,
            i16le(s, 26) * QUAT,
            i16le(s, 28) * QUAT,
            i16le(s, 30) * QUAT,
        ],
        temp_c: i16le(s, 32) * TEMP_C,
        pressure_hpa: i32le(s, 34) * PRESSURE_PA / 100.0,
        height_m: i32le(s, 38) * HEIGHT_M,
        mag_accuracy: (data[0] >> 2) & 0x3,
    })
}

/// Drain complete, checksum-valid `FA FB .. FC FD` report frames from the accumulator, resyncing on
/// a bad frame. Only sensor reports (cmd 0x00) are returned; ACKs to config commands are skipped.
fn parse_frames(acc: &mut Vec<u8>) -> Vec<Report> {
    let mut out = Vec::new();
    loop {
        match acc.iter().position(|&b| b == 0xFA) {
            Some(0) => {}
            Some(i) => {
                acc.drain(..i);
            }
            None => {
                acc.clear();
                break;
            }
        }
        if acc.len() < 2 {
            break;
        }
        if acc[1] != 0xFB {
            acc.drain(..1); // 0xFA not followed by 0xFB — resync
            continue;
        }
        if acc.len() < 3 {
            break;
        }
        let len = acc[2] as usize;
        if !(2..=250).contains(&len) {
            acc.drain(..1);
            continue;
        }
        let total = len + 5; // FA FB len [cmd..check = len bytes] FC FD
        if acc.len() < total {
            break; // wait for the rest
        }
        let tail_ok = acc[len + 3] == 0xFC && acc[len + 4] == 0xFD;
        let sum = acc[3..len + 2].iter().fold(0u32, |s, &b| s + u32::from(b)) & 0xFF;
        if tail_ok && sum as u8 == acc[len + 2] {
            if acc[3] == 0x00 {
                if let Some(r) = decode_report(&acc[4..len + 2]) {
                    out.push(r);
                }
            }
            acc.drain(..total);
        } else {
            acc.drain(..1); // bad tail/checksum — drop this header and resync
        }
    }
    out
}

trait ByteSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn describe(&self) -> String;
}

struct SerialSource {
    dev: Serial,
    desc: String,
}

impl ByteSource for SerialSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.dev.read(buf)
    }
    fn describe(&self) -> String {
        self.desc.clone()
    }
}

/// Build an AS201 frame: `FA FB len cmd <data> SUM FC FD`.
fn build_frame(cmd: u8, data: &[u8]) -> Vec<u8> {
    let len = (data.len() + 2) as u8; // cmd + data + check
    let check = (u32::from(cmd) + data.iter().map(|&b| u32::from(b)).sum::<u32>()) as u8;
    let mut f = vec![0xFA, 0xFB, len, cmd];
    f.extend_from_slice(data);
    f.push(check);
    f.extend_from_slice(&[0xFC, 0xFD]);
    f
}

/// Emits synthetic AS201 report frames at ~10 Hz: gravity on Z, a slow tumble, an intermittent
/// shake, ~sea-level pressure. Paces itself. Exercises the real frame parser.
struct ImuSim {
    next: Instant,
    phase: f64,
}

impl ImuSim {
    fn new() -> Self {
        Self {
            next: Instant::now(),
            phase: 0.0,
        }
    }
}

impl ByteSource for ImuSim {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let now = Instant::now();
        if now < self.next {
            std::thread::sleep(self.next - now);
        }
        self.next += Duration::from_millis(100);
        self.phase += 0.1;
        let g = |x: f64| (x / ACC_G).round().clamp(-32768.0, 32767.0) as i16;
        let dps = |x: f64| (x / GYRO_DPS).round().clamp(-32768.0, 32767.0) as i16;
        let deg = |x: f64| (x / EULER_DEG).round().clamp(-32768.0, 32767.0) as i16;
        let ut = |x: f64| (x / MAG_UT).round().clamp(-32768.0, 32767.0) as i16;
        let shaking = self.phase.sin() > 0.6;
        let rate = if shaking { 90.0 } else { 3.0 };
        let mut d: Vec<u8> = Vec::with_capacity(43);
        d.push(0x00); // sensor type: ten axes, mag accuracy 0
        let mut push16 = |raw: i16| d.extend_from_slice(&raw.to_le_bytes());
        push16(g(0.02 * self.phase.cos()));
        push16(g(0.02 * self.phase.sin()));
        push16(g(1.0)); // gravity on Z
        push16(dps(rate * self.phase.sin()));
        push16(dps(rate * self.phase.cos()));
        push16(dps(rate * 0.5));
        push16(deg(30.0 * self.phase.sin()));
        push16(deg(15.0 * self.phase.cos()));
        push16(deg((self.phase * 20.0) % 180.0));
        push16(ut(12.0));
        push16(ut(-8.0));
        push16(ut(40.0));
        push16((QUAT.recip()) as i16); // quaternion ~identity (w=1)
        push16(0);
        push16(0);
        push16(0);
        push16((25.0 / TEMP_C) as i16); // 25.0 °C
        d.extend_from_slice(&((101_325.0 / PRESSURE_PA) as i32).to_le_bytes()); // ~sea level
        d.extend_from_slice(&0i32.to_le_bytes()); // height 0
        let frame = build_frame(0x00, &d);
        let n = frame.len().min(buf.len());
        buf[..n].copy_from_slice(&frame[..n]);
        Ok(n)
    }
    fn describe(&self) -> String {
        "simulated HLK-AS201 (Hi-Link protocol, 10 Hz)".to_string()
    }
}

fn gyro_mag(g: &[f64; 3]) -> f64 {
    (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt()
}

/// Read bytes, parse AS201 reports, and keep shared state current: latest values, a motion trace,
/// and motion onsets.
fn read_loop(mut src: Box<dyn ByteSource + Send>, motion_deg_s: f64, shared: Shared) {
    let mut acc: Vec<u8> = Vec::with_capacity(64);
    let mut buf = [0u8; 256];
    let mut errors = 0u64;
    let mut moving = false;
    loop {
        let n = match src.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(e) => {
                errors += 1;
                if errors.is_multiple_of(200) {
                    eprintln!("{TAG} read error: {e}");
                }
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
        };
        acc.extend_from_slice(&buf[..n]);
        if acc.len() > 4096 {
            acc.drain(..acc.len() - 4096);
        }
        let reports = parse_frames(&mut acc);
        if reports.is_empty() {
            continue;
        }
        let t_ms = unix_ms();
        if let Ok(mut st) = shared.lock() {
            for r in reports {
                st.packets += 1;
                st.accel = r.accel;
                st.gyro = r.gyro;
                st.euler = r.euler;
                st.mag = r.mag;
                st.quat = r.quat;
                st.temp_c = r.temp_c;
                st.pressure_hpa = r.pressure_hpa;
                st.height_m = r.height_m;
                st.mag_accuracy = r.mag_accuracy;
                let mag = gyro_mag(&r.gyro);
                let now_moving = mag >= motion_deg_s;
                if now_moving && !moving {
                    st.mark_event(t_ms);
                }
                moving = now_moving;
                st.push(Sample { t_ms, v: mag });
            }
        }
    }
}

fn build_report(
    st: &State,
    window_s: f64,
    motion_deg_s: f64,
    export_url: &Option<String>,
) -> (serde_json::Value, Vec<f64>) {
    let now_ms = unix_ms();
    let win = st.tail(window_s);
    let gmag = gyro_mag(&st.gyro);
    let moving = gmag >= motion_deg_s;
    let events_60s = st.events_in(60, now_ms);
    let quiet_s = st
        .last_event_ms
        .map(|e| (now_ms.saturating_sub(e)) as f64 / 1000.0);
    let status = if st.packets == 0 {
        "no_frames"
    } else if moving {
        "moving"
    } else {
        "still"
    };
    let r3 = |a: &[f64; 3]| [round(a[0], 3), round(a[1], 3), round(a[2], 3)];

    let report = serde_json::json!({
        "status": status,
        "source": st.source,
        "moving": moving,
        "accel_g": {"x": r3(&st.accel)[0], "y": r3(&st.accel)[1], "z": r3(&st.accel)[2]},
        "gyro_dps": {"x": r3(&st.gyro)[0], "y": r3(&st.gyro)[1], "z": r3(&st.gyro)[2]},
        "euler_deg": {"roll": r3(&st.euler)[0], "pitch": r3(&st.euler)[1], "yaw": r3(&st.euler)[2]},
        "mag_ut": {"x": r3(&st.mag)[0], "y": r3(&st.mag)[1], "z": r3(&st.mag)[2]},
        "quaternion": {"w": round(st.quat[0], 4), "x": round(st.quat[1], 4), "y": round(st.quat[2], 4), "z": round(st.quat[3], 4)},
        "temp_c": round(st.temp_c, 1),
        "pressure_hpa": round(st.pressure_hpa, 2),
        "height_m": round(st.height_m, 2),
        "mag_accuracy": st.mag_accuracy,
        "gyro_mag_dps": round(gmag, 2),
        "motion_events_per_min": events_60s,
        "total_motion_events": st.total_events,
        "quiet_s": quiet_s.map(|q| round(q, 1)),
        "packets": st.packets,
        "frame_protocol": "HLK-AS201 Hi-Link protocol (confirmed, datasheet V1.1 2025-08-01)",
        "window_s": window_s,
        "samples": win.len(),
        "export": export_url,
        "timestamp": now_ms / 1000,
    });

    let norm_angle = |d: f64| (d / 180.0 + 1.0) / 2.0;
    let norm_g = |x: f64| (x / 16.0 + 1.0) / 2.0;
    let vector: Vec<f64> = [
        norm_angle(st.euler[0]),
        norm_angle(st.euler[1]),
        norm_angle(st.euler[2]),
        norm_g(st.accel[0]),
        norm_g(st.accel[1]),
        norm_g(st.accel[2]),
        gmag / 2000.0,
        if moving { 1.0 } else { 0.0 },
    ]
    .into_iter()
    .map(|v| v.clamp(0.0, 1.0))
    .collect();
    (report, vector)
}

fn store_to_seed(vector: &[f64]) -> Result<(), String> {
    let payload = serde_json::json!({ "vectors": [[STORE_ID, vector]], "dedup": true });
    let body = serde_json::to_vec(&payload).map_err(|e| format!("json: {e}"))?;
    let mut conn =
        std::net::TcpStream::connect("127.0.0.1:80").map_err(|e| format!("connect: {e}"))?;
    conn.set_read_timeout(Some(Duration::from_secs(5))).ok();
    conn.set_write_timeout(Some(Duration::from_secs(5))).ok();
    write!(conn, "POST /api/v1/store/ingest HTTP/1.0\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).map_err(|e| format!("write: {e}"))?;
    conn.write_all(&body).map_err(|e| format!("body: {e}"))?;
    let mut resp = Vec::new();
    let mut buf = [0u8; 1024];
    while !response_complete(&resp) {
        match conn.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => resp.extend_from_slice(&buf[..n]),
        }
    }
    let status = String::from_utf8_lossy(&resp)
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    if status.starts_with('2') {
        Ok(())
    } else {
        Err(format!("ingest replied {status:?}"))
    }
}

fn response_complete(resp: &[u8]) -> bool {
    let Some(end) = resp.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&resp[..end]).to_ascii_lowercase();
    let len = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    resp.len() >= end + 4 + len
}

fn open_source(o: &Opts) -> Result<Box<dyn ByteSource + Send>, String> {
    if o.simulate {
        return Ok(Box::new(ImuSim::new()));
    }
    let dev = Serial::open(&o.port, o.baud)?;
    Ok(Box::new(SerialSource {
        dev,
        desc: format!("HLK-AS201 on {} @ {} 8N1 (Hi-Link)", o.port, o.baud),
    }))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    cog_sensor_sources::handle_help(
        &args,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        include_str!("../cog.toml"),
    );
    let o = parse_opts(&args);
    eprintln!("{TAG} start (once={}, interval={}s, window={}s, port={}, baud={}, motion={}deg/s, simulate={})", o.once, o.interval, o.window, o.port, o.baud, o.motion, o.simulate);

    let shared: Shared = Arc::new(Mutex::new(State::new(f64::from(o.baud), "none".into())));
    let export_url = if o.once && !o.simulate && open_source(&o).is_err() {
        None
    } else {
        match export::serve(&o.api_bind, shared.clone()) {
            Ok(addr) => Some(format!("http://{addr}/raw")),
            Err(e) => {
                eprintln!("{TAG} export disabled: {e}");
                None
            }
        }
    };

    let src = loop {
        match open_source(&o) {
            Ok(s) => break s,
            Err(e) => {
                let r =
                    serde_json::json!({"status":"no_source","error":e,"timestamp":unix_ms()/1000});
                println!("{r}");
                eprintln!("{TAG} no source: {e}");
                if let Ok(mut st) = shared.lock() {
                    st.latest_report = Some(r);
                }
                if o.once {
                    return;
                }
                std::thread::sleep(Duration::from_secs(o.interval.clamp(1, 5)));
            }
        }
    };
    if let Ok(mut st) = shared.lock() {
        st.source = src.describe();
    }
    std::thread::spawn({
        let (shared, motion) = (shared.clone(), o.motion);
        move || read_loop(src, motion, shared)
    });

    let period = if o.once { o.window } else { o.interval };
    loop {
        std::thread::sleep(Duration::from_secs(period));
        let (report, vector) = match shared.lock() {
            Ok(mut st) => {
                let out = build_report(&st, period as f64, o.motion, &export_url);
                st.latest_report = Some(out.0.clone());
                out
            }
            Err(_) => {
                eprintln!("{TAG} state poisoned");
                return;
            }
        };
        println!("{report}");
        if report["status"] != "no_frames" {
            if let Err(e) = store_to_seed(&vector) {
                eprintln!("{TAG} store error: {e}");
            }
        }
        if o.once {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        std::iter::once("cog")
            .chain(v.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn options_bounded_and_defaulted() {
        let o = parse_opts(&s(&["--once", "--baud", "230400", "--motion", "50"]));
        assert!(o.once);
        assert_eq!(o.baud, 230_400);
        assert_eq!(o.motion, 50.0);
        assert_eq!(parse_opts(&s(&[])).api_bind, "0.0.0.0:8051");
    }

    #[test]
    fn build_frame_matches_datasheet_example() {
        // Datasheet: "Send: FA FB 03 10 01 11 FC FD" (get version, cmd 0x10, data 0x01, SUM 0x11).
        assert_eq!(
            build_frame(0x10, &[0x01]),
            vec![0xFA, 0xFB, 0x03, 0x10, 0x01, 0x11, 0xFC, 0xFD]
        );
    }

    #[test]
    fn decodes_a_report_frame_with_gravity_on_z() {
        // Build a report with az = 1 g, temp 25 C, rest zero, and parse it back.
        let g = |x: f64| ((x / ACC_G).round()) as i16;
        let mut d = vec![0x00u8];
        for raw in [g(0.0), g(0.0), g(1.0)] {
            d.extend_from_slice(&raw.to_le_bytes());
        }
        while d.len() < 1 + 32 {
            d.push(0); // gyro + euler + mag + quaternion zeros
        }
        d.extend_from_slice(&((25.0 / TEMP_C) as i16).to_le_bytes()); // temp
        d.extend_from_slice(&0i32.to_le_bytes()); // pressure
        d.extend_from_slice(&0i32.to_le_bytes()); // height
        assert_eq!(d.len(), 43);
        let mut acc = build_frame(0x00, &d);
        let r = parse_frames(&mut acc);
        assert_eq!(r.len(), 1, "expected one report");
        assert!((r[0].accel[2] - 1.0).abs() < 1e-3, "az={}", r[0].accel[2]);
        assert!((r[0].temp_c - 25.0).abs() < 0.05, "temp={}", r[0].temp_c);
        assert!(acc.is_empty());
    }

    #[test]
    fn resyncs_past_garbage_and_bad_checksum() {
        let good = {
            let mut d = vec![0x00u8];
            d.resize(43, 0);
            build_frame(0x00, &d)
        };
        let mut acc = vec![0x11, 0x22, 0xFA, 0xFB, 0x03, 0x10, 0x01, 0xEE, 0xFC, 0xFD]; // bad SUM
        acc.extend_from_slice(&good);
        let r = parse_frames(&mut acc);
        assert_eq!(r.len(), 1, "should recover the one valid report");
    }

    #[test]
    fn simulator_feeds_the_real_parser_and_vector_is_bounded() {
        let mut sim = ImuSim::new();
        let mut acc = Vec::new();
        let mut buf = [0u8; 128];
        let mut st = State::new(115_200.0, "sim".into());
        for _ in 0..30 {
            let n = sim.read(&mut buf).unwrap();
            acc.extend_from_slice(&buf[..n]);
            for r in parse_frames(&mut acc) {
                st.packets += 1;
                st.accel = r.accel;
                st.euler = r.euler;
                st.gyro = r.gyro;
                st.pressure_hpa = r.pressure_hpa;
            }
        }
        assert!(st.packets >= 20, "expected reports, got {}", st.packets);
        assert!(
            (st.accel[2] - 1.0).abs() < 0.1,
            "gravity on Z expected, az={}",
            st.accel[2]
        );
        assert!(
            (st.pressure_hpa - 1013.25).abs() < 2.0,
            "~sea-level pressure, got {}",
            st.pressure_hpa
        );
        let (_r, v) = build_report(&st, 2.0, MOTION_DEG_S, &None);
        assert_eq!(v.len(), 8);
        assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
    }
}

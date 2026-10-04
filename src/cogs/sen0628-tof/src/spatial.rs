//! `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107 §7, 2026-10-03 amendment), built
//! only with the cargo feature `spatial-evidence` and active only when `--spatial-out` is set.
//!
//! Depth frames become `tof_depth` lines in `room_enu`, at most `--spatial-max-hz` per second
//! (default 1: walls and furniture do not need 10 Hz). Each line carries the sensor's mount
//! pose, the 60°×60° field of view, the grid, every zone's range and a validity flag. The wire
//! shape is mirrored with local serde structs, as `ruview-spatial-evidence` does, so the cog
//! never depends on the WeftOS consumer.
//!
//! Zone order. ADR-107 wants row 0 at the top and column 0 on the left, both looking out
//! along the boresight. The cog's frames are row-major with X left->right and Y top->bottom
//! as seen looking out of the lens (DFRobot wiki, guide "Mounting"), which is the same order,
//! so zones are copied across. Which edge of the board is "top" is not verified on hardware:
//! a board turned upside down is `roll_deg` 180, and a mirrored image is `x_sign` -1, which
//! reverses each row.

use serde::Serialize;

use crate::scene;

pub const SCHEMA_V1: &str = "spatial.evidence.v1";
pub const PRODUCER: &str = concat!("sen0628-tof@", env!("CARGO_PKG_VERSION"));
const MAX_COORD_M: f64 = 1_000.0;
const MAX_ID_LEN: usize = 128;
/// Default 1-sigma range uncertainty. An assumption, not a measurement.
pub const DEFAULT_UNCERTAINTY_M: f64 = 0.05;
/// Full field of view `[horizontal, vertical]`, degrees: the VL53L7CX's 60°×60° square view
/// (ADR-159; DFRobot SEN0628 product page "60° field of view").
pub const FOV_DEG: [f64; 2] = [60.0, 60.0];
/// Default and largest line rate.
pub const DEFAULT_MAX_HZ: f64 = 1.0;
const MAX_HZ: f64 = 15.0;

/// Where the sensor is in the room and how it is turned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Aperture position in `room_enu`, metres (east, north, height).
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Boresight bearing in degrees counter-clockwise from room +x (east): 0 faces +x,
    /// 90 faces +y (north). The project's one yaw convention (ENU math yaw).
    pub yaw_deg: f64,
    /// Boresight elevation above horizontal, degrees (negative tilts down).
    pub pitch_deg: f64,
    /// Rotation about the boresight, degrees, positive clockwise looking out.
    pub roll_deg: f64,
    /// +1 when the frame's column 0 is on the left looking out (the documented order),
    /// -1 when the image is mirrored.
    pub x_sign: f64,
}

impl Pose {
    /// Parse `x,y,z,yaw_deg,pitch_deg[,roll_deg[,x_sign]]` (roll 0 and x_sign 1 when omitted).
    pub fn parse(s: &str) -> Result<Pose, String> {
        let v: Vec<f64> = s
            .split(',')
            .map(|p| p.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .map_err(|_| {
                format!("--tof-pose must be x,y,z,yaw_deg,pitch_deg[,roll_deg[,x_sign]], got {s:?}")
            })?;
        let (x, y, z, yaw_deg, pitch_deg, roll_deg, x_sign) = match v[..] {
            [x, y, z, yaw, pitch] => (x, y, z, yaw, pitch, 0.0, 1.0),
            [x, y, z, yaw, pitch, roll] => (x, y, z, yaw, pitch, roll, 1.0),
            [x, y, z, yaw, pitch, roll, sign] => (x, y, z, yaw, pitch, roll, sign),
            _ => return Err(format!("--tof-pose needs 5 to 7 numbers, got {}", v.len())),
        };
        if ![x, y, z]
            .iter()
            .all(|c| c.is_finite() && c.abs() <= MAX_COORD_M)
        {
            return Err("--tof-pose: x, y, z within +-1000 m".into());
        }
        let within = |v: f64, lim: f64| v.is_finite() && (-lim..=lim).contains(&v);
        if !within(yaw_deg, 360.0) {
            return Err("--tof-pose: yaw_deg must be within -360..=360".into());
        }
        if !within(pitch_deg, 90.0) {
            return Err("--tof-pose: pitch_deg must be within -90..=90".into());
        }
        if !within(roll_deg, 180.0) {
            return Err("--tof-pose: roll_deg must be within -180..=180".into());
        }
        if x_sign != 1.0 && x_sign != -1.0 {
            return Err("--tof-pose: x_sign must be 1 or -1".into());
        }
        Ok(Pose {
            x,
            y,
            z,
            yaw_deg,
            pitch_deg,
            roll_deg,
            x_sign,
        })
    }

    /// Unit vector of zone (row, col)'s ray in `room_enu`, by ADR-107's ray rule: azimuth
    /// `(col + ½)/cols · h − h/2` (positive right), elevation `v/2 − (row + ½)/rows · v`
    /// (positive up), in the mount frame turned by yaw, pitch and roll. Used by tests to
    /// pin the zone order against the engine's convention.
    #[cfg(test)]
    pub fn zone_dir(&self, row: usize, col: usize, side: usize) -> [f64; 3] {
        let n = side as f64;
        let az = ((col as f64 + 0.5) / n * FOV_DEG[0] - FOV_DEG[0] / 2.0).to_radians();
        let el = (FOV_DEG[1] / 2.0 - (row as f64 + 0.5) / n * FOV_DEG[1]).to_radians();
        let (sy, cy) = self.yaw_deg.to_radians().sin_cos();
        let (sp, cp) = self.pitch_deg.to_radians().sin_cos();
        let (sr, cr) = self.roll_deg.to_radians().sin_cos();
        let fwd = [cp * cy, cp * sy, sp];
        let right0 = [sy, -cy, 0.0];
        let up0 = [-sp * cy, -sp * sy, cp];
        // Roll clockwise looking out: the right edge goes down.
        let right: [f64; 3] = std::array::from_fn(|i| cr * right0[i] - sr * up0[i]);
        let up: [f64; 3] = std::array::from_fn(|i| sr * right0[i] + cr * up0[i]);
        let (se, ce) = el.sin_cos();
        let (sa, ca) = az.sin_cos();
        std::array::from_fn(|i| fwd[i] * ce * ca + right[i] * ce * sa + up[i] * se)
    }
}

/// Where lines go.
#[derive(Debug, Clone, PartialEq)]
pub enum Sink {
    /// Append JSONL to this file.
    File(String),
    /// Serve the last 30 s at `GET /spatial` on the cog's export.
    Export,
}

/// Everything `--spatial-*` configures.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub sink: Sink,
    pub pose: Option<Pose>,
    pub region: Option<String>,
    pub source_id: String,
    pub uncertainty_m: f64,
    pub max_hz: f64,
}

impl Settings {
    /// Why emission is refused, if it is. The cog keeps running without it.
    pub fn refusal(&self) -> Option<String> {
        if self.pose.is_none() {
            return Some(
                "no sensor pose: set --tof-pose x,y,z,yaw_deg,pitch_deg[,roll_deg[,x_sign]] \
                 (room_enu metres, yaw counter-clockwise from +x (east), pitch up-positive, \
                 roll clockwise looking out, x_sign -1 for a mirrored image)"
                    .into(),
            );
        }
        if self.region.is_none() {
            return Some("no region: set --spatial-region region/urth/meso/<room>".into());
        }
        None
    }

    /// The report's `spatial` status for a refusal: `no_pose` or `no_region`.
    pub fn refusal_status(&self) -> Option<&'static str> {
        if self.pose.is_none() {
            Some("no_pose")
        } else if self.region.is_none() {
            Some("no_region")
        } else {
            None
        }
    }
}

/// Collects the `--spatial-*` flags from the argument list.
#[derive(Debug, Default)]
pub struct Args {
    sink: Option<Sink>,
    pose: Option<Pose>,
    region: Option<String>,
    source_id: Option<String>,
    uncertainty_m: Option<f64>,
    max_hz: Option<f64>,
}

impl Args {
    pub const FLAGS: [&'static str; 6] = [
        "--spatial-out",
        "--tof-pose",
        "--spatial-region",
        "--spatial-source-id",
        "--spatial-uncertainty-m",
        "--spatial-max-hz",
    ];

    /// Collect every spatial flag and its value from `args`.
    pub fn from_args(args: &[String]) -> Result<Args, String> {
        let mut a = Args::default();
        let mut it = args.iter();
        while let Some(f) = it.next() {
            if Self::FLAGS.contains(&f.as_str()) {
                let v = it.next().ok_or(format!("{f} needs a value"))?;
                a.set(f, v)?;
            }
        }
        Ok(a)
    }

    pub fn set(&mut self, flag: &str, v: &str) -> Result<(), String> {
        let bounded = |lo: f64, hi: f64| {
            v.parse::<f64>()
                .ok()
                .filter(|u| *u > lo && *u <= hi)
                .ok_or(format!("{flag} must be in ({lo}, {hi}], got {v:?}"))
        };
        match flag {
            "--spatial-out" => {
                self.sink = Some(match v {
                    "export" => Sink::Export,
                    p if valid_path(p) => Sink::File(p.to_string()),
                    _ => {
                        return Err(format!(
                            "--spatial-out must be export or a file path, got {v:?}"
                        ))
                    }
                })
            }
            "--tof-pose" => self.pose = Some(Pose::parse(v)?),
            "--spatial-region" | "--spatial-source-id" => {
                if !valid_id(v) {
                    return Err(format!(
                        "{flag} must match [A-Za-z0-9._:/-@]{{1,128}}, got {v:?}"
                    ));
                }
                if flag == "--spatial-region" {
                    self.region = Some(v.to_string());
                } else {
                    self.source_id = Some(v.to_string());
                }
            }
            "--spatial-uncertainty-m" => self.uncertainty_m = Some(bounded(0.0, 100.0)?),
            "--spatial-max-hz" => self.max_hz = Some(bounded(0.0, MAX_HZ)?),
            _ => return Err(format!("unknown spatial flag {flag:?}")),
        }
        Ok(())
    }

    /// `None` unless `--spatial-out` was given. Pose and region may still be missing:
    /// that is a runtime refusal ([`Settings::refusal`]), not a parse error.
    pub fn finish(self) -> Result<Option<Settings>, String> {
        let Some(sink) = self.sink else {
            let other = self.pose.is_some()
                || self.region.is_some()
                || self.source_id.is_some()
                || self.uncertainty_m.is_some()
                || self.max_hz.is_some();
            return if other {
                Err("--tof-pose and --spatial-* need --spatial-out export|FILE".into())
            } else {
                Ok(None)
            };
        };
        Ok(Some(Settings {
            sink,
            pose: self.pose,
            region: self.region,
            source_id: self.source_id.unwrap_or_else(|| "sen0628-tof".into()),
            uncertainty_m: self.uncertainty_m.unwrap_or(DEFAULT_UNCERTAINTY_M),
            max_hz: self.max_hz.unwrap_or(DEFAULT_MAX_HZ),
        }))
    }
}

pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/-@".contains(&b))
}

/// A file sink is a plain path: never a device, no traversal.
pub fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 4096
        && !["/dev/", "/proc/", "/sys/"]
            .iter()
            .any(|d| p.starts_with(d))
        && !p.split('/').any(|c| c == "..")
        && !p.contains('\0')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Proof {
    Measured,
    Synthetic,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Provenance {
    pub receipt: String,
    pub producer: &'static str,
    pub proof: Proof,
}

/// One `tof_depth` line, fields in ADR-107's example order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TofDepth {
    pub schema: &'static str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub t_ns: u64,
    pub frame: &'static str,
    pub region: String,
    pub source_id: String,
    pub uncertainty_m: f64,
    pub provenance: Provenance,
    pub position: [f64; 3],
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roll_deg: Option<f64>,
    pub fov_deg: [f64; 2],
    /// `[columns, rows]`.
    pub grid: [u32; 2],
    pub range_mm: Vec<u32>,
    pub valid: Vec<bool>,
}

/// Millimetre rounding keeps float artefacts out of the JSON.
fn mm(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Turns depth frames into lines. Holds the receipt counter and the rate limit.
pub struct Emitter {
    settings: Settings,
    pose: Pose,
    region: String,
    proof: Proof,
    max_mm: u16,
    seq: u64,
    last_ms: Option<u64>,
}

impl Emitter {
    /// The refusal reason when emission is refused. `max_mm` is the cog's
    /// `--max-range-mm`: zones outside 20..=max_mm are sent with `valid: false`.
    pub fn new(settings: Settings, simulated: bool, max_mm: u16) -> Result<Emitter, String> {
        if let Some(why) = settings.refusal() {
            return Err(why);
        }
        let (Some(pose), Some(region)) = (settings.pose, settings.region.clone()) else {
            unreachable!("refusal() checked both");
        };
        Ok(Emitter {
            settings,
            pose,
            region,
            proof: if simulated {
                Proof::Synthetic
            } else {
                Proof::Measured
            },
            max_mm,
            seq: 0,
            last_ms: None,
        })
    }

    pub fn sink(&self) -> &Sink {
        &self.settings.sink
    }

    /// One line for a `side`×`side` frame read at `t_ms`, or `None` when the rate limit
    /// holds it back or the frame is not a square grid of 1..=16.
    pub fn record(&mut self, t_ms: u64, side: usize, frame: &[u16]) -> Option<TofDepth> {
        if !(1..=16).contains(&side) || frame.len() != side * side {
            return None;
        }
        let min_gap_ms = (1000.0 / self.settings.max_hz).round() as u64;
        if self
            .last_ms
            .is_some_and(|l| t_ms.saturating_sub(l) < min_gap_ms)
        {
            return None;
        }
        self.last_ms = Some(t_ms);
        let mut range_mm = Vec::with_capacity(frame.len());
        let mut valid = Vec::with_capacity(frame.len());
        for row in frame.chunks_exact(side) {
            let mut push = |d: u16| {
                range_mm.push(u32::from(d));
                valid.push(scene::valid(d, self.max_mm));
            };
            if self.pose.x_sign < 0.0 {
                row.iter().rev().copied().for_each(&mut push);
            } else {
                row.iter().copied().for_each(&mut push);
            }
        }
        self.seq += 1;
        let p = self.pose;
        Some(TofDepth {
            schema: SCHEMA_V1,
            kind: "tof_depth",
            t_ns: t_ms.saturating_mul(1_000_000),
            frame: "room_enu",
            region: self.region.clone(),
            source_id: self.settings.source_id.clone(),
            uncertainty_m: self.settings.uncertainty_m,
            provenance: Provenance {
                receipt: format!("{}:{:06}", self.settings.source_id, self.seq),
                producer: PRODUCER,
                proof: self.proof,
            },
            position: [mm(p.x), mm(p.y), mm(p.z)],
            yaw_deg: p.yaw_deg,
            pitch_deg: p.pitch_deg,
            roll_deg: (p.roll_deg != 0.0).then_some(p.roll_deg),
            fov_deg: FOV_DEG,
            grid: [side as u32, side as u32],
            range_mm,
            valid,
        })
    }
}

/// Append lines to the JSONL file.
pub fn append(path: &str, lines: &[String]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut buf = String::new();
    for l in lines {
        buf.push_str(l);
        buf.push('\n');
    }
    f.write_all(buf.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(yaw: f64, pitch: f64) -> Pose {
        Pose {
            x: 4.80,
            y: 0.05,
            z: 1.60,
            yaw_deg: yaw,
            pitch_deg: pitch,
            roll_deg: 0.0,
            x_sign: 1.0,
        }
    }

    fn settings(pose: Option<Pose>) -> Settings {
        Settings {
            sink: Sink::Export,
            pose,
            region: Some("region/urth/meso/test-room".into()),
            source_id: "sen0628-1".into(),
            uncertainty_m: 0.02,
            max_hz: 1.0,
        }
    }

    const ADR_EXAMPLE: &str = "{\"schema\":\"spatial.evidence.v1\",\"type\":\"tof_depth\",\"t_ns\":1759500004000000000,\"frame\":\"room_enu\",\"region\":\"region/urth/meso/test-room\",\"source_id\":\"sen0628-1\",\"uncertainty_m\":0.02,\"provenance\":{\"receipt\":\"sen0628-1:000042\",\"producer\":\"sen0628-tof@0.1.0\",\"proof\":\"MEASURED\"},\"position\":[4.80,0.05,1.60],\"yaw_deg\":90.0,\"pitch_deg\":-35.0,\"fov_deg\":[60.0,60.0],\"grid\":[4,4],\"range_mm\":[0,0,0,0,3290,3105,3110,3302,2512,1190,1185,2530,2005,1072,1066,2011],\"valid\":[false,false,false,false,true,true,true,true,true,true,true,true,true,true,true,true]}";

    #[test]
    fn golden_line_is_the_adr_107_example() {
        // ADR-107 §7.1's tof_depth example is a SEN0628 4x4 frame; the top row has no return.
        let mut e = Emitter::new(settings(Some(pose(90.0, -35.0))), false, 3500).unwrap();
        e.seq = 41;
        let frame = [
            0, 0, 0, 0, 3290, 3105, 3110, 3302, 2512, 1190, 1185, 2530, 2005, 1072, 1066, 2011,
        ];
        let r = e.record(1_759_500_004_000, 4, &frame).unwrap();
        let line = serde_json::to_string(&r).unwrap();
        let ours: serde_json::Value = serde_json::from_str(&line).unwrap();
        let want: serde_json::Value = serde_json::from_str(ADR_EXAMPLE).unwrap();
        assert_eq!(ours, want);
        let keys =
            |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&ours), keys(&want), "key order");
        assert!(line.len() < 16 * 1024);
        assert!(valid_id(&r.provenance.receipt) && valid_id(PRODUCER));
    }

    #[test]
    fn a_full_8x8_frame_fits_one_line() {
        let mut e = Emitter::new(settings(Some(pose(0.0, 0.0))), false, 3500).unwrap();
        let r = e.record(1, 8, &[3500; 64]).unwrap();
        assert_eq!((r.grid, r.range_mm.len(), r.valid.len()), ([8, 8], 64, 64));
        assert!(serde_json::to_string(&r).unwrap().len() < 16 * 1024);
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn yaw_0_faces_plus_x_and_yaw_90_faces_plus_y() {
        // The centre of the view (between the four middle zones) is the boresight.
        let c = |p: Pose| -> [f64; 3] {
            let d: Vec<[f64; 3]> = [(1, 1), (1, 2), (2, 1), (2, 2)]
                .iter()
                .map(|&(r, c)| p.zone_dir(r, c, 4))
                .collect();
            std::array::from_fn(|i| d.iter().map(|v| v[i]).sum::<f64>() / 4.0)
        };
        let n = |v: [f64; 3]| {
            let l = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            v.map(|x| x / l)
        };
        assert!(close(n(c(pose(0.0, 0.0))), [1.0, 0.0, 0.0]));
        assert!(close(n(c(pose(90.0, 0.0))), [0.0, 1.0, 0.0]));
    }

    #[test]
    fn zone_order_is_top_row_first_left_column_first() {
        // Facing +y (north) level: the robot's right is +x (east), up is +z.
        let p = pose(90.0, 0.0);
        let top_left = p.zone_dir(0, 0, 8);
        let bottom_right = p.zone_dir(7, 7, 8);
        assert!(top_left[0] < 0.0 && top_left[2] > 0.0, "{top_left:?}");
        assert!(
            bottom_right[0] > 0.0 && bottom_right[2] < 0.0,
            "{bottom_right:?}"
        );
        // Pitching down 35 degrees tips every ray down.
        assert!(pose(90.0, -35.0).zone_dir(0, 0, 4)[2] < top_left[2]);
        // Roll 180 swaps top-left and bottom-right.
        let mut r = p;
        r.roll_deg = 180.0;
        assert!(close(r.zone_dir(0, 0, 8), bottom_right));
    }

    #[test]
    fn mirrored_image_reverses_each_row() {
        let mut p = pose(0.0, 0.0);
        p.x_sign = -1.0;
        let mut e = Emitter::new(settings(Some(p)), false, 3500).unwrap();
        let frame: Vec<u16> = (1..=16).map(|i| i * 100).collect();
        let r = e.record(1, 4, &frame).unwrap();
        assert_eq!(&r.range_mm[..4], &[400, 300, 200, 100]);
        assert_eq!(&r.range_mm[12..], &[1600, 1500, 1400, 1300]);
    }

    #[test]
    fn validity_follows_the_cogs_range_rule() {
        let mut e = Emitter::new(settings(Some(pose(0.0, 0.0))), false, 3000).unwrap();
        let r = e.record(1, 2, &[0, 19, 20, 3001]).unwrap();
        assert_eq!(r.valid, vec![false, false, true, false]);
        assert_eq!(r.range_mm, vec![0, 19, 20, 3001]);
    }

    #[test]
    fn rate_limit_and_bad_frames() {
        let mut e = Emitter::new(settings(Some(pose(0.0, 0.0))), false, 3500).unwrap();
        assert!(e.record(1_000, 4, &[1000; 16]).is_some());
        assert!(e.record(1_500, 4, &[1000; 16]).is_none());
        let r = e.record(2_000, 4, &[1000; 16]).unwrap();
        assert_eq!(r.provenance.receipt, "sen0628-1:000002");
        assert!(e.record(9_000, 4, &[1000; 15]).is_none());
        assert!(e.record(9_000, 17, &[1000; 289]).is_none());
    }

    #[test]
    fn refuses_without_a_pose_and_says_why() {
        let why = Emitter::new(settings(None), false, 3500).err().unwrap();
        assert!(
            why.contains("no sensor pose") && why.contains("--tof-pose"),
            "{why}"
        );
        let mut s = settings(Some(pose(0.0, 0.0)));
        s.region = None;
        assert_eq!(s.refusal_status(), Some("no_region"));
        assert!(Emitter::new(s, false, 3500).is_err());
    }

    #[test]
    fn simulate_is_synthetic_and_real_is_measured() {
        let p = Some(pose(0.0, 0.0));
        let s = Emitter::new(settings(p), true, 3500)
            .unwrap()
            .record(1, 4, &[1000; 16]);
        let r = Emitter::new(settings(p), false, 3500)
            .unwrap()
            .record(1, 4, &[1000; 16]);
        assert_eq!(s.unwrap().provenance.proof, Proof::Synthetic);
        assert_eq!(r.unwrap().provenance.proof, Proof::Measured);
    }

    #[test]
    fn pose_parsing_is_bounded() {
        let p = Pose::parse("4.8,0.05,1.6,90,-35").unwrap();
        assert_eq!((p.roll_deg, p.x_sign), (0.0, 1.0));
        let p = Pose::parse("4.8,0.05,1.6,90,-35,180,-1").unwrap();
        assert_eq!((p.roll_deg, p.x_sign), (180.0, -1.0));
        for bad in [
            "1,2,3,4",
            "1,2,3,4,5,6,7,8",
            "1,2,3,400,0",
            "1,2,3,0,91",
            "1,2,3,0,0,181",
            "1,2,3,0,0,0,0",
            "NaN,0,0,0,0",
            "2000,0,0,0,0",
            "a,b,c,d,e",
        ] {
            assert!(Pose::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn args_build_settings_or_refuse() {
        let parse = |v: &[&str]| {
            let args: Vec<String> = v.iter().map(|s| s.to_string()).collect();
            Args::from_args(&args)?.finish()
        };
        assert_eq!(parse(&["--once", "--simulate"]).unwrap(), None);
        let s = parse(&["--spatial-out", "export"]).unwrap().unwrap();
        assert_eq!((&s.sink, s.pose, s.max_hz), (&Sink::Export, None, 1.0));
        assert_eq!(
            (s.source_id.as_str(), s.uncertainty_m),
            ("sen0628-tof", 0.05)
        );
        assert_eq!(s.refusal_status(), Some("no_pose"));
        let s = parse(&[
            "--once",
            "--spatial-out",
            "/tmp/e.jsonl",
            "--tof-pose",
            "4.8,0.05,1.6,90,-35",
            "--spatial-region",
            "region/urth/meso/test-room",
            "--spatial-max-hz",
            "5",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            (&s.sink, s.max_hz),
            (&Sink::File("/tmp/e.jsonl".into()), 5.0)
        );
        assert!(s.refusal().is_none());
        assert!(parse(&["--tof-pose", "4.8,0.05,1.6,90,-35"]).is_err()); // no sink
        assert!(parse(&["--spatial-out", "/dev/null"]).is_err());
        assert!(parse(&["--spatial-out"]).is_err());
        assert!(parse(&["--spatial-out", "export", "--spatial-max-hz", "16"]).is_err());
        assert!(parse(&["--spatial-out", "export", "--spatial-region", "a b"]).is_err());
    }

    #[test]
    fn file_sink_appends_lines() {
        let p = std::env::temp_dir().join(format!("sen0628-spatial-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let path = p.to_str().unwrap();
        append(path, &["{\"a\":1}".into()]).unwrap();
        append(path, &["{\"a\":2}".into()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "{\"a\":1}\n{\"a\":2}\n"
        );
        std::fs::remove_file(&p).unwrap();
    }
}

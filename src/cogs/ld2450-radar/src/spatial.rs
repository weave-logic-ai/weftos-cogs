//! `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107 §7), built only with the cargo
//! feature `spatial-evidence` and active only when `--spatial-out` is set.
//!
//! Each target of each report frame becomes one `radar_track_point` line in `room_enu`.
//! The wire shape is mirrored with local serde structs, as `ruview-spatial-evidence`
//! does, so the cog never depends on the WeftOS consumer.
//!
//! Target z is 0.0: the radar is 2-D. Each line carries a `sensor` block (mount position,
//! yaw, pitch and the datasheet beam half-angles) so ADR-107's radar model can place the
//! evidence in the beam's vertical band at the track's range and drop floor clutter.
//! `velocity` is omitted: the radar's speed is radial only and its sign convention is not
//! established (ADR-161).

use serde::Serialize;

use crate::window::Target;

pub const SCHEMA_V1: &str = "spatial.evidence.v1";
pub const PRODUCER: &str = concat!("ld2450-radar@", env!("CARGO_PKG_VERSION"));
const MAX_COORD_M: f64 = 1_000.0;
const MAX_ID_LEN: usize = 128;
/// Default 1-sigma position uncertainty. An assumption, not a measurement.
pub const DEFAULT_UNCERTAINTY_M: f64 = 0.3;
/// Beam half-angle in elevation, degrees (Hi-Link HLK-LD2450 manual: ±35° pitch coverage).
pub const ELEV_HALF_DEG: f64 = 35.0;
/// Beam half-angle in azimuth, degrees (Hi-Link HLK-LD2450 manual: ±60°).
pub const AZ_HALF_DEG: f64 = 60.0;

/// Where the radar is in the room and which way it faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Mount position in `room_enu`, metres (east, north, height).
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Boresight bearing in degrees counter-clockwise from room +x (east): 0 faces +x,
    /// 90 faces +y (north). The project's one yaw convention (ENU math yaw).
    pub yaw_deg: f64,
    /// Boresight pitch in degrees above horizontal (negative tilts down). The radar
    /// reports targets in its tilted sensing plane; forward distance on the floor is
    /// `y * cos(pitch)`.
    pub pitch_deg: f64,
    /// +1 if the radar's +x points to its right looking out along the boresight, -1 if
    /// left. Found with the guide's first-run step-right check.
    pub x_sign: f64,
}

impl Pose {
    /// Parse `x,y,z,yaw_deg,pitch_deg,x_sign`, or the older `x,y,z,yaw_deg,x_sign`
    /// (pitch 0).
    pub fn parse(s: &str) -> Result<Pose, String> {
        let v: Vec<f64> = s
            .split(',')
            .map(|p| p.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .map_err(|_| {
                format!("--radar-pose must be x,y,z,yaw_deg,pitch_deg,x_sign, got {s:?}")
            })?;
        let (x, y, z, yaw_deg, pitch_deg, x_sign) = match v[..] {
            [x, y, z, yaw, sign] => (x, y, z, yaw, 0.0, sign),
            [x, y, z, yaw, pitch, sign] => (x, y, z, yaw, pitch, sign),
            _ => {
                return Err(format!(
                    "--radar-pose needs 5 or 6 numbers, got {}",
                    v.len()
                ))
            }
        };
        if !pitch_deg.is_finite() || !(-90.0..=90.0).contains(&pitch_deg) {
            return Err("--radar-pose: pitch_deg must be within -90..=90".into());
        }
        if ![x, y, z, yaw_deg]
            .iter()
            .all(|c| c.is_finite() && c.abs() <= MAX_COORD_M)
        {
            return Err("--radar-pose: x, y, z within +-1000 m, yaw_deg finite".into());
        }
        if !(-360.0..=360.0).contains(&yaw_deg) {
            return Err("--radar-pose: yaw_deg must be within -360..=360".into());
        }
        if x_sign != 1.0 && x_sign != -1.0 {
            return Err("--radar-pose: x_sign must be 1 or -1".into());
        }
        Ok(Pose {
            x,
            y,
            z,
            yaw_deg,
            pitch_deg,
            x_sign,
        })
    }

    /// Radar-local (x lateral, y forward in the tilted sensing plane) to room ENU
    /// east/north, metres. The in-plane forward distance projects to `y * cos(pitch)` on
    /// the floor; forward is `(cos yaw, sin yaw)` and the radar's right is
    /// `(sin yaw, -cos yaw)`.
    pub fn radar_to_room(&self, x_m: f64, y_m: f64) -> [f64; 2] {
        let (s, c) = self.yaw_deg.to_radians().sin_cos();
        let fwd = y_m * self.pitch_deg.to_radians().cos();
        let right = self.x_sign * x_m; // metres to the right of the boresight
        [self.x + fwd * c + right * s, self.y + fwd * s - right * c]
    }

    /// The `sensor` block for ADR-107's radar beam geometry.
    pub fn sensor(&self) -> Sensor {
        Sensor {
            position: [mm(self.x), mm(self.y), mm(self.z)],
            yaw_deg: self.yaw_deg,
            pitch_deg: self.pitch_deg,
            elev_half_deg: ELEV_HALF_DEG,
            az_half_deg: AZ_HALF_DEG,
        }
    }
}

/// Mount and beam of the radar, carried on every line (ADR-107 §7 amendment).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Sensor {
    pub position: [f64; 3],
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub elev_half_deg: f64,
    pub az_half_deg: f64,
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
}

impl Settings {
    /// Why emission is refused, if it is. The cog keeps running without it.
    pub fn refusal(&self) -> Option<String> {
        if self.pose.is_none() {
            return Some(
                "no radar pose: set --radar-pose x,y,z,yaw_deg,pitch_deg,x_sign (room_enu metres, \
                 yaw counter-clockwise from +x (east), pitch up-positive, x_sign from the \
                 guide's step-right check)"
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

/// Collects the `--spatial-*` flags while arguments are parsed.
#[derive(Debug, Default)]
pub struct Args {
    sink: Option<Sink>,
    pose: Option<Pose>,
    region: Option<String>,
    source_id: Option<String>,
    uncertainty_m: Option<f64>,
}

impl Args {
    pub const FLAGS: [&'static str; 5] = [
        "--spatial-out",
        "--radar-pose",
        "--spatial-region",
        "--spatial-source-id",
        "--spatial-uncertainty-m",
    ];

    pub fn set(&mut self, flag: &str, v: &str) -> Result<(), String> {
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
            "--radar-pose" => self.pose = Some(Pose::parse(v)?),
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
            "--spatial-uncertainty-m" => {
                self.uncertainty_m = Some(
                    v.parse::<f64>()
                        .ok()
                        .filter(|u| *u > 0.0 && *u <= 100.0)
                        .ok_or(format!(
                            "--spatial-uncertainty-m must be in (0, 100], got {v:?}"
                        ))?,
                )
            }
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
                || self.uncertainty_m.is_some();
            return if other {
                Err("--radar-pose and --spatial-* need --spatial-out export|FILE".into())
            } else {
                Ok(None)
            };
        };
        Ok(Some(Settings {
            sink,
            pose: self.pose,
            region: self.region,
            source_id: self.source_id.unwrap_or_else(|| "ld2450-radar".into()),
            uncertainty_m: self.uncertainty_m.unwrap_or(DEFAULT_UNCERTAINTY_M),
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

/// One `radar_track_point` line, fields in ADR-107's example order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RadarTrackPoint {
    pub schema: &'static str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub t_ns: u64,
    pub frame: &'static str,
    pub region: String,
    pub source_id: String,
    pub uncertainty_m: f64,
    pub provenance: Provenance,
    /// The radar's slot (1-3): ephemeral, never a person identifier.
    pub track: u32,
    pub position: [f64; 3],
    pub sensor: Sensor,
}

/// Millimetre rounding keeps trigonometry artefacts out of the JSON.
fn mm(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Turns targets into lines. Holds the receipt counter.
pub struct Emitter {
    settings: Settings,
    pose: Pose,
    region: String,
    proof: Proof,
    seq: u64,
}

impl Emitter {
    /// The refusal reason when emission is refused.
    pub fn new(settings: Settings, simulated: bool) -> Result<Emitter, String> {
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
            seq: 0,
        })
    }

    pub fn sink(&self) -> &Sink {
        &self.settings.sink
    }

    /// One line per target of one frame parsed at `t_ms`. Targets whose room position
    /// falls outside +-1000 m (a wrong pose) are skipped.
    pub fn records(&mut self, t_ms: u64, targets: &[Target]) -> Vec<RadarTrackPoint> {
        let mut out = Vec::new();
        for t in targets {
            let [e, n] = self.pose.radar_to_room(t.x_m, t.y_m);
            let position = [mm(e), mm(n), 0.0];
            if !position
                .iter()
                .all(|c| c.is_finite() && c.abs() <= MAX_COORD_M)
            {
                continue;
            }
            self.seq += 1;
            out.push(RadarTrackPoint {
                schema: SCHEMA_V1,
                kind: "radar_track_point",
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
                track: u32::from(t.slot),
                position,
                sensor: self.pose.sensor(),
            });
        }
        out
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

    fn settings(pose: Option<Pose>) -> Settings {
        Settings {
            sink: Sink::Export,
            pose,
            region: Some("region/urth/meso/test-room".into()),
            source_id: "ld2450-1".into(),
            uncertainty_m: 0.15,
        }
    }

    fn pose(x: f64, y: f64, yaw: f64, sign: f64) -> Pose {
        Pose {
            x,
            y,
            z: 1.5,
            yaw_deg: yaw,
            pitch_deg: 0.0,
            x_sign: sign,
        }
    }

    fn target(slot: u8, x_m: f64, y_m: f64) -> Target {
        Target {
            slot,
            x_m,
            y_m,
            speed_mps: 0.4,
            resolution_mm: 360,
        }
    }

    #[test]
    fn golden_line_matches_the_adr_107_example_shape() {
        // ADR-107 §7's radar_track_point example, minus the optional velocity this cog
        // does not emit. A radar at the origin facing +y (yaw 90) with x to its right
        // maps local (2.10, 1.40) to room (2.10, 1.40).
        let mut e = Emitter::new(settings(Some(pose(0.0, 0.0, 90.0, 1.0))), false).unwrap();
        e.seq = 122;
        let r = e.records(1_759_500_001_250, &[target(2, 2.10, 1.40)]);
        let line = serde_json::to_string(&r[0]).unwrap();
        assert_eq!(
            line,
            "{\"schema\":\"spatial.evidence.v1\",\"type\":\"radar_track_point\",\
             \"t_ns\":1759500001250000000,\"frame\":\"room_enu\",\
             \"region\":\"region/urth/meso/test-room\",\"source_id\":\"ld2450-1\",\
             \"uncertainty_m\":0.15,\"provenance\":{\"receipt\":\"ld2450-1:000123\",\
             \"producer\":\"ld2450-radar@0.1.0\",\"proof\":\"MEASURED\"},\
             \"track\":2,\"position\":[2.1,1.4,0.0],\
             \"sensor\":{\"position\":[0.0,0.0,1.5],\"yaw_deg\":90.0,\"pitch_deg\":0.0,\
             \"elev_half_deg\":35.0,\"az_half_deg\":60.0}}"
        );
        let adr = "{\"schema\":\"spatial.evidence.v1\",\"type\":\"radar_track_point\",\"t_ns\":1759500001250000000,\"frame\":\"room_enu\",\"region\":\"region/urth/meso/test-room\",\"source_id\":\"ld6002-1\",\"uncertainty_m\":0.15,\"provenance\":{\"receipt\":\"ld6002-1:000123\",\"producer\":\"ruview-adapter@0.1\",\"proof\":\"MEASURED\"},\"track\":2,\"position\":[2.10,1.40,0.0],\"velocity\":[0.4,0.1,0.0]}";
        let mut want: serde_json::Value = serde_json::from_str(adr).unwrap();
        let ours: serde_json::Value = serde_json::from_str(&line).unwrap();
        want.as_object_mut().unwrap().remove("velocity");
        // ADR-107's beam-geometry example adds the `sensor` block after `position`.
        want.as_object_mut()
            .unwrap()
            .insert("sensor".into(), ours["sensor"].clone());
        let keys =
            |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&ours), keys(&want));
        for k in [
            "schema",
            "type",
            "t_ns",
            "frame",
            "region",
            "uncertainty_m",
            "track",
            "position",
        ] {
            assert_eq!(ours[k], want[k], "{k}");
        }
        assert!(line.len() < 16 * 1024);
        assert!(valid_id(&r[0].provenance.receipt) && valid_id(PRODUCER));
    }

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    #[test]
    fn yaw_0_faces_plus_x_and_yaw_90_faces_plus_y() {
        // Radar-forward (local y) lands on room +x at yaw 0 and on room +y at yaw 90.
        assert!(close(
            pose(0.0, 0.0, 0.0, 1.0).radar_to_room(0.0, 2.0),
            [2.0, 0.0]
        ));
        assert!(close(
            pose(0.0, 0.0, 90.0, 1.0).radar_to_room(0.0, 2.0),
            [0.0, 2.0]
        ));
    }

    #[test]
    fn pose_transform() {
        // Yaw 0, facing +x: the radar's right is -y.
        assert!(close(
            pose(1.0, 2.0, 0.0, 1.0).radar_to_room(0.5, 2.0),
            [3.0, 1.5]
        ));
        // x_sign -1: the radar's +x points left, so +0.5 lands on +y.
        assert!(close(
            pose(1.0, 2.0, 0.0, -1.0).radar_to_room(0.5, 2.0),
            [3.0, 2.5]
        ));
        // Yaw 90, facing +y: the radar's right is +x.
        assert!(close(
            pose(0.0, 3.0, 90.0, 1.0).radar_to_room(0.5, 2.0),
            [0.5, 5.0]
        ));
        // Yaw 225, facing south-west, straight ahead 1 m.
        let h = std::f64::consts::FRAC_1_SQRT_2;
        assert!(close(
            pose(6.2, 3.5, 225.0, 1.0).radar_to_room(0.0, 1.0),
            [6.2 - h, 3.5 - h]
        ));
        // A point to the right lands clockwise of the boresight ray (negative cross product).
        let p = pose(0.0, 0.0, 30.0, 1.0);
        let [ex, ey] = p.radar_to_room(1.0, 2.0);
        let (fx, fy) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
        assert!(fx * ey - fy * ex < 0.0);
        // Range is preserved whatever the pose.
        let p = pose(2.0, 1.0, 37.0, -1.0);
        let [e, n] = p.radar_to_room(-1.2, 3.4);
        assert!(((e - 2.0).hypot(n - 1.0) - 1.2f64.hypot(3.4)).abs() < 1e-9);
    }

    #[test]
    fn non_finite_and_out_of_range_positions_are_dropped() {
        let mut e = Emitter::new(settings(Some(pose(0.0, 0.0, 0.0, 1.0))), false).unwrap();
        let r = e.records(
            1,
            &[
                target(1, f64::NAN, 2.0),
                target(2, 0.0, 2000.0),
                target(3, 0.0, 2.0),
            ],
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].track, 3);
        assert_eq!(r[0].provenance.receipt, "ld2450-1:000001");
    }

    #[test]
    fn refuses_without_a_pose_and_says_why() {
        let why = Emitter::new(settings(None), false).err().unwrap();
        assert!(
            why.contains("no radar pose") && why.contains("--radar-pose"),
            "{why}"
        );
        let mut s = settings(Some(pose(0.0, 0.0, 0.0, 1.0)));
        s.region = None;
        assert!(Emitter::new(s, false)
            .err()
            .unwrap()
            .contains("--spatial-region"));
    }

    #[test]
    fn simulate_is_synthetic_and_real_is_measured() {
        let p = Some(pose(0.0, 0.0, 0.0, 1.0));
        let mut sim = Emitter::new(settings(p), true).unwrap();
        let mut real = Emitter::new(settings(p), false).unwrap();
        let t = [target(1, 0.0, 2.0)];
        let s = serde_json::to_value(&sim.records(1, &t)[0]).unwrap();
        let r = serde_json::to_value(&real.records(1, &t)[0]).unwrap();
        assert_eq!(s["provenance"]["proof"], "SYNTHETIC");
        assert_eq!(r["provenance"]["proof"], "MEASURED");
    }

    #[test]
    fn receipts_are_unique_and_one_line_per_target() {
        let mut e = Emitter::new(settings(Some(pose(0.0, 0.0, 0.0, 1.0))), false).unwrap();
        let r = e.records(1, &[target(1, 0.0, 1.0), target(3, 1.0, 2.0)]);
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].track, 3);
        assert_ne!(r[0].provenance.receipt, r[1].provenance.receipt);
        assert!(e.records(2, &[]).is_empty());
    }

    #[test]
    fn pose_parsing_is_bounded() {
        assert_eq!(
            Pose::parse("6.2,3.5,1.1,225,-1").unwrap(),
            Pose {
                x: 6.2,
                y: 3.5,
                z: 1.1,
                yaw_deg: 225.0,
                pitch_deg: 0.0,
                x_sign: -1.0
            }
        );
        assert_eq!(
            Pose::parse("0.5,0.5,1.8,26,-10,1").unwrap().pitch_deg,
            -10.0
        );
        assert!(Pose::parse("0.5,0.5,1.8,26,-95,1").is_err());
        for bad in [
            "1,2,3,4",
            "1,2,3,4,0",
            "1,2,3,4,2",
            "1,2,3,999,1",
            "NaN,0,0,0,1",
            "2000,0,0,0,1",
            "a,b,c,d,e",
        ] {
            assert!(Pose::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn args_build_settings_or_refuse() {
        let parse = |pairs: &[(&str, &str)]| {
            let mut a = Args::default();
            for (f, v) in pairs {
                a.set(f, v)?;
            }
            a.finish()
        };
        assert_eq!(parse(&[]).unwrap(), None);
        let s = parse(&[("--spatial-out", "export")]).unwrap().unwrap();
        assert_eq!((s.sink, s.pose, s.uncertainty_m), (Sink::Export, None, 0.3));
        assert_eq!(s.source_id, "ld2450-radar");
        let s = parse(&[
            ("--spatial-out", "/tmp/e.jsonl"),
            ("--radar-pose", "1,2,1.5,90,1"),
            ("--spatial-region", "region/urth/meso/test-room"),
            ("--spatial-uncertainty-m", "0.2"),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(s.sink, Sink::File("/tmp/e.jsonl".into()));
        assert!(s.refusal().is_none() && s.refusal_status().is_none());
        let s = parse(&[("--spatial-out", "export")]).unwrap().unwrap();
        assert_eq!(s.refusal_status(), Some("no_pose"));
        assert!(parse(&[("--radar-pose", "1,2,1.5,90,1")]).is_err()); // no sink
        assert!(parse(&[("--spatial-out", "/dev/null")]).is_err());
        assert!(parse(&[("--spatial-region", "bad region")]).is_err());
        assert!(parse(&[("--spatial-uncertainty-m", "0")]).is_err());
    }

    #[test]
    fn ids_and_paths() {
        assert!(valid_id("ld2450-1") && valid_id("region/urth/meso/test-room"));
        assert!(!valid_id("") && !valid_id("has space") && !valid_id(&"a".repeat(129)));
        assert!(valid_path("/var/lib/cognitum/ld2450.evidence.jsonl"));
        assert!(!valid_path("/dev/null") && !valid_path("../x.jsonl") && !valid_path(""));
    }

    #[test]
    fn file_sink_appends_lines() {
        let p = std::env::temp_dir().join(format!("ld2450-spatial-{}.jsonl", std::process::id()));
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

#[cfg(test)]
mod pitch_tests {
    use super::*;

    fn tilted(pitch: f64) -> Pose {
        Pose {
            x: 0.0,
            y: 0.0,
            z: 1.8,
            yaw_deg: 0.0,
            pitch_deg: pitch,
            x_sign: 1.0,
        }
    }

    #[test]
    fn pitch_zero_matches_the_level_transform() {
        assert_eq!(tilted(0.0).radar_to_room(0.3, 2.0), [2.0, -0.3]);
    }

    #[test]
    fn pitching_down_shortens_the_floor_distance() {
        let [e, _] = tilted(-20.0).radar_to_room(0.0, 2.0);
        assert!((e - 2.0 * 20f64.to_radians().cos()).abs() < 1e-12, "{e}");
        assert!(e < 2.0);
    }

    #[test]
    fn every_line_carries_the_mount_and_datasheet_beam() {
        let s = tilted(-10.0).sensor();
        assert_eq!(
            (s.pitch_deg, s.elev_half_deg, s.az_half_deg),
            (-10.0, 35.0, 60.0)
        );
        assert_eq!(s.position, [0.0, 0.0, 1.8]);
    }
}

//! `spatial.evidence.v1` adapter (WeftOS-spatial ADR-107 §7, 2026-10-03 amendment), built
//! only with the cargo feature `spatial-evidence` and active only when `--spatial-out` is set.
//!
//! A report frame with state 1 ("present") and a non-zero distance becomes one `radar_range`
//! line: something reflects at that range, bearing unknown inside the beam. At most
//! `--spatial-max-hz` lines per second (default 2). No-target frames send nothing, as
//! ADR-107 asks, and so do the vendor gesture codes (state 2-8), whose distance meaning is
//! not documented. The wire shape is mirrored with local serde structs, as
//! `ruview-spatial-evidence` does, so the cog never depends on the WeftOS consumer.

use serde::Serialize;

pub const SCHEMA_V1: &str = "spatial.evidence.v1";
pub const PRODUCER: &str = concat!("rd-03e@", env!("CARGO_PKG_VERSION"));
const MAX_COORD_M: f64 = 1_000.0;
const MAX_ID_LEN: usize = 128;
/// Default 1-sigma range uncertainty. An assumption: the datasheet claims ±5 cm from 30 to
/// 350 cm and ±5 % to 600 cm, which is not measured here.
pub const DEFAULT_UNCERTAINTY_M: f64 = 0.1;
/// Full beam `[horizontal, vertical]`, degrees. Ai-Thinker Rd-03E Specification V1.0.0, §1
/// and §7.2: "Accurate ranging detection range azimuth ±20°, elevation angle ±45°", for the
/// wall-mounted orientation shown there.
pub const FOV_DEG: [f64; 2] = [40.0, 90.0];
pub const DEFAULT_MAX_HZ: f64 = 2.0;
const MAX_HZ: f64 = 20.0;

/// Where the antenna is in the room and which way it faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Antenna position in `room_enu`, metres (east, north, height).
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Boresight bearing in degrees counter-clockwise from room +x (east): 0 faces +x,
    /// 90 faces +y (north). The project's one yaw convention (ENU math yaw).
    pub yaw_deg: f64,
    /// Boresight pitch in degrees above horizontal (negative tilts down).
    pub pitch_deg: f64,
}

impl Pose {
    /// Parse `x,y,z,yaw_deg[,pitch_deg]` (pitch 0 when omitted).
    pub fn parse(s: &str) -> Result<Pose, String> {
        let v: Vec<f64> = s
            .split(',')
            .map(|p| p.trim().parse::<f64>())
            .collect::<Result<_, _>>()
            .map_err(|_| format!("--radar-pose must be x,y,z,yaw_deg[,pitch_deg], got {s:?}"))?;
        let (x, y, z, yaw_deg, pitch_deg) = match v[..] {
            [x, y, z, yaw] => (x, y, z, yaw, 0.0),
            [x, y, z, yaw, pitch] => (x, y, z, yaw, pitch),
            _ => {
                return Err(format!(
                    "--radar-pose needs 4 or 5 numbers, got {}",
                    v.len()
                ))
            }
        };
        if ![x, y, z]
            .iter()
            .all(|c| c.is_finite() && c.abs() <= MAX_COORD_M)
        {
            return Err("--radar-pose: x, y, z within +-1000 m".into());
        }
        if !yaw_deg.is_finite() || !(-360.0..=360.0).contains(&yaw_deg) {
            return Err("--radar-pose: yaw_deg must be within -360..=360".into());
        }
        if !pitch_deg.is_finite() || !(-90.0..=90.0).contains(&pitch_deg) {
            return Err("--radar-pose: pitch_deg must be within -90..=90".into());
        }
        Ok(Pose {
            x,
            y,
            z,
            yaw_deg,
            pitch_deg,
        })
    }

    /// Unit boresight in `room_enu`. Used by tests to pin the yaw convention.
    #[cfg(test)]
    pub fn boresight(&self) -> [f64; 3] {
        let (sy, cy) = self.yaw_deg.to_radians().sin_cos();
        let (sp, cp) = self.pitch_deg.to_radians().sin_cos();
        [cp * cy, cp * sy, sp]
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
                "no radar pose: set --radar-pose x,y,z,yaw_deg[,pitch_deg] (room_enu metres, \
                 yaw counter-clockwise from +x (east), pitch up-positive)"
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
        "--radar-pose",
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
                Err("--radar-pose and --spatial-* need --spatial-out export|FILE".into())
            } else {
                Ok(None)
            };
        };
        Ok(Some(Settings {
            sink,
            pose: self.pose,
            region: self.region,
            source_id: self.source_id.unwrap_or_else(|| "rd-03e".into()),
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

/// One `radar_range` line, fields in ADR-107's example order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RadarRange {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch_deg: Option<f64>,
    pub fov_deg: [f64; 2],
    pub range_m: f64,
    /// The module reports one target (the nearest).
    pub targets: u32,
}

/// Millimetre rounding keeps float artefacts out of the JSON.
fn mm(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Turns report frames into lines. Holds the receipt counter and the rate limit.
pub struct Emitter {
    settings: Settings,
    pose: Pose,
    region: String,
    proof: Proof,
    seq: u64,
    last_ms: Option<u64>,
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
            last_ms: None,
        })
    }

    pub fn sink(&self) -> &Sink {
        &self.settings.sink
    }

    /// A line for one report frame read at `t_ms`, or `None` for a no-target or gesture
    /// frame, a zero distance, or while the rate limit holds.
    pub fn record(&mut self, t_ms: u64, distance_cm: u16, state: u8) -> Option<RadarRange> {
        if state != 1 || distance_cm == 0 {
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
        self.seq += 1;
        let p = self.pose;
        Some(RadarRange {
            schema: SCHEMA_V1,
            kind: "radar_range",
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
            pitch_deg: (p.pitch_deg != 0.0).then_some(p.pitch_deg),
            fov_deg: FOV_DEG,
            range_m: f64::from(distance_cm) / 100.0,
            targets: 1,
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
            x: 0.10,
            y: 1.83,
            z: 1.00,
            yaw_deg: yaw,
            pitch_deg: pitch,
        }
    }

    fn settings(pose: Option<Pose>) -> Settings {
        Settings {
            sink: Sink::Export,
            pose,
            region: Some("region/urth/meso/test-room".into()),
            source_id: "rd03e-1".into(),
            uncertainty_m: 0.3,
            max_hz: 2.0,
        }
    }

    const ADR_EXAMPLE: &str = "{\"schema\":\"spatial.evidence.v1\",\"type\":\"radar_range\",\"t_ns\":1759500004100000000,\"frame\":\"room_enu\",\"region\":\"region/urth/meso/test-room\",\"source_id\":\"rd03e-1\",\"uncertainty_m\":0.3,\"provenance\":{\"receipt\":\"rd03e-1:000007\",\"producer\":\"rd-03e@0.1.0\",\"proof\":\"MEASURED\"},\"position\":[0.10,1.83,1.00],\"yaw_deg\":0.0,\"fov_deg\":[90.0,60.0],\"range_m\":2.45,\"targets\":1}";

    #[test]
    fn golden_line_matches_the_adr_107_example() {
        // ADR-107 §7.1's radar_range example is an RD-03E reading at 245 cm. Its fov_deg
        // [90, 60] is illustrative; the cog sends the datasheet's [40, 90].
        let mut e = Emitter::new(settings(Some(pose(0.0, 0.0))), false).unwrap();
        e.seq = 6;
        let r = e.record(1_759_500_004_100, 245, 1).unwrap();
        let line = serde_json::to_string(&r).unwrap();
        let ours: serde_json::Value = serde_json::from_str(&line).unwrap();
        let mut want: serde_json::Value = serde_json::from_str(ADR_EXAMPLE).unwrap();
        want["fov_deg"] = serde_json::json!([40.0, 90.0]);
        assert_eq!(ours, want);
        let keys =
            |v: &serde_json::Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        assert_eq!(keys(&ours), keys(&want), "key order");
        assert!(line.len() < 16 * 1024);
        assert!(valid_id(&r.provenance.receipt) && valid_id(PRODUCER));
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn yaw_0_faces_plus_x_and_yaw_90_faces_plus_y() {
        assert!(close(pose(0.0, 0.0).boresight(), [1.0, 0.0, 0.0]));
        assert!(close(pose(90.0, 0.0).boresight(), [0.0, 1.0, 0.0]));
        assert!(pose(0.0, -30.0).boresight()[2] < 0.0);
    }

    #[test]
    fn pitch_is_sent_only_when_set() {
        let mut e = Emitter::new(settings(Some(pose(26.0, -10.0))), false).unwrap();
        let v = serde_json::to_value(e.record(1, 100, 1).unwrap()).unwrap();
        assert_eq!(
            (v["yaw_deg"].clone(), v["pitch_deg"].clone()),
            (26.0.into(), (-10.0).into())
        );
        assert_eq!(v["range_m"], 1.0);
    }

    #[test]
    fn only_present_frames_with_a_distance_become_lines() {
        let mut e = Emitter::new(settings(Some(pose(0.0, 0.0))), false).unwrap();
        assert!(e.record(1_000, 120, 0).is_none(), "no target");
        assert!(e.record(1_000, 0, 1).is_none(), "zero distance");
        assert!(e.record(1_000, 80, 3).is_none(), "gesture code");
        assert!(e.record(1_000, 120, 1).is_some());
        assert!(e.record(1_400, 120, 1).is_none(), "rate limit, 2 Hz");
        let r = e.record(1_500, 130, 1).unwrap();
        assert_eq!(r.provenance.receipt, "rd03e-1:000002");
    }

    #[test]
    fn refuses_without_a_pose_and_says_why() {
        let why = Emitter::new(settings(None), false).err().unwrap();
        assert!(
            why.contains("no radar pose") && why.contains("--radar-pose"),
            "{why}"
        );
        let mut s = settings(Some(pose(0.0, 0.0)));
        s.region = None;
        assert_eq!(s.refusal_status(), Some("no_region"));
        assert!(Emitter::new(s, false).is_err());
    }

    #[test]
    fn simulate_is_synthetic_and_real_is_measured() {
        let p = Some(pose(0.0, 0.0));
        let s = Emitter::new(settings(p), true).unwrap().record(1, 100, 1);
        let r = Emitter::new(settings(p), false).unwrap().record(1, 100, 1);
        assert_eq!(s.unwrap().provenance.proof, Proof::Synthetic);
        assert_eq!(r.unwrap().provenance.proof, Proof::Measured);
    }

    #[test]
    fn pose_parsing_is_bounded() {
        assert_eq!(Pose::parse("0.1,1.83,1.0,0").unwrap(), pose(0.0, 0.0));
        assert_eq!(Pose::parse("0.1,1.83,1.0,26,-10").unwrap().pitch_deg, -10.0);
        for bad in [
            "1,2,3",
            "1,2,3,4,5,6",
            "1,2,3,400",
            "1,2,3,0,91",
            "NaN,0,0,0",
            "2000,0,0,0",
            "a,b,c,d",
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
        assert_eq!((&s.sink, s.pose, s.max_hz), (&Sink::Export, None, 2.0));
        assert_eq!((s.source_id.as_str(), s.uncertainty_m), ("rd-03e", 0.1));
        assert_eq!(s.refusal_status(), Some("no_pose"));
        let s = parse(&[
            "--spatial-out",
            "/tmp/e.jsonl",
            "--radar-pose",
            "0.1,1.83,1.0,0",
            "--spatial-region",
            "region/urth/meso/test-room",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(&s.sink, &Sink::File("/tmp/e.jsonl".into()));
        assert!(s.refusal().is_none());
        assert!(parse(&["--radar-pose", "0.1,1.83,1.0,0"]).is_err()); // no sink
        assert!(parse(&["--spatial-out", "/proc/x"]).is_err());
        assert!(parse(&["--spatial-out"]).is_err());
        assert!(parse(&["--spatial-out", "export", "--spatial-max-hz", "0"]).is_err());
        assert!(parse(&["--spatial-out", "export", "--spatial-source-id", "a b"]).is_err());
    }

    #[test]
    fn file_sink_appends_lines() {
        let p = std::env::temp_dir().join(format!("rd03e-spatial-{}.jsonl", std::process::id()));
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

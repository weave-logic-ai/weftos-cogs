//! Binary-level checks that need no radar: --help, argument refusal, the fail-honest line
//! for a device that does not exist, the --simulate path end to end, and --replay.

use std::process::Command;
use std::time::{Duration, Instant};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cog-ld2450-radar"))
}

fn one_line(out: &std::process::Output) -> serde_json::Value {
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8(out.stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    serde_json::from_str(lines[0]).unwrap()
}

#[test]
fn help_exits_zero_and_lists_console_commands() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--once --simulate"), "{text}");
}

#[test]
fn unknown_argument_is_refused() {
    let out = bin().args(["--once", "--exec", "sh"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn device_outside_dev_is_refused() {
    let out = bin()
        .args(["--once", "--device", "/etc/hosts"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn missing_device_emits_one_null_line_and_exits_zero() {
    let t0 = Instant::now();
    let out = bin()
        .args([
            "--once",
            "--device",
            "/dev/ld2450-radar-test-does-not-exist",
        ])
        .output()
        .unwrap();
    assert!(t0.elapsed() < Duration::from_secs(15));
    let v = one_line(&out);
    assert_eq!(v["health"], "no_source");
    assert!(v["quality"].is_null());
    assert!(v["target_count"].is_null());
    assert!(v["frame_rate_hz"].is_null());
    assert_eq!(v["targets"], serde_json::json!([]));
    assert_eq!(v["source"]["verified"], false);
    assert_eq!(v["frame"], "radar_local");
    assert!(v["hint"].as_str().unwrap().contains("--device"));
    let ts = v["timestamp"].as_u64().unwrap();
    assert!(ts < 100_000_000_000, "timestamp must be seconds");
}

#[test]
fn simulate_once_runs_the_real_decoder_end_to_end() {
    let t0 = Instant::now();
    let out = bin().args(["--once", "--simulate"]).output().unwrap();
    assert!(t0.elapsed() < Duration::from_secs(5));
    let v = one_line(&out);
    assert_eq!(v["health"], "ok", "{v}");
    assert_eq!(v["source"]["simulated"], true);
    assert_eq!(v["source"]["verified"], false);
    assert!(v["reasons"]
        .as_array()
        .unwrap()
        .contains(&"simulated".into()));
    let frames = v["frames"].as_u64().unwrap();
    assert!((8..=12).contains(&frames), "{frames} frames in 1 s");
    assert!(v["target_count"].as_u64().unwrap() >= 1);
    let t = &v["targets"][0];
    for k in ["x_m", "y_m", "speed_mps"] {
        assert!(t[k].is_number(), "{k}: {t}");
    }
    assert_eq!(t["resolution_mm"], 360);
    assert_eq!(v["parse_errors"], 0);
    assert!(v["hint"].is_null());
    assert!(serde_json::to_string(&v).unwrap().len() < 65_536);
}

#[test]
fn simulate_answers_the_firmware_query() {
    let out = bin()
        .args(["--once", "--simulate", "--query-firmware"])
        .output()
        .unwrap();
    let v = one_line(&out);
    assert_eq!(v["firmware"], "V0.00.00000000");
    assert_eq!(v["tracking_mode"], "multi");
    assert!(v["acks"].as_u64().unwrap() >= 4);
}

fn capture(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn replay_decodes_a_capture_with_garbage() {
    // The protocol document's example frame, ten times, behind some line noise.
    let doc: [u8; 30] = [
        0xAA, 0xFF, 0x03, 0x00, 0x0E, 0x03, 0xB1, 0x86, 0x10, 0x00, 0x40, 0x01, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x55, 0xCC,
    ];
    let mut bytes = vec![0x00, 0x55, 0xAA, 0xFF];
    for _ in 0..10 {
        bytes.extend(doc);
    }
    let p = capture("replay-doc.bin", &bytes);
    let out = bin()
        .args(["--once", "--replay", p.to_str().unwrap()])
        .output()
        .unwrap();
    let v = one_line(&out);
    assert_eq!(v["frames"], 10);
    assert_eq!(v["resync_bytes"], 4);
    assert!(v["frame_rate_hz"].is_null());
    assert_eq!(v["source"]["kind"], "ld2450-replay");
    assert_eq!(
        v["targets"],
        serde_json::json!([{"slot":1,"x_m":-0.782,"y_m":1.713,"speed_mps":-0.16,"resolution_mm":320}])
    );
}

#[test]
fn replay_of_a_missing_file_is_no_source() {
    let out = bin()
        .args(["--once", "--replay", "/nonexistent/ld2450.bin"])
        .output()
        .unwrap();
    let v = one_line(&out);
    assert_eq!(v["health"], "no_source");
}

#[cfg(feature = "spatial-evidence")]
#[test]
fn simulate_writes_synthetic_spatial_evidence_jsonl() {
    let p = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sim-evidence.jsonl");
    let _ = std::fs::remove_file(&p);
    let out = bin()
        .args([
            "--once",
            "--simulate",
            "--spatial-out",
            p.to_str().unwrap(),
            "--radar-pose",
            "3.0,0.0,1.5,90,1",
            "--spatial-region",
            "region/urth/meso/test-room",
        ])
        .output()
        .unwrap();
    let v = one_line(&out);
    assert!(
        !v["reasons"].to_string().contains("spatial_evidence"),
        "{v}"
    );
    assert_eq!(v["spatial"], "emitting");
    let text = std::fs::read_to_string(&p).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(lines.len() >= 8, "{} lines", lines.len());
    for l in &lines {
        assert_eq!(l["schema"], "spatial.evidence.v1");
        assert_eq!(l["type"], "radar_track_point");
        assert_eq!(l["frame"], "room_enu");
        assert_eq!(l["provenance"]["proof"], "SYNTHETIC");
        assert_eq!(l["position"][2], 0.0);
        assert!(l.get("velocity").is_none());
    }
}

#[cfg(feature = "spatial-evidence")]
#[test]
fn spatial_evidence_without_pose_is_refused_and_says_why() {
    let p = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-pose.jsonl");
    let _ = std::fs::remove_file(&p);
    let out = bin()
        .args(["--once", "--simulate", "--spatial-out", p.to_str().unwrap()])
        .output()
        .unwrap();
    let v = one_line(&out);
    assert_eq!(v["spatial"], "no_pose");
    let reasons = v["reasons"].to_string();
    assert!(
        reasons.contains("spatial_evidence_refused: no radar pose"),
        "{reasons}"
    );
    assert!(!p.exists(), "nothing may be written without a pose");
}

#[cfg(not(feature = "spatial-evidence"))]
#[test]
fn default_build_has_no_spatial_flags() {
    let out = bin()
        .args(["--once", "--simulate", "--spatial-out", "export"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

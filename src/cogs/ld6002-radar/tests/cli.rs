//! Binary-level checks that need no radar: --help, argument refusal, and the
//! fail-honest line for a device that does not exist.

use std::process::Command;
use std::time::{Duration, Instant};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cog-ld6002-radar"))
}

#[test]
fn help_exits_zero_and_lists_console_commands() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--once --device /dev/ttyUSB0"), "{text}");
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
            "/dev/ld6002-radar-test-does-not-exist",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(t0.elapsed() < Duration::from_secs(15));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "{stdout}");
    let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(v["health"], "no_source");
    assert!(v["quality"].is_null());
    assert_eq!(v["source"]["verified"], false);
    for k in [
        "presence",
        "distance_cm",
        "heart_rate_bpm",
        "breathing_rate_bpm",
        "target_count",
    ] {
        assert!(v[k].is_null(), "{k} must be null without a device");
    }
    assert!(v["nearest"].is_null());
    assert_eq!(v["targets"], serde_json::json!([]));
    assert_eq!(v["frame"], "radar_local");
    let ts = v["timestamp"].as_u64().unwrap();
    assert!(ts < 100_000_000_000, "timestamp must be seconds");
    assert_eq!(ts, v["timestamp_ms"].as_u64().unwrap() / 1000);
}

#[cfg(feature = "spatial-evidence")]
#[test]
fn spatial_evidence_without_pose_is_refused_and_says_why() {
    let p = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-pose.jsonl");
    let _ = std::fs::remove_file(&p);
    let out = bin()
        .args([
            "--once",
            "--device",
            "/dev/ld6002-radar-test-does-not-exist",
            "--spatial-out",
            p.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    assert_eq!(v["spatial"], "no_pose");
    let reasons = v["reasons"].to_string();
    assert!(
        reasons.contains("spatial_evidence_refused: no radar pose"),
        "{reasons}"
    );
    assert!(!p.exists(), "nothing may be written without a pose");
}

#[cfg(feature = "spatial-evidence")]
#[test]
fn spatial_evidence_with_pose_reports_emitting_and_writes_nothing_without_frames() {
    let p = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-frames.jsonl");
    let _ = std::fs::remove_file(&p);
    let out = bin()
        .args([
            "--once",
            "--device",
            "/dev/ld6002-radar-test-does-not-exist",
            "--spatial-out",
            p.to_str().unwrap(),
            "--radar-pose",
            "0.5,0.5,1.8,26,-10,1",
            "--spatial-region",
            "region/urth/meso/test-room",
        ])
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out.stdout).unwrap().trim()).unwrap();
    assert_eq!(v["health"], "no_source");
    assert_eq!(v["spatial"], "emitting");
    assert!(!p.exists(), "no frames, no evidence");
    let out = bin()
        .args(["--once", "--spatial-out", "export"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "this cog has no export");
}

#[cfg(not(feature = "spatial-evidence"))]
#[test]
fn default_build_has_no_spatial_flags() {
    let out = bin()
        .args(["--once", "--spatial-out", "/tmp/x.jsonl"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

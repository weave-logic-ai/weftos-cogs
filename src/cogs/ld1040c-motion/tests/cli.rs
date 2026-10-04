//! Binary-level checks that need no radar: --help, argument refusal, and the --simulate path end
//! to end. The real OUT line is Linux + GPIO only, so the no_source case is asserted off-Linux.

use std::process::Command;
use std::time::{Duration, Instant};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cog-ld1040c-motion"))
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
fn uart_outside_dev_is_refused() {
    let out = bin()
        .args(["--once", "--uart", "/etc/hosts"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn simulate_once_runs_the_real_aggregation_end_to_end() {
    let t0 = Instant::now();
    // Two seconds lands inside the simulator's walk + hold, so motion is reliably held.
    let out = bin()
        .args(["--once", "--simulate", "--interval", "2"])
        .output()
        .unwrap();
    assert!(t0.elapsed() < Duration::from_secs(10));
    let v = one_line(&out);
    assert_eq!(v["status"], "present", "{v}");
    assert_eq!(v["simulated"], true);
    assert_eq!(v["present"], true);
    assert_eq!(v["warming"], false); // --simulate has no warm-up
    assert!(v["motion_events"].as_u64().unwrap() >= 1, "{v}");
    let frac = v["active_fraction"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&frac));
    // UART is off by default: telemetry fields must be null, never fabricated.
    assert!(v["motion_amplitude"].is_null());
    assert!(v["signal"].is_null());
    let ts = v["timestamp"].as_u64().unwrap();
    assert!(ts < 100_000_000_000, "timestamp must be seconds");
    assert!(serde_json::to_string(&v).unwrap().len() < 65_536);
}

#[cfg(not(target_os = "linux"))]
#[test]
fn no_gpio_host_is_no_source_and_exits_zero() {
    let t0 = Instant::now();
    let out = bin().args(["--once", "--gpio", "17"]).output().unwrap();
    assert!(t0.elapsed() < Duration::from_secs(15));
    let v = one_line(&out);
    assert_eq!(v["status"], "no_source");
    assert_eq!(v["present"], false);
    assert!(v["seconds_since_motion"].is_null());
}

//! Bosch BME280 cog. One JSON line per sample.
//!
//!   cog-bme280 --once
//!   cog-bme280 --once --simulate
//!   cog-bme280 --help

mod bus;
mod compensate;
mod decode;

use bus::read_once;
use compensate::{compensate, humidity_pct, pressure_pa, temp_c, Compensated};
use decode::synthetic_sample;
use std::io::{self, Write};
use std::thread::sleep;
use std::time::Duration;

const TAG: &str = "[cog-bme280]";

struct Opts {
    once: bool,
    simulate: bool,
    interval: u64,
    bus: u8,
    addr: u8,
}

fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|s| s == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn num(args: &[String], flag: &str, default: u64, lo: u64, hi: u64) -> u64 {
    match arg(args, flag).and_then(|v| v.parse().ok()) {
        Some(v) if (lo..=hi).contains(&v) => v,
        Some(_) => {
            eprintln!("{TAG} {flag} out of range, using {default}");
            default
        }
        None => default,
    }
}

fn parse_addr(args: &[String]) -> u8 {
    let raw = arg(args, "--i2c-addr");
    let value = raw.and_then(|s| match s.strip_prefix("0x") {
        Some(hex) => u8::from_str_radix(hex, 16).ok(),
        None => s.parse().ok(),
    });
    match value {
        Some(v) if v == 0x76 || v == 0x77 => v,
        Some(_) => {
            eprintln!("{TAG} --i2c-addr must be 118 or 119 (0x76 or 0x77), using 118");
            0x76
        }
        None => 0x76,
    }
}

fn parse_opts(args: &[String]) -> Opts {
    Opts {
        once: args.iter().any(|s| s == "--once"),
        simulate: args.iter().any(|s| s == "--simulate"),
        interval: num(args, "--interval", 5, 1, 60),
        bus: num(args, "--i2c-bus", 1, 0, 9) as u8,
        addr: parse_addr(args),
    }
}

fn help_text() -> &'static str {
    "\
cog-bme280 — Bosch BME280 temperature, humidity, and pressure over I2C

  cog-bme280 --once                 one sample from /dev/i2c-N, then exit
  cog-bme280 --once --simulate      synthetic registers, labelled SYNTHETIC
  cog-bme280 --interval 5           a sample every 5 seconds
  cog-bme280 --i2c-bus 1            Linux bus, default 1
  cog-bme280 --i2c-addr 118         0x76 (SDO low) or 119 / 0x77 (SDO high)
  cog-bme280 --help
"
}

struct Line {
    status: &'static str,
    provenance: Option<&'static str>,
    error: Option<String>,
    reading: Option<Compensated>,
}

fn line_json(line: &Line) -> String {
    let (temp, press, hum) = match line.reading {
        Some(r) => (
            temp_c(r.temp_centi),
            pressure_pa(r.pressure_centi_pa),
            humidity_pct(r.humidity_q22_10),
        ),
        None => ("null".into(), "null".into(), "null".into()),
    };
    let temp = if line.reading.is_some() { format!("\"{temp}\"") } else { temp };
    let press = if line.reading.is_some() { format!("\"{press}\"") } else { press };
    let hum = if line.reading.is_some() { format!("\"{hum}\"") } else { hum };
    let provenance = match line.provenance {
        Some(p) => format!("\"{p}\""),
        None => "null".into(),
    };
    let error = match &line.error {
        Some(e) => format!("\"{}\"", e.replace('\\', "\\\\").replace('"', "\\\"")),
        None => "null".into(),
    };
    format!(
        "{{\"catalog_id\":\"bme280\",\"status\":\"{}\",\"provenance\":{provenance},\"temp_c\":{temp},\"pressure_pa\":{press},\"humidity_pct\":{hum},\"error\":{error}}}",
        line.status
    )
}

fn simulate_line() -> Line {
    let (raw, mut cal) = synthetic_sample();
    let reading = compensate(&mut cal, raw);
    Line { status: "simulate", provenance: Some("SYNTHETIC"), error: None, reading: Some(reading) }
}

fn measure(opts: &Opts) -> Line {
    if opts.simulate {
        return simulate_line();
    }
    match read_once(opts.bus, opts.addr) {
        Ok(sample) => {
            let mut cal = sample.cal;
            let reading = compensate(&mut cal, sample.raw);
            Line { status: "ok", provenance: Some("MEASURED"), error: None, reading: Some(reading) }
        }
        Err(error) => Line {
            status: "no_source",
            provenance: None,
            error: Some(error),
            reading: None,
        },
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|s| s == "--help") {
        print!("{}", help_text());
        return;
    }
    let opts = parse_opts(&args);
    loop {
        let line = line_json(&measure(&opts));
        println!("{line}");
        let _ = io::stdout().flush();
        if opts.once {
            break;
        }
        sleep(Duration::from_secs(opts.interval));
    }
}

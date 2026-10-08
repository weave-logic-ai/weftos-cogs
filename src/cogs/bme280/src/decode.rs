//! Register map from BST-BME280-DS002 and the Bosch driver v3.5.1 parser.
//! Burst at 0xF7 is pressure, temperature (20-bit each), then humidity (16-bit).
//! Calibration is little-endian at 0x88 (26 bytes, including dig_H1) and 0xE1 (7 bytes).

use crate::compensate::{Calib, Raw};

// The Linux bus is the only caller. A macOS build still typechecks this file.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const REG_CHIP_ID: u8 = 0xD0;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const CHIP_ID_BME280: u8 = 0x60;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const REG_CTRL_HUM: u8 = 0xF2;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const REG_CTRL_MEAS: u8 = 0xF4;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const REG_DATA: u8 = 0xF7;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const REG_CALIB_TP: u8 = 0x88;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const REG_CALIB_H: u8 = 0xE1;
/// Humidity x1. Written before ctrl_meas, or the humidity setting is ignored.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const CTRL_HUM_X1: u8 = 0x01;
/// Temperature x1, pressure x1, forced mode.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const CTRL_MEAS_FORCED_X1: u8 = 0x25;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn chip_is_bme280(id: u8) -> bool {
    id == CHIP_ID_BME280
}

pub fn parse_burst(reg: &[u8; 8]) -> Raw {
    let take20 = |b0: u8, b1: u8, b2: u8| -> u32 {
        (u32::from(b0) << 12) | (u32::from(b1) << 4) | (u32::from(b2) >> 4)
    };
    Raw {
        pressure: take20(reg[0], reg[1], reg[2]),
        temperature: take20(reg[3], reg[4], reg[5]),
        humidity: (u32::from(reg[6]) << 8) | u32::from(reg[7]),
    }
}

pub fn parse_calib(tp: &[u8; 26], hum: &[u8; 7]) -> Calib {
    let u16_le = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let i16_le = |b: &[u8], i: usize| i16::from_le_bytes([b[i], b[i + 1]]);
    let dig_h4_msb = i16::from(hum[3] as i8) * 16;
    let dig_h4_lsb = i16::from(hum[4] & 0x0f);
    let dig_h5_msb = i16::from(hum[5] as i8) * 16;
    let dig_h5_lsb = i16::from(hum[4] >> 4);
    Calib {
        dig_t1: u16_le(tp, 0),
        dig_t2: i16_le(tp, 2),
        dig_t3: i16_le(tp, 4),
        dig_p1: u16_le(tp, 6),
        dig_p2: i16_le(tp, 8),
        dig_p3: i16_le(tp, 10),
        dig_p4: i16_le(tp, 12),
        dig_p5: i16_le(tp, 14),
        dig_p6: i16_le(tp, 16),
        dig_p7: i16_le(tp, 18),
        dig_p8: i16_le(tp, 20),
        dig_p9: i16_le(tp, 22),
        dig_h1: tp[25],
        dig_h2: i16_le(hum, 0),
        dig_h3: hum[2],
        dig_h4: dig_h4_msb | dig_h4_lsb,
        dig_h5: dig_h5_msb | dig_h5_lsb,
        dig_h6: hum[6] as i8,
        t_fine: 0,
    }
}

/// Fixed calibration image `--simulate` parses. Same integers as `example_calib`.
pub fn pack_example_calib() -> ([u8; 26], [u8; 7]) {
    let want = crate::compensate::example_calib();
    let mut tp = [0u8; 26];
    let mut hum = [0u8; 7];
    put_u16(&mut tp, 0, want.dig_t1);
    put_i16(&mut tp, 2, want.dig_t2);
    put_i16(&mut tp, 4, want.dig_t3);
    put_u16(&mut tp, 6, want.dig_p1);
    put_i16(&mut tp, 8, want.dig_p2);
    put_i16(&mut tp, 10, want.dig_p3);
    put_i16(&mut tp, 12, want.dig_p4);
    put_i16(&mut tp, 14, want.dig_p5);
    put_i16(&mut tp, 16, want.dig_p6);
    put_i16(&mut tp, 18, want.dig_p7);
    put_i16(&mut tp, 20, want.dig_p8);
    put_i16(&mut tp, 22, want.dig_p9);
    tp[25] = want.dig_h1;
    put_i16(&mut hum, 0, want.dig_h2);
    hum[2] = want.dig_h3;
    hum[3] = (want.dig_h4 / 16) as u8;
    hum[5] = (want.dig_h5 / 16) as u8;
    hum[4] = (((want.dig_h5 & 0x0f) as u8) << 4) | ((want.dig_h4 & 0x0f) as u8);
    hum[6] = want.dig_h6 as u8;
    (tp, hum)
}

/// Synthetic registers run through the same parsers as a live burst.
pub fn synthetic_sample() -> (Raw, Calib) {
    let raw = parse_burst(&pack_example_burst());
    let (tp, hum) = pack_example_calib();
    (raw, parse_calib(&tp, &hum))
}

pub fn pack_example_burst() -> [u8; 8] {
    let raw = crate::compensate::example_raw();
    let mut out = [0u8; 8];
    put20(&mut out, 0, raw.pressure);
    put20(&mut out, 3, raw.temperature);
    out[6] = (raw.humidity >> 8) as u8;
    out[7] = raw.humidity as u8;
    out
}

fn put20(out: &mut [u8], at: usize, value: u32) {
    out[at] = (value >> 12) as u8;
    out[at + 1] = (value >> 4) as u8;
    out[at + 2] = ((value & 0x0f) << 4) as u8;
}

fn put_u16(buf: &mut [u8], at: usize, v: u16) {
    buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_i16(buf: &mut [u8], at: usize, v: i16) {
    buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compensate::{compensate, example_raw};

    #[test]
    fn only_chip_id_0x60_is_a_bme280() {
        assert!(chip_is_bme280(0x60));
        assert!(!chip_is_bme280(0x58));
        assert!(!chip_is_bme280(0x61));
    }

    #[test]
    fn burst_roundtrip_keeps_the_example_adcs() {
        let parsed = parse_burst(&pack_example_burst());
        assert_eq!(parsed, example_raw());
    }

    #[test]
    fn calib_bytes_decode_into_the_example_and_compensate() {
        let want = crate::compensate::example_calib();
        let (raw, mut cal) = synthetic_sample();
        assert_eq!(raw, example_raw());
        assert_eq!(cal.dig_t1, want.dig_t1);
        assert_eq!(cal.dig_h4, want.dig_h4);
        assert_eq!(cal.dig_h5, want.dig_h5);
        assert_eq!(cal.dig_h6, want.dig_h6);
        let out = compensate(&mut cal, example_raw());
        assert_eq!(out.temp_centi, 2508);
        assert_eq!(out.pressure_centi_pa, 10_065_328);
        assert_eq!(out.humidity_q22_10, 13_091);
    }
}

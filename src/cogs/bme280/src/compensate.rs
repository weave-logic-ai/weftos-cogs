//! Integer compensation from Bosch Sensortec BME280 driver v3.5.1 (2020-12-17),
//! the non-floating path: temperature in 0.01 °C, 64-bit pressure in Pa × 100,
//! humidity in Q22.10 (%RH × 1024). Formulas also published in BST-BME280-DS002.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calib {
    pub dig_t1: u16,
    pub dig_t2: i16,
    pub dig_t3: i16,
    pub dig_p1: u16,
    pub dig_p2: i16,
    pub dig_p3: i16,
    pub dig_p4: i16,
    pub dig_p5: i16,
    pub dig_p6: i16,
    pub dig_p7: i16,
    pub dig_p8: i16,
    pub dig_p9: i16,
    pub dig_h1: u8,
    pub dig_h2: i16,
    pub dig_h3: u8,
    pub dig_h4: i16,
    pub dig_h5: i16,
    pub dig_h6: i8,
    pub t_fine: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Raw {
    pub temperature: u32,
    pub pressure: u32,
    pub humidity: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Compensated {
    pub t_fine: i32,
    pub temp_centi: i32,
    pub pressure_centi_pa: u32,
    pub humidity_q22_10: u32,
}

/// The fixed register image `--simulate` decodes. Temperature matches the
/// widely published 25.08 °C worked example (adc_T 519888, dig_T*).
pub fn example_calib() -> Calib {
    Calib {
        dig_t1: 27504,
        dig_t2: 26435,
        dig_t3: -1000,
        dig_p1: 36477,
        dig_p2: -10685,
        dig_p3: 3024,
        dig_p4: 2855,
        dig_p5: 140,
        dig_p6: -7,
        dig_p7: 15500,
        dig_p8: -14600,
        dig_p9: 6000,
        dig_h1: 75,
        dig_h2: 364,
        dig_h3: 0,
        dig_h4: 316,
        dig_h5: 50,
        dig_h6: 30,
        t_fine: 0,
    }
}

pub fn example_raw() -> Raw {
    Raw { temperature: 519888, pressure: 415148, humidity: 22636 }
}

pub fn compensate(cal: &mut Calib, raw: Raw) -> Compensated {
    let temp_centi = compensate_temperature(cal, raw.temperature);
    let pressure_centi_pa = compensate_pressure(cal, raw.pressure);
    let humidity_q22_10 = compensate_humidity(cal, raw.humidity);
    Compensated { t_fine: cal.t_fine, temp_centi, pressure_centi_pa, humidity_q22_10 }
}

fn compensate_temperature(cal: &mut Calib, adc: u32) -> i32 {
    let adc = adc as i32;
    let mut var1 = (adc / 8) - (i32::from(cal.dig_t1) * 2);
    var1 = (var1 * i32::from(cal.dig_t2)) / 2048;
    let mut var2 = (adc / 16) - i32::from(cal.dig_t1);
    var2 = (((var2 * var2) / 4096) * i32::from(cal.dig_t3)) / 16384;
    cal.t_fine = var1 + var2;
    let mut temperature = (cal.t_fine * 5 + 128) / 256;
    temperature = temperature.clamp(-4000, 8500);
    temperature
}

fn compensate_pressure(cal: &Calib, adc: u32) -> u32 {
    let mut var1: i64 = i64::from(cal.t_fine) - 128_000;
    let mut var2: i64 = var1 * var1 * i64::from(cal.dig_p6);
    var2 += (var1 * i64::from(cal.dig_p5)) * 131_072;
    var2 += i64::from(cal.dig_p4) * 34_359_738_368;
    var1 = ((var1 * var1 * i64::from(cal.dig_p3)) / 256) + (var1 * i64::from(cal.dig_p2) * 4096);
    let var3: i64 = 140_737_488_355_328;
    var1 = (var3 + var1) * i64::from(cal.dig_p1) / 8_589_934_592;
    if var1 == 0 {
        return 3_000_000;
    }
    let mut var4: i64 = 1_048_576 - i64::from(adc);
    var4 = (((var4 * 2_147_483_648) - var2) * 3125) / var1;
    var1 = (i64::from(cal.dig_p9) * (var4 / 8192) * (var4 / 8192)) / 33_554_432;
    var2 = (i64::from(cal.dig_p8) * var4) / 524_288;
    var4 = ((var4 + var1 + var2) / 256) + (i64::from(cal.dig_p7) * 16);
    let mut pressure = ((var4 / 2) * 100) / 128;
    pressure = pressure.clamp(3_000_000, 11_000_000);
    pressure as u32
}

fn compensate_humidity(cal: &Calib, adc: u32) -> u32 {
    let var1: i32 = cal.t_fine - 76_800;
    let var2_in: i32 = (adc as i32) * 16_384;
    let var3_in: i32 = i32::from(cal.dig_h4) * 1_048_576;
    let var4_in: i32 = i32::from(cal.dig_h5) * var1;
    let mut var5: i32 = (((var2_in - var3_in) - var4_in) + 16_384) / 32_768;
    let mut var2: i32 = (var1 * i32::from(cal.dig_h6)) / 1024;
    let var3: i32 = (var1 * i32::from(cal.dig_h3)) / 2048;
    let var4: i32 = ((var2 * (var3 + 32_768)) / 1024) + 2_097_152;
    var2 = ((var4 * i32::from(cal.dig_h2)) + 8192) / 16_384;
    let var3b: i32 = var5 * var2;
    let var4b: i32 = ((var3b / 32_768) * (var3b / 32_768)) / 128;
    var5 = var3b - ((var4b * i32::from(cal.dig_h1)) / 16);
    var5 = var5.clamp(0, 419_430_400);
    let humidity = (var5 as u32) / 4096;
    humidity.min(102_400)
}

pub fn temp_c(centi: i32) -> String {
    let sign = if centi < 0 { "-" } else { "" };
    let v = centi.abs();
    format!("{sign}{}.{:02}", v / 100, v % 100)
}

pub fn pressure_pa(centi_pa: u32) -> String {
    format!("{}.{:02}", centi_pa / 100, centi_pa % 100)
}

pub fn humidity_pct(q: u32) -> String {
    let milli = (u64::from(q) * 1000) / 1024;
    format!("{}.{:03}", milli / 1000, milli % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datasheet_shaped_example_matches_the_integer_driver() {
        let mut cal = example_calib();
        let out = compensate(&mut cal, example_raw());
        assert_eq!(out.t_fine, 128_423);
        assert_eq!(out.temp_centi, 2508);
        assert_eq!(temp_c(out.temp_centi), "25.08");
        assert_eq!(out.pressure_centi_pa, 10_065_328);
        assert_eq!(pressure_pa(out.pressure_centi_pa), "100653.28");
        assert_eq!(out.humidity_q22_10, 13_091);
        // 13091 / 1024 = 12.784179… %RH. Milli-units are (q * 1000) / 1024 = 12784.
        assert_eq!(humidity_pct(out.humidity_q22_10), "12.784");
    }
}

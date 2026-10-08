//! One forced BME280 sample on Linux `/dev/i2c-N`. Other hosts have no device node.
//! The chip id is read first. A write happens only after 0xD0 returns 0x60.

use crate::compensate::{Calib, Raw};

pub struct Sample {
    pub raw: Raw,
    pub cal: Calib,
}

pub fn read_once(bus: u8, addr: u8) -> Result<Sample, String> {
    linux::read_once(bus, addr)
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::decode::{
        chip_is_bme280, parse_burst, parse_calib, CHIP_ID_BME280, CTRL_HUM_X1, CTRL_MEAS_FORCED_X1,
        REG_CALIB_H, REG_CALIB_TP, REG_CHIP_ID, REG_CTRL_HUM, REG_CTRL_MEAS, REG_DATA,
    };
    use std::ffi::CString;

    struct Fd(std::os::raw::c_int);
    impl Drop for Fd {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }

    pub fn read_once(bus: u8, addr: u8) -> Result<Sample, String> {
        let path = format!("/dev/i2c-{bus}");
        let c_path = CString::new(path.clone()).map_err(|_| format!("bad path {path}"))?;
        let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
        if fd < 0 {
            return Err(format!(
                "cannot open {path} ({err}). Enable I2C and power-cycle the sensor after changing SDO.",
                err = std::io::Error::last_os_error()
            ));
        }
        let fd = Fd(fd);
        // `I2C_SLAVE` from linux/i2c-dev.h. The libc crate does not export it.
        const I2C_SLAVE: libc::c_ulong = 0x0703;
        // SAFETY: plain ioctl on an owned, open fd; the argument is the 7-bit address.
        let rc = unsafe { libc::ioctl(fd.0, I2C_SLAVE as _, libc::c_ulong::from(addr)) };
        if rc < 0 {
            return Err(format!("I2C address 0x{addr:02x} was refused ({})", std::io::Error::last_os_error()));
        }
        let id = read_reg(&fd, REG_CHIP_ID, 1)?;
        if !chip_is_bme280(id[0]) {
            return Err(format!(
                "chip id 0x{got:02x} is not a BME280 (wanted 0x{CHIP_ID_BME280:02x}). A BMP280 or BME680 is a different part.",
                got = id[0]
            ));
        }
        let tp = read_reg(&fd, REG_CALIB_TP, 26)?;
        let hum = read_reg(&fd, REG_CALIB_H, 7)?;
        write_reg(&fd, REG_CTRL_HUM, CTRL_HUM_X1)?;
        write_reg(&fd, REG_CTRL_MEAS, CTRL_MEAS_FORCED_X1)?;
        std::thread::sleep(std::time::Duration::from_millis(20));
        let data = read_reg(&fd, REG_DATA, 8)?;
        let mut burst = [0u8; 8];
        burst.copy_from_slice(&data);
        let mut tp_a = [0u8; 26];
        tp_a.copy_from_slice(&tp);
        let mut hum_a = [0u8; 7];
        hum_a.copy_from_slice(&hum);
        Ok(Sample { raw: parse_burst(&burst), cal: parse_calib(&tp_a, &hum_a) })
    }

    fn write_reg(fd: &Fd, reg: u8, value: u8) -> Result<(), String> {
        let buf = [reg, value];
        let n = unsafe { libc::write(fd.0, buf.as_ptr().cast(), buf.len()) };
        if n != buf.len() as isize {
            return Err(format!("I2C write of 0x{reg:02x} failed ({})", std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn read_reg(fd: &Fd, reg: u8, len: usize) -> Result<Vec<u8>, String> {
        let n = unsafe { libc::write(fd.0, [reg].as_ptr().cast(), 1) };
        if n != 1 {
            return Err(format!("I2C address write of 0x{reg:02x} failed ({})", std::io::Error::last_os_error()));
        }
        let mut buf = vec![0u8; len];
        let n = unsafe { libc::read(fd.0, buf.as_mut_ptr().cast(), len) };
        if n != len as isize {
            return Err(format!("I2C read of 0x{reg:02x} returned {n} bytes ({})", std::io::Error::last_os_error()));
        }
        Ok(buf)
    }
}

#[cfg(not(target_os = "linux"))]
mod linux {
    use super::Sample;

    pub fn read_once(_bus: u8, _addr: u8) -> Result<Sample, String> {
        Err("I2C is only opened on Linux. Use --simulate on this machine.".into())
    }
}

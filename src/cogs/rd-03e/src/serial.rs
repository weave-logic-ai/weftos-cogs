//! Minimal portable serial reader. Opens a tty, sets raw 8N1 at `baud`, and reads bytes.
//!
//! Linux: configured via `termios2` + `BOTHER` (the `TCSETS2` ioctl), which accepts *arbitrary*
//! baud rates — needed because the RD-03E's 256000 bps is not a standard POSIX `Bxxxx` constant.
//! macOS: raw `termios`, then the exact baud is set with the `IOSSIOSPEED` ioctl. Used by the
//! radar/IMU cogs.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::io::AsRawFd;

pub struct Serial {
    dev: File,
}

impl Serial {
    pub fn open(path: &str, baud: u32) -> Result<Self, String> {
        let dev = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("open {path}: {e}"))?;
        configure(dev.as_raw_fd(), baud)?;
        Ok(Self { dev })
    }
    pub fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.dev.read(buf)
    }
}

#[cfg(target_os = "linux")]
fn configure(fd: i32, baud: u32) -> Result<(), String> {
    // SAFETY: termios2 ioctls on an owned, open tty fd; the struct is zeroed then filled by TCGETS2.
    unsafe {
        let mut t: libc::termios2 = std::mem::zeroed();
        if libc::ioctl(fd, libc::TCGETS2, &mut t) != 0 {
            return Err(format!("TCGETS2: {}", std::io::Error::last_os_error()));
        }
        t.c_iflag &= !(libc::IGNBRK
            | libc::BRKINT
            | libc::PARMRK
            | libc::ISTRIP
            | libc::INLCR
            | libc::IGNCR
            | libc::ICRNL
            | libc::IXON);
        t.c_oflag &= !libc::OPOST;
        t.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
        t.c_cflag &= !(libc::CSIZE | libc::PARENB | libc::CBAUD);
        t.c_cflag |= libc::CS8 | libc::CLOCAL | libc::CREAD | libc::BOTHER; // 8N1, custom baud
        t.c_ispeed = baud;
        t.c_ospeed = baud;
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 1; // 0.1 s read timeout
        if libc::ioctl(fd, libc::TCSETS2, &t) != 0 {
            return Err(format!(
                "TCSETS2 (baud {baud}): {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn configure(fd: i32, baud: u32) -> Result<(), String> {
    // SAFETY: termios ioctls on an owned, open tty fd; the struct is zeroed then filled by tcgetattr.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            return Err(format!("tcgetattr: {}", std::io::Error::last_os_error()));
        }
        libc::cfmakeraw(&mut t);
        t.c_cflag |= libc::CLOCAL | libc::CREAD;
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 1;
        libc::cfsetispeed(&mut t, libc::B9600);
        libc::cfsetospeed(&mut t, libc::B9600);
        if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
            return Err(format!("tcsetattr: {}", std::io::Error::last_os_error()));
        }
        // IOSSIOSPEED = _IOW('T', 2, speed_t); speed_t is 4-byte -> request 0x80045402
        let speed: libc::c_uint = baud;
        if libc::ioctl(fd, 0x80045402, &speed) != 0 {
            return Err(format!(
                "set baud {baud} (IOSSIOSPEED): {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

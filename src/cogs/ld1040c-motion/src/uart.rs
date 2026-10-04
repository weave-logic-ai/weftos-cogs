//! Optional, best-effort UART telemetry (OFF by default). The OUT line is the ground truth; this
//! only enriches a window with the module's reported amplitudes when a frame parses cleanly.
//!
//! Frame (all multi-byte fields BIG-ENDIAN), 25 bytes:
//!   0x3C 0x3A | LEN | command | work_pattern | version | radar_threshold[3] | light_threshold |
//!   output_delay[2] | module_id[4] | output_mode | sensitivity_ad | mid_freq_ad |
//!   noise_sum0[2] | signal_sum2[2] | 0x3A 0x3E
//!
//! mid_freq_ad is the motion amplitude; signal = signal_sum2 / 64; noise = noise_sum0 / 64.
//! A frame that does not match the header/length/footer is skipped; nothing is fabricated.
//!
//! The serial setup mirrors rd-03e/hlk-as201's src/serial.rs (termios2 + BOTHER on Linux so any
//! baud works; IOSSIOSPEED on macOS). The datasheet does NOT document the baud rate: 9600 is a
//! guess that must be verified on the bench.

use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::time::Duration;

use crate::export::{Shared, Telemetry};
use crate::source::unix_ms;

const TAG: &str = "[cog-ld1040c-motion]";
/// Byte length of a complete telemetry frame.
const FRAME_LEN: usize = 25;
const HEADER: [u8; 2] = [0x3C, 0x3A];
const FOOTER: [u8; 2] = [0x3A, 0x3E];

/// Drain complete, well-framed telemetry frames from the accumulator, resyncing on bad framing.
pub fn parse_frames(acc: &mut Vec<u8>) -> Vec<Telemetry> {
    let mut out = Vec::new();
    loop {
        // Advance to the next header.
        match acc.windows(2).position(|w| w == HEADER) {
            Some(0) => {}
            Some(i) => {
                acc.drain(..i);
            }
            None => {
                // Keep at most one trailing byte in case it is the start of a header.
                if acc.len() > 1 {
                    acc.drain(..acc.len() - 1);
                }
                break;
            }
        }
        if acc.len() < FRAME_LEN {
            break; // wait for the rest of the frame
        }
        if acc[23..25] == FOOTER {
            out.push(Telemetry {
                t_ms: unix_ms(),
                motion_amplitude: Some(acc[18]),
                noise: Some(f64::from(u16::from_be_bytes([acc[19], acc[20]])) / 64.0),
                signal: Some(f64::from(u16::from_be_bytes([acc[21], acc[22]])) / 64.0),
            });
            acc.drain(..FRAME_LEN);
        } else {
            // Misframed: drop this header and resync on the next one.
            acc.drain(..1);
        }
    }
    out
}

/// Open the UART and keep `shared.telemetry` current. Best-effort: a failure logs to stderr and
/// ends the thread; the OUT line keeps working.
pub fn telemetry_loop(path: String, baud: u32, shared: Shared) {
    let dev = match OpenOptions::new().read(true).write(false).open(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{TAG} UART disabled: open {path}: {e}");
            return;
        }
    };
    if let Err(e) = configure(dev.as_raw_fd(), baud) {
        eprintln!("{TAG} UART disabled: configure {path}: {e}");
        return;
    }
    eprintln!("{TAG} UART telemetry on {path} @ {baud} 8N1 (best-effort)");
    let mut dev = dev;
    let mut acc: Vec<u8> = Vec::with_capacity(128);
    let mut buf = [0u8; 256];
    loop {
        let n = match dev.read(&mut buf) {
            Ok(0) => continue,
            Ok(n) => n,
            Err(e) => {
                eprintln!("{TAG} UART read error: {e}");
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        acc.extend_from_slice(&buf[..n]);
        if acc.len() > 4096 {
            acc.drain(..acc.len() - 4096);
        }
        if let Some(t) = parse_frames(&mut acc).pop() {
            if let Ok(mut st) = shared.lock() {
                st.telemetry = Some(t);
            }
        }
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
        // IOSSIOSPEED = _IOW('T', 2, speed_t); speed_t is 4-byte -> request 0x80045402.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a telemetry frame with the given amplitude, noise-sum and signal-sum.
    fn frame(mid_freq: u8, noise_sum: u16, signal_sum: u16) -> Vec<u8> {
        let mut f = vec![0x3C, 0x3A, 0x13, 0x01, 0x00, 0x01];
        f.extend_from_slice(&[0, 0, 0]); // radar_threshold
        f.push(0); // light_threshold
        f.extend_from_slice(&[0, 0]); // output_delay
        f.extend_from_slice(&[0, 0, 0, 0]); // module_id
        f.push(0); // output_mode
        f.push(0); // sensitivity_ad
        f.push(mid_freq); // mid_freq_ad
        f.extend_from_slice(&noise_sum.to_be_bytes());
        f.extend_from_slice(&signal_sum.to_be_bytes());
        f.extend_from_slice(&FOOTER);
        assert_eq!(f.len(), FRAME_LEN);
        f
    }

    #[test]
    fn parses_a_well_framed_telemetry_frame() {
        let mut acc = frame(200, 128, 640);
        let t = parse_frames(&mut acc);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].motion_amplitude, Some(200));
        assert_eq!(t[0].noise, Some(2.0)); // 128 / 64
        assert_eq!(t[0].signal, Some(10.0)); // 640 / 64
        assert!(acc.is_empty());
    }

    #[test]
    fn resyncs_past_garbage_and_keeps_a_partial() {
        let mut acc = vec![0x11, 0x22];
        acc.extend(frame(10, 64, 64));
        acc.extend_from_slice(&[0x3C]); // start of a possible next header
        let t = parse_frames(&mut acc);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].motion_amplitude, Some(10));
        assert_eq!(acc, vec![0x3C]); // partial header retained
    }

    #[test]
    fn bad_footer_is_dropped() {
        let mut bad = frame(5, 0, 0);
        bad[24] = 0x00; // corrupt the footer
        let mut acc = bad;
        assert!(parse_frames(&mut acc).is_empty());
    }
}

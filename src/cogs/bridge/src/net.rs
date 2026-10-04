//! Small network hardening primitives shared by the server and the relay: a read wrapper with a
//! whole-request deadline, a capped line reader, and token-bucket rate limiters.

use std::collections::HashMap;
use std::hash::Hash;
use std::io::{self, BufRead, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// A `Read` over a socket that fails with `TimedOut` once a fixed deadline passes, however the
/// peer paces its bytes. A per-read socket timeout alone lets a slow sender drip bytes forever.
pub struct DeadlineStream {
    conn: TcpStream,
    deadline: Instant,
}

impl DeadlineStream {
    pub fn new(conn: TcpStream, deadline: Instant) -> Self {
        Self { conn, deadline }
    }

    /// Moves the deadline (used to widen from the header budget to the whole-request budget).
    pub fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }
}

/// Writes all of `buf` within `total`, however slowly the peer drains its socket. A per-write
/// timeout alone lets a reader that accepts one byte every few seconds hold a slot indefinitely.
pub fn write_all_deadline(conn: &mut TcpStream, mut buf: &[u8], total: Duration) -> io::Result<()> {
    let end = Instant::now() + total;
    while !buf.is_empty() {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "write deadline"));
        }
        conn.set_write_timeout(Some(left))?;
        match conn.write(buf) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Lets a log line through at most once a second per key, counting what it swallowed, so a
/// flood of refusals cannot flood the Seed's log (and the cog's stderr pipe).
#[derive(Default)]
pub struct LogGate {
    map: HashMap<&'static str, (u64, u64)>,
}

impl LogGate {
    /// `Some(n)`: print now, `n` lines were suppressed since the last print. `None`: suppress.
    pub fn check(&mut self, key: &'static str, now_ms: u64) -> Option<u64> {
        let e = self.map.entry(key).or_insert((0, 0));
        if e.0 == 0 || now_ms.saturating_sub(e.0) >= 1000 {
            let suppressed = e.1;
            *e = (now_ms.max(1), 0);
            Some(suppressed)
        } else {
            e.1 += 1;
            None
        }
    }
}

impl Read for DeadlineStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "request deadline"));
        }
        self.conn.set_read_timeout(Some(left))?;
        self.conn.read(buf)
    }
}

/// Reads one line of at most `cap` bytes (including the newline). `Ok(None)` is EOF. A longer
/// line is an `InvalidData` error rather than unbounded buffering.
pub fn read_capped_line<R: BufRead + ?Sized>(r: &mut R, cap: usize) -> io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = (&mut *r).take(cap as u64 + 1).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > cap {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// `TcpStream::connect` with a bound on the connect itself.
pub fn connect(target: &str, timeout: Duration) -> Result<TcpStream, String> {
    let addr = target
        .to_socket_addrs()
        .map_err(|e| format!("resolve {target}: {e}"))?
        .next()
        .ok_or_else(|| format!("resolve {target}: no address"))?;
    TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("connect {target}: {e}"))
}

/// A token bucket: `rate` tokens/second up to `burst`.
#[derive(Clone, Copy, Debug)]
pub struct Bucket {
    tokens: f64,
    last_ms: u64,
}

impl Bucket {
    pub fn full(burst: f64, now_ms: u64) -> Self {
        Self {
            tokens: burst,
            last_ms: now_ms,
        }
    }

    pub fn take(&mut self, rate: f64, burst: f64, now_ms: u64) -> bool {
        let dt = now_ms.saturating_sub(self.last_ms) as f64 / 1000.0;
        self.tokens = (self.tokens + dt * rate).min(burst);
        self.last_ms = now_ms;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Per-key rate limiter with a bounded key table (the least recently used key is evicted).
pub struct Throttle<K> {
    map: HashMap<K, Bucket>,
    cap: usize,
    rate: f64,
    burst: f64,
}

impl<K: Hash + Eq + Clone> Throttle<K> {
    pub fn new(cap: usize, rate: f64, burst: f64) -> Self {
        Self {
            map: HashMap::new(),
            cap,
            rate,
            burst,
        }
    }

    pub fn allow(&mut self, key: &K, now_ms: u64) -> bool {
        if !self.map.contains_key(key) && self.map.len() >= self.cap {
            if let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, b)| b.last_ms)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&oldest);
            }
        }
        let (rate, burst) = (self.rate, self.burst);
        self.map
            .entry(key.clone())
            .or_insert_with(|| Bucket::full(burst, now_ms))
            .take(rate, burst, now_ms)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn capped_line_rejects_long_lines_and_reads_short_ones() {
        let mut r = BufReader::new(&b"short\r\n"[..]);
        assert_eq!(
            read_capped_line(&mut r, 16).unwrap().as_deref(),
            Some("short\r\n")
        );
        assert_eq!(read_capped_line(&mut r, 16).unwrap(), None);
        let long = [b'a'; 100];
        let mut r = BufReader::new(&long[..]);
        assert!(read_capped_line(&mut r, 16).is_err());
    }

    #[test]
    fn log_gate_prints_once_a_second_and_counts_the_rest() {
        let mut g = LogGate::default();
        assert_eq!(g.check("a", 5_000), Some(0));
        assert_eq!(g.check("a", 5_100), None);
        assert_eq!(g.check("a", 5_900), None);
        assert_eq!(g.check("b", 5_900), Some(0), "keys are independent");
        assert_eq!(g.check("a", 6_100), Some(2));
        assert_eq!(g.check("a", 6_200), None);
    }

    #[test]
    fn a_stalled_reader_cannot_hold_a_write_past_the_deadline() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let _idle = TcpStream::connect(addr).unwrap(); // connects, never reads
        let (mut srv, _) = l.accept().unwrap();
        let big = vec![0u8; 64 * 1024 * 1024];
        let t = Instant::now();
        let r = write_all_deadline(&mut srv, &big, Duration::from_millis(300));
        assert!(r.is_err(), "64 MiB cannot fit in socket buffers");
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn bucket_allows_burst_then_refills() {
        let mut b = Bucket::full(3.0, 0);
        assert!((0..3).all(|_| b.take(1.0, 3.0, 0)));
        assert!(!b.take(1.0, 3.0, 0));
        assert!(b.take(1.0, 3.0, 1000));
        assert!(!b.take(1.0, 3.0, 1100));
    }

    #[test]
    fn throttle_table_is_bounded() {
        let mut t = Throttle::new(4, 1.0, 1.0);
        for i in 0..50u32 {
            t.allow(&i, i as u64);
        }
        assert_eq!(t.len(), 4);
    }
}
